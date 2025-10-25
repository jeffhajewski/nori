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
use crate::entry::Entry;
use crate::error::{Result, SSTableError};
use crate::format::Footer;
use crate::index::Index;
use crate::iterator::SSTableIterator;
use bytes::Bytes;
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
    bloom: BloomFilter,
    file_size: u64,
}

impl SSTableReader {
    /// Opens an SSTable file for reading.
    ///
    /// This reads and caches the footer, index, and bloom filter in memory.
    pub async fn open(path: PathBuf) -> Result<Self> {
        let mut file = File::open(&path)
            .await
            .map_err(SSTableError::Io)?;

        // Get file size
        let file_size = file
            .metadata()
            .await
            .map_err(SSTableError::Io)?
            .len();

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

        // Read bloom filter
        file.seek(std::io::SeekFrom::Start(footer.bloom_offset))
            .await
            .map_err(SSTableError::Io)?;

        let mut bloom_bytes = vec![0u8; footer.bloom_size as usize];
        file.read_exact(&mut bloom_bytes)
            .await
            .map_err(SSTableError::Io)?;

        let bloom = BloomFilter::decode(&bloom_bytes)?;

        Ok(Self {
            file: Mutex::new(file),
            path,
            footer,
            index,
            bloom,
            file_size,
        })
    }

    /// Looks up a key in the SSTable.
    ///
    /// Returns `None` if the key is not present (using bloom filter for fast negative lookups).
    /// Returns `Some(entry)` if found, which may be a tombstone.
    pub async fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        // Check bloom filter first (fast negative lookup)
        if !self.bloom.contains(key) {
            return Ok(None);
        }

        // Find which block might contain the key
        let block_idx = match self.index.find_block(key) {
            Some(idx) => idx,
            None => return Ok(None), // Key is before first block
        };

        // Read the block
        let entry = self.index.get(block_idx).unwrap();
        let block = self.read_block(entry.block_offset, entry.block_size).await?;

        // Search within the block
        block.get(key)
    }

    /// Reads a specific block from disk.
    pub(crate) async fn read_block(&self, offset: u64, size: u32) -> Result<Block> {
        let mut file = self.file.lock().await;

        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(SSTableError::Io)?;

        let mut block_bytes = vec![0u8; size as usize];
        file.read_exact(&mut block_bytes)
            .await
            .map_err(SSTableError::Io)?;

        Block::decode(Bytes::from(block_bytes))
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
    #[cfg(test)]
    pub(crate) fn bloom(&self) -> &BloomFilter {
        &self.bloom
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
        assert!(reader.bloom().contains(b"key0000"));
        assert!(reader.bloom().contains(b"key0050"));

        // This key wasn't added, bloom should (likely) reject it
        let not_present = reader.get(b"key9999").await.unwrap();
        assert!(not_present.is_none());
    }

    #[tokio::test]
    async fn test_reader_empty_sstable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.sst");

        build_test_sstable(path.clone(), vec![])
            .await
            .unwrap();

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
        tokio::fs::write(&path, b"invalid data")
            .await
            .unwrap();

        // Should fail to open
        let result = SSTableReader::open(path).await;
        assert!(result.is_err());
    }
}
