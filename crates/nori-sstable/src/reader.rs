//! SSTable reader for efficient key lookups and iteration.
//!
//! The reader loads metadata (footer, index, bloom filter) into memory on open,
//! enabling fast lookups with minimal disk I/O. The bloom filter provides a fast
//! negative lookup path for non-existent keys.
//!
//! # Example
//!
//! ```no_run
//! use nori_sstable::SSTableReader;
//! use std::path::PathBuf;
//! use std::sync::Arc;
//!
//! # async fn example() -> nori_sstable::Result<()> {
//! let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);
//!
//! // Point lookup
//! if let Some(entry) = reader.get(b"key1").await? {
//!     println!("Found: {:?}", entry.value);
//! }
//!
//! // Full table scan
//! let mut iter = reader.clone().iter();
//! while let Some(entry) = iter.try_next().await? {
//!     println!("{:?} = {:?}", entry.key, entry.value);
//! }
//! # Ok(())
//! # }
//! ```

use crate::block::Block;
use crate::bloom::BloomFilter;
use crate::compress;
use crate::entry::Entry;
use crate::error::{Result, SSTableError};
use crate::format::Footer;
use crate::index::Index;
use crate::iterator::SSTableIterator;
use bytes::Bytes;
use lru::LruCache;
use nori_observe::{Meter, NoopMeter};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::Mutex;

/// SSTable reader with cached metadata for efficient lookups.
pub struct SSTableReader {
    file: Mutex<File>,
    path: PathBuf,
    footer: Footer,
    index: Index,
    /// Per-file Bloom filter (v1 format only, None for v2 with per-block QF).
    bloom: Option<BloomFilter>,
    file_size: u64,
    meter: Arc<dyn Meter>,
    /// LRU cache for blocks (key: block_offset)
    block_cache: Option<Mutex<LruCache<u64, Block>>>,
}

impl SSTableReader {
    /// Opens an SSTable file for reading with default (noop) metrics and 64MB block cache.
    ///
    /// This reads and caches the footer, index, and bloom filter in memory.
    pub async fn open(path: PathBuf) -> Result<Self> {
        Self::open_with_config(path, Arc::new(NoopMeter), 64).await
    }

    /// Opens an SSTable file for reading with custom metrics and default 64MB block cache.
    ///
    /// This reads and caches the footer, index, and bloom filter in memory.
    /// Metrics are emitted via the provided `Meter` implementation.
    pub async fn open_with_meter(path: PathBuf, meter: Arc<dyn Meter>) -> Result<Self> {
        Self::open_with_config(path, meter, 64).await
    }

    /// Opens an SSTable file for reading with custom metrics and cache size.
    ///
    /// # Parameters
    /// - `path`: Path to the SSTable file
    /// - `meter`: Metrics implementation
    /// - `block_cache_mb`: Block cache size in MB (0 to disable caching)
    pub async fn open_with_config(
        path: PathBuf,
        meter: Arc<dyn Meter>,
        block_cache_mb: usize,
    ) -> Result<Self> {
        let start = std::time::Instant::now();

        let mut file = File::open(&path).await.map_err(SSTableError::Io)?;

        // Get file size
        let file_size = file.metadata().await.map_err(SSTableError::Io)?.len();

        if file_size < 64 {
            return Err(SSTableError::InvalidFormat(
                "File too small to contain footer".to_string(),
            ));
        }

        // Read footer from last 64 bytes
        file.seek(std::io::SeekFrom::End(-64))
            .await
            .map_err(SSTableError::Io)?;

        let mut footer_bytes = [0u8; 64];
        file.read_exact(&mut footer_bytes)
            .await
            .map_err(SSTableError::Io)?;

        let footer = Footer::decode(&footer_bytes)?;

        // Read index
        file.seek(std::io::SeekFrom::Start(footer.index_offset))
            .await
            .map_err(SSTableError::Io)?;

        let mut index_bytes = vec![0u8; footer.index_size as usize];
        file.read_exact(&mut index_bytes)
            .await
            .map_err(SSTableError::Io)?;

        let index = Index::decode(&index_bytes)?;

        // Read bloom filter only for v1 format (v2 uses per-block QF)
        let bloom = if footer.uses_bloom_filter() && footer.bloom_size > 0 {
            file.seek(std::io::SeekFrom::Start(footer.bloom_offset))
                .await
                .map_err(SSTableError::Io)?;

            let mut bloom_bytes = vec![0u8; footer.bloom_size as usize];
            file.read_exact(&mut bloom_bytes)
                .await
                .map_err(SSTableError::Io)?;

            Some(BloomFilter::decode(&bloom_bytes)?)
        } else {
            None
        };

        // Initialize block cache if enabled
        let block_cache = if block_cache_mb > 0 {
            // Calculate number of blocks that fit in cache
            // Assume average block size of 4KB
            let blocks_in_cache = (block_cache_mb * 1024 * 1024) / 4096;
            Some(Mutex::new(LruCache::new(
                NonZeroUsize::new(blocks_in_cache.max(1)).unwrap(),
            )))
        } else {
            None
        };

        // Track open duration
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        meter
            .histo(
                "sstable_open_duration_ms",
                &[0.1, 0.5, 1.0, 5.0, 10.0, 50.0, 100.0],
                &[],
            )
            .observe(duration_ms);

        Ok(Self {
            file: Mutex::new(file),
            path,
            footer,
            index,
            bloom,
            file_size,
            meter,
            block_cache,
        })
    }

