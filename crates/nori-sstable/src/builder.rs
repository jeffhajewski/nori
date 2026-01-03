//! High-level SSTable builder API.
//!
//! The builder provides a convenient interface for creating SSTables from a stream
//! of sorted key-value entries. It handles block management, index building, bloom
//! filter population, and file writing.
//!
//! # Example
//!
//! ```
//! use nori_sstable::{SSTableBuilder, SSTableConfig, Entry};
//! use std::path::PathBuf;
//!
//! # async fn example() -> nori_sstable::Result<()> {
//! let config = SSTableConfig {
//!     path: PathBuf::from("/tmp/test.sst"),
//!     estimated_entries: 1000,
//!     ..Default::default()
//! };
//!
//! let mut builder = SSTableBuilder::new(config).await?;
//!
//! builder.add(&Entry::put("key1", "value1")).await?;
//! builder.add(&Entry::put("key2", "value2")).await?;
//!
//! let metadata = builder.finish().await?;
//! println!("Created SSTable with {} entries", metadata.entry_count);
//! # Ok(())
//! # }
//! ```

use crate::block::BlockBuilder;
use crate::bloom::BloomFilter;
use crate::compress;
use crate::entry::Entry;
use crate::error::{Result, SSTableError};
use crate::format::{
    Compression, Footer, BLOOM_BITS_PER_KEY, DEFAULT_BLOCK_SIZE, DEFAULT_QF_REMAINDER_BITS,
    DEFAULT_RESTART_INTERVAL,
};
use crate::index::Index;
use crate::quotient_filter::{Fingerprint, QuotientFilterConfig};
use crate::writer::SSTableWriter;
use bytes::Bytes;
use nori_observe::{Meter, NoopMeter};
use std::path::PathBuf;

/// Filter type for SSTable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FilterType {
    /// Per-file Bloom filter (v1 format, legacy).
    #[default]
    Bloom,
    /// Per-block Quotient Filters (v2 format).
    /// Enables filter merging during compaction without re-hashing keys.
    QuotientFilter,
}

/// Configuration for building an SSTable.
#[derive(Debug, Clone)]
pub struct SSTableConfig {
    /// Path where the SSTable file will be written.
    pub path: PathBuf,
    /// Estimated number of entries (used for bloom filter sizing).
    pub estimated_entries: usize,
    /// Target block size in bytes (default: 4096).
    pub block_size: u32,
    /// Restart interval for prefix compression (default: 16).
    pub restart_interval: usize,
    /// Compression algorithm (default: None).
    pub compression: Compression,
    /// Bits per key for bloom filter (default: 10). Only used with FilterType::Bloom.
    pub bloom_bits_per_key: usize,
    /// Block cache size in MB for readers (default: 64). Set to 0 to disable caching.
    pub block_cache_mb: usize,
    /// Filter type: Bloom (v1) or QuotientFilter (v2).
    pub filter_type: FilterType,
    /// QF remainder bits for false positive rate (default: 7 for ~0.78% FPR).
    /// Only used with FilterType::QuotientFilter.
    pub qf_remainder_bits: u8,
}

impl Default for SSTableConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("sstable.sst"),
            estimated_entries: 1000,
            block_size: DEFAULT_BLOCK_SIZE,
            restart_interval: DEFAULT_RESTART_INTERVAL,
            compression: Compression::None,
            bloom_bits_per_key: BLOOM_BITS_PER_KEY,
            block_cache_mb: 64, // 64MB default cache
            filter_type: FilterType::Bloom,
            qf_remainder_bits: DEFAULT_QF_REMAINDER_BITS,
        }
    }
}

/// Metadata about the completed SSTable.
#[derive(Debug, Clone)]
pub struct SSTableMetadata {
    /// Path to the SSTable file.
    pub path: PathBuf,
    /// Total number of entries written.
    pub entry_count: u64,
    /// Total file size in bytes.
    pub file_size: u64,
    /// Number of data blocks.
    pub block_count: usize,
    /// Compression used.
    pub compression: Compression,
}