    /// Looks up a key in the SSTable.
    ///
    /// Returns `None` if the key is not present (using bloom filter for fast negative lookups).
    /// Returns `Some(entry)` if found, which may be a tombstone.
    ///
    /// For v1 format: Uses per-file bloom filter for fast rejection.
    /// For v2 format: Uses per-block quotient filter for fast rejection.
    pub async fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        let start = std::time::Instant::now();

        // Check bloom filter first for v1 format (fast negative lookup)
        if let Some(ref bloom) = self.bloom {
            if !bloom.contains(key) {
                // Track bloom filter skip
                self.meter
                    .counter("sstable_bloom_checks", &[("outcome", "skip")])
                    .inc(1);

                let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                self.meter
                    .histo(
                        "sstable_get_duration_ms",
                        &[0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 50.0],
                        &[("outcome", "bloom_skip")],
                    )
                    .observe(duration_ms);

                return Ok(None);
            }

            // Track bloom filter pass
            self.meter
                .counter("sstable_bloom_checks", &[("outcome", "pass")])
                .inc(1);
        }
        // For v2 format, fast rejection happens in Block::get() via per-block QF

        // Find which block might contain the key
        let block_idx = match self.index.find_block(key) {
            Some(idx) => idx,
            None => {
                let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                self.meter
                    .histo(
                        "sstable_get_duration_ms",
                        &[0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 50.0],
                        &[("outcome", "miss")],
                    )
                    .observe(duration_ms);
                return Ok(None); // Key is before first block
            }
        };

        // Read the block
        let entry = self.index.get(block_idx).unwrap();
        let block = self
            .read_block(entry.block_offset, entry.block_size)
            .await?;

        // Search within the block
        let result = block.get(key)?;

        // Track outcome
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;
        match &result {
            Some(_) => {
                self.meter
                    .histo(
                        "sstable_get_duration_ms",
                        &[0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 50.0],
                        &[("outcome", "hit")],
                    )
                    .observe(duration_ms);
            }
            None => {
                self.meter
                    .histo(
                        "sstable_get_duration_ms",
                        &[0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 50.0],
                        &[("outcome", "miss")],
                    )
                    .observe(duration_ms);
            }
        }