/// High-level builder for creating SSTables.
pub struct SSTableBuilder {
    config: SSTableConfig,
    writer: SSTableWriter,
    block_builder: BlockBuilder,
    index: Index,
    /// Bloom filter for v1 format (None for v2 format with per-block QF).
    bloom: Option<BloomFilter>,
    entry_count: u64,
    last_key: Bytes,
    first_key: Option<Bytes>,
    /// First key of the current block (reset after each flush)
    block_first_key: Option<Bytes>,
    meter: Box<dyn Meter>,
    start_time: std::time::Instant,
}

impl SSTableBuilder {
    /// Creates a new SSTable builder with default (noop) metrics.
    ///
    /// This opens the file for writing and initializes all internal structures.
    pub async fn new(config: SSTableConfig) -> Result<Self> {
        Self::new_with_meter(config, Box::new(NoopMeter)).await
    }

    /// Creates a new SSTable builder with custom metrics.
    ///
    /// This opens the file for writing and initializes all internal structures.
    /// Metrics are emitted via the provided `Meter` implementation.
    pub async fn new_with_meter(config: SSTableConfig, meter: Box<dyn Meter>) -> Result<Self> {
        let writer = SSTableWriter::create(config.path.clone()).await?;

        // Create block builder based on filter type
        let (block_builder, bloom) = match config.filter_type {
            FilterType::Bloom => {
                // v1 format: per-file Bloom filter, blocks without inline QF
                let bb = BlockBuilder::new(config.restart_interval);
                let bloom = BloomFilter::new(config.estimated_entries, config.bloom_bits_per_key);
                (bb, Some(bloom))
            }
            FilterType::QuotientFilter => {
                // v2 format: per-block Quotient Filters, no per-file Bloom
                // Estimate entries per block based on block size
                let avg_entry_size = 64; // Conservative estimate
                let entries_per_block = (config.block_size as usize / avg_entry_size).max(16);
                let qf_config = QuotientFilterConfig::for_keys(entries_per_block);
                let bb = BlockBuilder::new_with_filter(config.restart_interval, qf_config);
                (bb, None)
            }
        };

        let index = Index::new();

        Ok(Self {
            config,
            writer,
            block_builder,
            index,
            bloom,
            entry_count: 0,
            last_key: Bytes::new(),
            first_key: None,
            block_first_key: None,
            meter,
            start_time: std::time::Instant::now(),
        })
    }

    /// Adds an entry to the SSTable.
    ///
    /// Entries must be added in strictly sorted order by key.
    /// This method will flush the current block if it exceeds the target block size.
    pub async fn add(&mut self, entry: &Entry) -> Result<()> {
        // Validate sorted order
        if !self.last_key.is_empty() && entry.key <= self.last_key {
            return Err(SSTableError::KeysNotSorted(
                self.last_key.to_vec(),
                entry.key.to_vec(),
            ));
        }

        // Track first key in SSTable
        if self.first_key.is_none() {
            self.first_key = Some(entry.key.clone());
        }

        // Add to bloom filter for v1 format (v2 uses per-block QF in block_builder)
        if let Some(ref mut bloom) = self.bloom {
            bloom.add(&entry.key);
        }

        // Try to add to current block (populates inline QF for v2 format)
        self.block_builder.add(entry)?;

        // Track first key of current block
        if self.block_first_key.is_none() {
            self.block_first_key = Some(entry.key.clone());
        }

        // Check if we need to flush the block
        if self.block_builder.current_size() >= self.config.block_size as usize {
            self.flush_block().await?;
        }

        self.last_key = entry.key.clone();
        self.entry_count += 1;

        // Track entry written
        self.meter.counter("sstable_entries_written", &[]).inc(1);

        Ok(())
    }