        Ok(result)
    }

    /// Reads a specific block from cache or disk.
    pub(crate) async fn read_block(&self, offset: u64, size: u32) -> Result<Block> {
        // Check cache first if enabled
        if let Some(ref cache) = self.block_cache {
            let mut cache_lock = cache.lock().await;
            if let Some(block) = cache_lock.get(&offset) {
                // Cache hit - return cloned block (already decompressed)
                self.meter.counter("sstable_block_cache_hits", &[]).inc(1);
                return Ok(block.clone());
            }
            // Cache miss - continue to read from disk
            self.meter.counter("sstable_block_cache_misses", &[]).inc(1);
        }

        // Read compressed block from disk
        let mut file = self.file.lock().await;

        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(SSTableError::Io)?;

        let mut compressed_bytes = vec![0u8; size as usize];
        file.read_exact(&mut compressed_bytes)
            .await
            .map_err(SSTableError::Io)?;

        // Track block read
        self.meter.counter("sstable_block_reads", &[]).inc(1);

        // Decompress the block data
        let decompressed_bytes = compress::decompress(&compressed_bytes, self.footer.compression)?;

        // Decode the decompressed block
        // v2 format has inline QF, v1 format does not
        let has_inline_filter = self.footer.uses_quotient_filter();
        let block = Block::decode_with_filter(Bytes::from(decompressed_bytes), has_inline_filter)?;

        // Cache the decompressed block if caching is enabled
        if let Some(ref cache) = self.block_cache {
            let mut cache_lock = cache.lock().await;
            cache_lock.put(offset, block.clone());
        }

        Ok(block)
    }

    /// Returns an iterator over all entries in the SSTable.
    pub fn iter(self: Arc<Self>) -> SSTableIterator {
        SSTableIterator::new(self, None, None)
    }

    /// Returns an iterator over entries in the given key range.
    ///
    /// Iterates over all keys where `start <= key < end`.
    pub fn iter_range(self: Arc<Self>, start: Bytes, end: Bytes) -> SSTableIterator {
        SSTableIterator::new(self, Some(start), Some(end))
    }

    /// Returns metadata about this SSTable.
    pub fn entry_count(&self) -> u64 {
        self.footer.entry_count
    }

    /// Returns the file path.
    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Returns the file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    /// Returns the number of blocks in this SSTable.
    pub fn block_count(&self) -> usize {
        self.index.len()
    }

    /// Returns a reference to the index (for internal use).
    pub(crate) fn index(&self) -> &Index {
        &self.index
    }

    /// Returns a reference to the bloom filter (for testing/debugging).
    /// Returns None for v2 format SSTables that use per-block QF.
    #[cfg(test)]
    pub(crate) fn bloom(&self) -> Option<&BloomFilter> {
        self.bloom.as_ref()
    }

    /// Returns true if this SSTable uses v2 format with per-block Quotient Filters.
    pub fn uses_quotient_filter(&self) -> bool {
        self.footer.uses_quotient_filter()
    }

    /// Returns true if this SSTable uses v1 format with per-file Bloom filter.
    pub fn uses_bloom_filter(&self) -> bool {
        self.footer.uses_bloom_filter()
    }

    /// Returns a reference to the footer (for testing/debugging).
    pub fn footer(&self) -> &Footer {
        &self.footer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entry, SSTableBuilder, SSTableConfig};
    use tempfile::TempDir;

    async fn build_test_sstable(
        path: PathBuf,
        entries: Vec<(String, String)>,
    ) -> crate::Result<()> {
        let config = SSTableConfig {
            path,
            estimated_entries: entries.len(),
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await?;
        for (k, v) in entries {
            builder.add(&Entry::put(k, v)).await?;
        }
        builder.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_reader_open_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        // Build a test SSTable
        build_test_sstable(
            path.clone(),
            vec![
                ("key1".to_string(), "value1".to_string()),
                ("key2".to_string(), "value2".to_string()),
                ("key3".to_string(), "value3".to_string()),
            ],
        )
        .await
        .unwrap();

        // Open and verify
        let reader = SSTableReader::open(path.clone()).await.unwrap();
        assert_eq!(reader.entry_count(), 3);
        assert_eq!(reader.path(), &path);
        assert!(reader.file_size() > 0);
        assert_eq!(reader.block_count(), 1);
    }

    #[tokio::test]
    async fn test_reader_get_existing_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        build_test_sstable(
            path.clone(),
            vec![
                ("apple".to_string(), "red".to_string()),
                ("banana".to_string(), "yellow".to_string()),
                ("cherry".to_string(), "red".to_string()),
                ("date".to_string(), "brown".to_string()),
            ],
        )
        .await
        .unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // Test all keys exist
        let entry = reader.get(b"apple").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"red");

        let entry = reader.get(b"banana").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"yellow");

        let entry = reader.get(b"cherry").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"red");

        let entry = reader.get(b"date").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"brown");
    }

    #[tokio::test]
    async fn test_reader_get_missing_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        build_test_sstable(
            path.clone(),
            vec![
                ("key2".to_string(), "value2".to_string()),
                ("key4".to_string(), "value4".to_string()),
                ("key6".to_string(), "value6".to_string()),
            ],
        )
        .await
        .unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // Keys that don't exist
        assert!(reader.get(b"key1").await.unwrap().is_none());
        assert!(reader.get(b"key3").await.unwrap().is_none());
        assert!(reader.get(b"key5").await.unwrap().is_none());
        assert!(reader.get(b"key7").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_reader_bloom_filter_saves_io() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        // Build with many entries
        let mut entries = Vec::new();
        for i in 0..100 {
            entries.push((format!("key{:04}", i), format!("value{:04}", i)));
        }

        build_test_sstable(path.clone(), entries).await.unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // Bloom filter should reject keys not in table
        // (with high probability - small chance of false positive)
        let bloom = reader.bloom().expect("v1 format should have bloom filter");
        assert!(bloom.contains(b"key0000"));
        assert!(bloom.contains(b"key0050"));

        // This key wasn't added, bloom should (likely) reject it
        let not_present = reader.get(b"key9999").await.unwrap();
        assert!(not_present.is_none());
    }

    #[tokio::test]
    async fn test_reader_empty_sstable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.sst");

        build_test_sstable(path.clone(), vec![]).await.unwrap();

        let reader = SSTableReader::open(path).await.unwrap();
        assert_eq!(reader.entry_count(), 0);
        assert_eq!(reader.block_count(), 0);

        // Any key lookup should return None
        assert!(reader.get(b"any_key").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_reader_multiple_blocks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("multi.sst");

        // Build with small block size to force multiple blocks
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            block_size: 512, // Small blocks
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..100 {
            let key = format!("key{:04}", i);
            let value = "x".repeat(50); // Large enough to fill blocks
            builder.add(&Entry::put(key, value)).await.unwrap();
        }
        builder.finish().await.unwrap();

        let reader = SSTableReader::open(path).await.unwrap();
        assert!(reader.block_count() > 1, "Should have multiple blocks");

        // Verify we can read keys from different blocks
        let entry = reader.get(b"key0000").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);

        let entry = reader.get(b"key0050").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);

        let entry = reader.get(b"key0099").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);
    }

    #[tokio::test]
    async fn test_reader_tombstones() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tombstones.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 5,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        builder
            .add(&Entry::put(&b"key1"[..], &b"value1"[..]))
            .await
            .unwrap();
        builder.add(&Entry::delete(&b"key2"[..])).await.unwrap();
        builder
            .add(&Entry::put(&b"key3"[..], &b"value3"[..]))
            .await
            .unwrap();
        builder.finish().await.unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // key1 should exist
        let entry = reader.get(b"key1").await.unwrap().unwrap();
        assert!(!entry.tombstone);
        assert_eq!(entry.value.as_ref(), b"value1");

        // key2 should exist as tombstone
        let entry = reader.get(b"key2").await.unwrap().unwrap();
        assert!(entry.tombstone);

        // key3 should exist
        let entry = reader.get(b"key3").await.unwrap().unwrap();
        assert!(!entry.tombstone);
        assert_eq!(entry.value.as_ref(), b"value3");
    }

    #[tokio::test]
    async fn test_reader_invalid_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("invalid.sst");

        // Write invalid data
        tokio::fs::write(&path, b"invalid data").await.unwrap();

        // Should fail to open
        let result = SSTableReader::open(path).await;
        assert!(result.is_err());
    }

    // === V2 FORMAT TESTS (with Quotient Filter) ===

    use crate::builder::FilterType;

    async fn build_test_sstable_v2(
        path: PathBuf,
        entries: Vec<(String, String)>,
    ) -> crate::Result<()> {
        let config = SSTableConfig {
            path,
            estimated_entries: entries.len().max(10),
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await?;
        for (k, v) in entries {
            builder.add(&Entry::put(k, v)).await?;
        }
        builder.finish().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_reader_v2_open_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2.sst");

        build_test_sstable_v2(
            path.clone(),
            vec![
                ("key1".to_string(), "value1".to_string()),
                ("key2".to_string(), "value2".to_string()),
                ("key3".to_string(), "value3".to_string()),
            ],
        )
        .await
        .unwrap();

        let reader = SSTableReader::open(path.clone()).await.unwrap();

        // Verify v2 format detection
        assert!(reader.uses_quotient_filter());
        assert!(!reader.uses_bloom_filter());
        assert!(reader.bloom().is_none()); // No per-file bloom for v2

        assert_eq!(reader.entry_count(), 3);
        assert_eq!(reader.path(), &path);
    }

    #[tokio::test]
    async fn test_reader_v2_get_existing_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2.sst");

        build_test_sstable_v2(
            path.clone(),
            vec![
                ("apple".to_string(), "red".to_string()),
                ("banana".to_string(), "yellow".to_string()),
                ("cherry".to_string(), "red".to_string()),
                ("date".to_string(), "brown".to_string()),
            ],
        )
        .await
        .unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // Test all keys exist
        let entry = reader.get(b"apple").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"red");

        let entry = reader.get(b"banana").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"yellow");

        let entry = reader.get(b"cherry").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"red");

        let entry = reader.get(b"date").await.unwrap().unwrap();
        assert_eq!(entry.value.as_ref(), b"brown");
    }

    #[tokio::test]
    async fn test_reader_v2_get_missing_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2.sst");

        build_test_sstable_v2(
            path.clone(),
            vec![
                ("key2".to_string(), "value2".to_string()),
                ("key4".to_string(), "value4".to_string()),
                ("key6".to_string(), "value6".to_string()),
            ],
        )
        .await
        .unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        // Keys that don't exist - per-block QF should help reject
        assert!(reader.get(b"key1").await.unwrap().is_none());
        assert!(reader.get(b"key3").await.unwrap().is_none());
        assert!(reader.get(b"key5").await.unwrap().is_none());
        assert!(reader.get(b"key7").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_reader_v2_multiple_blocks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2_multi.sst");

        // Build with small block size to force multiple blocks
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            block_size: 512, // Small blocks
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..100 {
            let key = format!("key{:04}", i);
            let value = "x".repeat(50); // Large enough to fill blocks
            builder.add(&Entry::put(key, value)).await.unwrap();
        }
        builder.finish().await.unwrap();

        let reader = SSTableReader::open(path).await.unwrap();

        assert!(reader.uses_quotient_filter());
        assert!(reader.block_count() > 1, "Should have multiple blocks");

        // Verify we can read keys from different blocks
        let entry = reader.get(b"key0000").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);

        let entry = reader.get(b"key0050").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);

        let entry = reader.get(b"key0099").await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 50);

        // Non-existent keys should return None
        assert!(reader.get(b"key9999").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_reader_v2_iterator() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2_iter.sst");

        let entries = vec![
            ("aaa".to_string(), "val_a".to_string()),
            ("bbb".to_string(), "val_b".to_string()),
            ("ccc".to_string(), "val_c".to_string()),
        ];

        build_test_sstable_v2(path.clone(), entries.clone()).await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());

        // Iterate and verify
        let mut iter = reader.iter();
        for (expected_key, expected_val) in &entries {
            let entry = iter.try_next().await.unwrap().unwrap();
            assert_eq!(entry.key.as_ref(), expected_key.as_bytes());
            assert_eq!(entry.value.as_ref(), expected_val.as_bytes());
        }
        assert!(iter.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_reader_v1_vs_v2_compatibility() {
        let dir = TempDir::new().unwrap();

        let entries = vec![
            ("key1".to_string(), "val1".to_string()),
            ("key2".to_string(), "val2".to_string()),
            ("key3".to_string(), "val3".to_string()),
        ];

        // Build v1 format
        let v1_path = dir.path().join("v1.sst");
        build_test_sstable(v1_path.clone(), entries.clone()).await.unwrap();

        // Build v2 format
        let v2_path = dir.path().join("v2.sst");
        build_test_sstable_v2(v2_path.clone(), entries.clone()).await.unwrap();

        // Open both
        let v1_reader = SSTableReader::open(v1_path).await.unwrap();
        let v2_reader = SSTableReader::open(v2_path).await.unwrap();

        // Verify format detection
        assert!(v1_reader.uses_bloom_filter());
        assert!(!v1_reader.uses_quotient_filter());
        assert!(v1_reader.bloom().is_some());

        assert!(!v2_reader.uses_bloom_filter());
        assert!(v2_reader.uses_quotient_filter());
        assert!(v2_reader.bloom().is_none());

        // Both should return same data
        for (key, val) in &entries {
            let v1_entry = v1_reader.get(key.as_bytes()).await.unwrap().unwrap();
            let v2_entry = v2_reader.get(key.as_bytes()).await.unwrap().unwrap();

            assert_eq!(v1_entry.key, v2_entry.key);
            assert_eq!(v1_entry.value, v2_entry.value);
        }

        // Both should reject non-existent keys
        assert!(v1_reader.get(b"nonexistent").await.unwrap().is_none());
        assert!(v2_reader.get(b"nonexistent").await.unwrap().is_none());
    }
}