    /// Adds an entry to the SSTable with a pre-computed fingerprint.
    ///
    /// This method is used during compaction to avoid re-hashing keys.
    /// Instead of hashing the key to compute the fingerprint, the caller
    /// provides a pre-computed fingerprint that is passed to the BlockBuilder.
    ///
    /// For v1 format (Bloom filter), this behaves like `add()` because:
    /// - Bloom filters use multiple hash functions, so a single fingerprint is insufficient
    /// - The fingerprint is ignored and the key is hashed for the Bloom filter
    ///
    /// For v2 format (QuotientFilter), the fingerprint is used directly,
    /// saving one xxhash64 computation per entry.
    ///
    /// Entries must be added in strictly sorted order by key.
    pub async fn add_with_fingerprint(&mut self, entry: &Entry, fp: Fingerprint) -> Result<()> {
        // Validate sorted order
        if !self.last_key.is_empty() && entry.key <= self.last_key {
            return Err(SSTableError::KeysNotSorted(
                self.last_key.to_vec(),
                entry.key.to_vec(),
            ));
        }

        // Track first key in SSTable
        if self.first_key.is_none() {
            self.first_key = Some(entry.key.clone());
        }

        // Add to bloom filter for v1 format (must hash, can't use QF fingerprint)
        if let Some(ref mut bloom) = self.bloom {
            bloom.add(&entry.key);
        }

        // Add to block with fingerprint (uses fingerprint directly for v2 format)
        self.block_builder.add_with_fingerprint(entry, fp)?;

        // Track first key of current block
        if self.block_first_key.is_none() {
            self.block_first_key = Some(entry.key.clone());
        }

        // Check if we need to flush the block
        if self.block_builder.current_size() >= self.config.block_size as usize {
            self.flush_block().await?;
        }

        self.last_key = entry.key.clone();
        self.entry_count += 1;

        // Track entry written
        self.meter.counter("sstable_entries_written", &[]).inc(1);

        Ok(())
    }

    /// Computes the fingerprint for a key using this builder's QF configuration.
    ///
    /// Returns `None` if this builder uses Bloom filters (v1 format).
    /// For v2 format, returns the fingerprint that would be used when adding the key.
    ///
    /// This is useful for pre-computing fingerprints during compaction.
    pub fn compute_fingerprint(&self, key: &[u8]) -> Option<Fingerprint> {
        self.block_builder.compute_fingerprint(key)
    }

    /// Returns true if this builder uses per-block Quotient Filters (v2 format).
    pub fn uses_quotient_filter(&self) -> bool {
        self.block_builder.has_quotient_filter()
    }

    /// Flushes the current block to disk and updates the index.
    async fn flush_block(&mut self) -> Result<()> {
        if self.block_builder.entry_count() == 0 {
            return Ok(()); // Nothing to flush
        }

        // Get the first key in this block
        let first_key = self
            .block_first_key
            .clone()
            .ok_or_else(|| SSTableError::InvalidFormat("no block first key".to_string()))?;

        // Finish the block (uncompressed)
        let block_data = self.block_builder.finish();
        let uncompressed_size = block_data.len();

        // Compress the block if compression is enabled
        let compressed_data = compress::compress(&block_data, self.config.compression)?;
        let compressed_size = compressed_data.len();

        // Write compressed block to file
        let block_offset = self.writer.write_block(&compressed_data).await?;

        // Add to index (store uncompressed size for reconstruction)
        self.index
            .add_block(first_key, block_offset, compressed_size as u32);

        // Track block flushed and bytes written (compressed size)
        self.meter.counter("sstable_blocks_flushed", &[]).inc(1);
        self.meter
            .counter("sstable_bytes_written", &[])
            .inc(compressed_size as u64);

        // Track compression ratio if compression is enabled
        if self.config.compression != Compression::None {
            let ratio = uncompressed_size as f64 / compressed_size.max(1) as f64;
            self.meter
                .histo(
                    "sstable_compression_ratio",
                    &[1.0, 1.5, 2.0, 3.0, 5.0, 10.0],
                    &[],
                )
                .observe(ratio);
        }

        // Reset for next block
        self.block_builder.reset();
        self.block_first_key = None;

        Ok(())
    }

    /// Finishes building the SSTable and writes all metadata.
    ///
    /// This flushes any remaining data, writes the index, bloom filter (v1), and footer,
    /// then syncs the file to disk.
    pub async fn finish(mut self) -> Result<SSTableMetadata> {
        // Flush any remaining block
        if self.block_builder.entry_count() > 0 {
            self.flush_block().await?;
        }

        let block_count = self.index.len();

        // Write index
        let (index_offset, index_size) = self.writer.write_index(&self.index).await?;

        // Write bloom filter and footer based on format
        let footer = match self.bloom {
            Some(ref bloom) => {
                // v1 format: write per-file bloom filter
                let (bloom_offset, bloom_size) = self.writer.write_bloom(bloom).await?;

                // Track metadata bytes written
                self.meter
                    .counter("sstable_bytes_written", &[])
                    .inc(index_size + bloom_size);

                Footer::new_bloom(
                    index_offset,
                    index_size,
                    bloom_offset,
                    bloom_size,
                    self.config.compression,
                    self.config.block_size,
                    self.entry_count,
                )
            }
            None => {
                // v2 format: no per-file bloom, QFs are inline in blocks
                self.meter
                    .counter("sstable_bytes_written", &[])
                    .inc(index_size);

                Footer::new_quotient(
                    index_offset,
                    index_size,
                    self.config.compression,
                    self.config.block_size,
                    self.entry_count,
                    self.config.qf_remainder_bits,
                )
            }
        };

        self.writer.write_footer(&footer).await?;

        // Track footer bytes written
        self.meter.counter("sstable_bytes_written", &[]).inc(64); // Footer is always 64 bytes

        // Sync to disk
        self.writer.sync().await?;

        let file_size = self.writer.bytes_written();
        let path = self.writer.path().clone();

        // Close file
        self.writer.finish().await?;

        // Track build duration
        let duration_ms = self.start_time.elapsed().as_secs_f64() * 1000.0;
        self.meter
            .histo(
                "sstable_build_duration_ms",
                &[1.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 5000.0],
                &[],
            )
            .observe(duration_ms);

        Ok(SSTableMetadata {
            path,
            entry_count: self.entry_count,
            file_size,
            block_count,
            compression: self.config.compression,
        })
    }

    /// Returns the number of entries added so far.
    pub fn entry_count(&self) -> u64 {
        self.entry_count
    }

    /// Returns the current file size in bytes.
    pub fn file_size(&self) -> u64 {
        self.writer.bytes_written()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_builder_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 10,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        builder
            .add(&Entry::put(&b"key1"[..], &b"value1"[..]))
            .await
            .unwrap();
        builder
            .add(&Entry::put(&b"key2"[..], &b"value2"[..]))
            .await
            .unwrap();
        builder
            .add(&Entry::put(&b"key3"[..], &b"value3"[..]))
            .await
            .unwrap();

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 3);
        assert_eq!(metadata.path, path);
        assert!(metadata.file_size > 0);
    }

    #[tokio::test]
    async fn test_builder_many_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 1000,
            block_size: 1024, // Small block size to force multiple blocks
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        // Add 1000 entries
        for i in 0..1000 {
            let key = format!("key{:04}", i);
            let value = format!("value{:04}", i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 1000);
        assert!(metadata.block_count > 1, "Should have multiple blocks");
        assert!(metadata.file_size > 0);
    }

    #[tokio::test]
    async fn test_builder_sorted_order() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path,
            estimated_entries: 10,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        builder
            .add(&Entry::put(&b"key2"[..], &b"value2"[..]))
            .await
            .unwrap();

        // Try to add out of order
        let result = builder.add(&Entry::put(&b"key1"[..], &b"value1"[..])).await;
        assert!(matches!(result, Err(SSTableError::KeysNotSorted(_, _))));
    }

    #[tokio::test]
    async fn test_builder_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 10,
            ..Default::default()
        };

        let builder = SSTableBuilder::new(config).await.unwrap();
        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 0);
        assert!(metadata.file_size > 0); // Still has index + bloom + footer
    }

    #[tokio::test]
    async fn test_builder_single_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 1,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        builder
            .add(&Entry::put(&b"only_key"[..], &b"only_value"[..]))
            .await
            .unwrap();

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 1);
        assert_eq!(metadata.block_count, 1);
    }

    #[tokio::test]
    async fn test_builder_block_flushing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            block_size: 256, // Very small to force frequent flushing
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        // Add entries with larger values to fill blocks
        for i in 0..50 {
            let key = format!("key{:04}", i);
            let value = "x".repeat(100); // Large value
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 50);
        assert!(
            metadata.block_count > 5,
            "Should have flushed multiple blocks"
        );
    }

    #[tokio::test]
    async fn test_builder_tombstones() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 10,
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

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 3);
    }

    // === V2 FORMAT TESTS (with Quotient Filter) ===

    #[tokio::test]
    async fn test_builder_v2_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        for i in 0..50 {
            let key = format!("key{:03}", i);
            let value = format!("val{:03}", i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 50);
        assert_eq!(metadata.path, path);
        assert!(metadata.file_size > 0);

        // Verify the file was created
        assert!(path.exists());
    }

    #[tokio::test]
    async fn test_builder_v2_multiple_blocks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2_multi.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 1000,
            block_size: 512, // Small block size to force multiple blocks
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        // Add entries with larger values to force block flushes
        for i in 0..200 {
            let key = format!("key{:04}", i);
            let value = "x".repeat(50); // ~50 byte values
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        let metadata = builder.finish().await.unwrap();

        assert_eq!(metadata.entry_count, 200);
        assert!(
            metadata.block_count > 1,
            "Should have multiple blocks, got {}",
            metadata.block_count
        );
    }

    #[tokio::test]
    async fn test_builder_v2_footer_format() {
        use crate::format::{Footer, FOOTER_SIZE, FORMAT_VERSION_QUOTIENT};

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_v2_footer.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 10,
            filter_type: FilterType::QuotientFilter,
            qf_remainder_bits: 7,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        builder.add(&Entry::put("key1", "val1")).await.unwrap();
        builder.add(&Entry::put("key2", "val2")).await.unwrap();
        builder.finish().await.unwrap();

        // Read and verify footer
        let file_data = tokio::fs::read(&path).await.unwrap();
        let footer_start = file_data.len() - FOOTER_SIZE;
        let footer_bytes: [u8; FOOTER_SIZE] = file_data[footer_start..].try_into().unwrap();
        let footer = Footer::decode(&footer_bytes).unwrap();

        assert_eq!(footer.format_version, FORMAT_VERSION_QUOTIENT);
        assert_eq!(footer.qf_remainder_bits, 7);
        assert_eq!(footer.entry_count, 2);
        assert!(footer.uses_quotient_filter());
        assert!(!footer.uses_bloom_filter());
        // v2 format has no per-file bloom
        assert_eq!(footer.bloom_offset, 0);
        assert_eq!(footer.bloom_size, 0);
    }

    #[tokio::test]
    async fn test_builder_v1_vs_v2_size_comparison() {
        let dir = TempDir::new().unwrap();

        // Build v1 format (Bloom)
        let v1_path = dir.path().join("v1.sst");
        let v1_config = SSTableConfig {
            path: v1_path.clone(),
            estimated_entries: 100,
            filter_type: FilterType::Bloom,
            ..Default::default()
        };
        let mut v1_builder = SSTableBuilder::new(v1_config).await.unwrap();
        for i in 0..100 {
            v1_builder
                .add(&Entry::put(format!("key{:03}", i), format!("val{:03}", i)))
                .await
                .unwrap();
        }
        let v1_meta = v1_builder.finish().await.unwrap();

        // Build v2 format (QF)
        let v2_path = dir.path().join("v2.sst");
        let v2_config = SSTableConfig {
            path: v2_path.clone(),
            estimated_entries: 100,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };
        let mut v2_builder = SSTableBuilder::new(v2_config).await.unwrap();
        for i in 0..100 {
            v2_builder
                .add(&Entry::put(format!("key{:03}", i), format!("val{:03}", i)))
                .await
                .unwrap();
        }
        let v2_meta = v2_builder.finish().await.unwrap();

        // Both should have same entry count
        assert_eq!(v1_meta.entry_count, v2_meta.entry_count);

        // Log sizes for comparison (v2 may be slightly larger due to per-block QFs)
        println!(
            "v1 (Bloom) size: {} bytes, v2 (QF) size: {} bytes",
            v1_meta.file_size, v2_meta.file_size
        );
    }
}
