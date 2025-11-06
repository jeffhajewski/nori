//! Iterator for sequential scans over SSTable entries.
//!
//! The iterator loads blocks on demand and supports range scans with
//! efficient block skipping using the index.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                  SSTableIterator State Machine               │
//! │                                                              │
//! │   ┌────────┐  load_next_block()  ┌──────────────┐          │
//! │   │  Init  │ ──────────────────→ │ Block Loaded │          │
//! │   └────────┘                     └──────────────┘          │
//! │       │                                  │                   │
//! │       │                                  │ try_next()       │
//! │       │                                  ↓                   │
//! │       │                         ┌────────────────┐          │
//! │       │                         │ Return Entry   │          │
//! │       │                         └────────────────┘          │
//! │       │                                  │                   │
//! │       │                    block exhausted?                 │
//! │       │                                  ↓                   │
//! │       │                         ┌────────────────┐          │
//! │       └────────────────────────→│   Exhausted    │          │
//! │                                 └────────────────┘          │
//! │                                                              │
//! │  Buffered Entry (for seek):                                 │
//! │    seek() finds first entry >= target and buffers it        │
//! │    try_next() returns buffered entry before normal flow     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Performance
//!
//! - **Complexity**: O(K) for K entries in range (sequential I/O)
//! - **Memory**: O(1) per iterator (only current block loaded)
//! - **Throughput**: ~1M entries/sec (sequential scan)
//! - **Seek**: O(B + E) where B = blocks to scan, E = entries per block
//!
//! # Examples
//!
//! ## Full Table Scan
//!
//! ```no_run
//! # use nori_sstable::SSTableReader;
//! # use std::sync::Arc;
//! # use std::path::PathBuf;
//! # async fn example() -> nori_sstable::Result<()> {
//! let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);
//! let mut iter = reader.iter();
//!
//! while let Some(entry) = iter.try_next().await? {
//!     println!("{:?} = {:?}", entry.key, entry.value);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Range Scan
//!
//! ```no_run
//! # use nori_sstable::SSTableReader;
//! # use std::sync::Arc;
//! # use std::path::PathBuf;
//! # use bytes::Bytes;
//! # async fn example() -> nori_sstable::Result<()> {
//! let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);
//! let mut iter = reader.iter_range(
//!     Bytes::from("key_start"),
//!     Bytes::from("key_end"),
//! );
//!
//! while let Some(entry) = iter.try_next().await? {
//!     // Only entries where key_start <= key < key_end
//!     println!("{:?}", entry.key);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Seek Operation
//!
//! ```no_run
//! # use nori_sstable::SSTableReader;
//! # use std::sync::Arc;
//! # use std::path::PathBuf;
//! # async fn example() -> nori_sstable::Result<()> {
//! let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);
//! let mut iter = reader.iter();
//!
//! // Skip to first entry >= "middle_key"
//! iter.seek(b"middle_key").await?;
//!
//! // Continue iterating from that position
//! while let Some(entry) = iter.try_next().await? {
//!     println!("{:?}", entry.key);
//! }
//! # Ok(())
//! # }
//! ```

use crate::block::BlockIterator;
use crate::entry::Entry;
use crate::error::Result;
use crate::reader::SSTableReader;
use bytes::Bytes;
use std::sync::Arc;

/// Iterator over entries in an SSTable.
///
/// Supports full table scans and range scans. Blocks are loaded on demand
/// to minimize memory usage.
pub struct SSTableIterator {
    reader: Arc<SSTableReader>,
    current_block_idx: usize,
    block_iter: Option<BlockIterator>,
    start_key: Option<Bytes>,
    end_key: Option<Bytes>,
    exhausted: bool,
    // Buffered entry for seek positioning
    buffered_entry: Option<Entry>,
}

impl SSTableIterator {
    /// Creates a new iterator.
    ///
    /// If `start_key` and `end_key` are provided, iterates only over keys in that range.
    pub(crate) fn new(
        reader: Arc<SSTableReader>,
        start_key: Option<Bytes>,
        end_key: Option<Bytes>,
    ) -> Self {
        let current_block_idx = if let Some(ref start) = start_key {
            // Find the first block that might contain start_key
            reader.index().find_block(start).unwrap_or(0)
        } else {
            0
        };

        Self {
            reader,
            current_block_idx,
            block_iter: None,
            start_key,
            end_key,
            exhausted: false,
            buffered_entry: None,
        }
    }

    /// Returns the next entry in the SSTable.
    ///
    /// Returns `Ok(Some(entry))` if there is another entry,
    /// `Ok(None)` if iteration is complete, or `Err` on I/O errors.
    pub async fn try_next(&mut self) -> Result<Option<Entry>> {
        if self.exhausted {
            return Ok(None);
        }

        // If we have a buffered entry from seek, return it first
        if let Some(entry) = self.buffered_entry.take() {
            return Ok(Some(entry));
        }

        // If this is the first call or we exhausted the current block, load next block
        if self.block_iter.is_none() && !self.load_next_block().await? {
            self.exhausted = true;
            return Ok(None);
        }

        loop {
            // Try to get next entry from current block
            if let Some(ref mut iter) = self.block_iter {
                match iter.try_next()? {
                    Some(entry) => {
                        // Check if entry is within range bounds
                        if let Some(ref start) = self.start_key {
                            if entry.key.as_ref() < start.as_ref() {
                                continue; // Skip entries before start
                            }
                        }

                        if let Some(ref end) = self.end_key {
                            if entry.key.as_ref() >= end.as_ref() {
                                self.exhausted = true;
                                return Ok(None); // Reached end of range
                            }
                        }

                        return Ok(Some(entry));
                    }
                    None => {
                        // Current block exhausted, try next block
                        self.block_iter = None;
                        if !self.load_next_block().await? {
                            self.exhausted = true;
                            return Ok(None);
                        }
                    }
                }
            } else {
                // Should not happen, but handle gracefully
                self.exhausted = true;
                return Ok(None);
            }
        }
    }

    /// Seeks to the first key >= target.
    ///
    /// After seeking, the next call to `try_next()` will return the first entry
    /// with key >= target, or None if no such entry exists.
    pub async fn seek(&mut self, target: &[u8]) -> Result<()> {
        // Clear any buffered entry
        self.buffered_entry = None;

        // Find the block that might contain target
        let block_idx = match self.reader.index().find_block(target) {
            Some(idx) => idx,
            None => {
                // Target is before all blocks - set exhausted
                self.exhausted = true;
                return Ok(());
            }
        };

        // Position at the found block
        self.current_block_idx = block_idx;
        self.block_iter = None;
        self.exhausted = false;

        // Load the block
        if !self.load_next_block().await? {
            self.exhausted = true;
            return Ok(());
        }

        // Scan within the block to find the first entry >= target
        if let Some(ref mut iter) = self.block_iter {
            while let Some(entry) = iter.try_next()? {
                if entry.key.as_ref() >= target {
                    // Found it! Buffer this entry for the next try_next() call
                    self.buffered_entry = Some(entry);
                    return Ok(());
                }
                // Continue scanning if entry < target
            }
        }

        // If we didn't find an entry >= target in this block,
        // the next try_next() will naturally advance to the next block
        // and continue from there
        Ok(())
    }

    /// Loads the next block into the block iterator.
    ///
    /// Returns `true` if a block was loaded, `false` if no more blocks.
    async fn load_next_block(&mut self) -> Result<bool> {
        // Check if we've exhausted all blocks
        if self.current_block_idx >= self.reader.block_count() {
            return Ok(false);
        }

        // Skip blocks that are entirely before start_key
        if let Some(ref start) = self.start_key {
            while self.current_block_idx < self.reader.block_count() {
                let _entry = self.reader.index().get(self.current_block_idx).unwrap();

                // Check if next block (if exists) starts before our range
                if self.current_block_idx + 1 < self.reader.block_count() {
                    let next_entry = self.reader.index().get(self.current_block_idx + 1).unwrap();
                    if next_entry.first_key.as_ref() <= start.as_ref() {
                        self.current_block_idx += 1;
                        continue;
                    }
                }
                break;
            }

            if self.current_block_idx >= self.reader.block_count() {
                return Ok(false);
            }
        }

        // Skip blocks that are entirely after end_key
        if let Some(ref end) = self.end_key {
            let entry = self.reader.index().get(self.current_block_idx).unwrap();
            if entry.first_key.as_ref() >= end.as_ref() {
                return Ok(false);
            }
        }

        // Get block entry from index
        let entry = self.reader.index().get(self.current_block_idx).unwrap();

        // Read the block
        let block = self
            .reader
            .read_block(entry.block_offset, entry.block_size)
            .await?;

        self.block_iter = Some(block.iter());
        self.current_block_idx += 1;

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entry, SSTableBuilder, SSTableConfig};
    use std::sync::Arc;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_iterator_full_scan() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.sst");

        // Build test SSTable
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 10,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..10 {
            let key = format!("key{:02}", i);
            let value = format!("value{:02}", i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }
        builder.finish().await.unwrap();

        // Iterate and verify
        let reader = Arc::new(SSTableReader::open(path).await.unwrap());
        let mut iter = reader.iter();

        let mut count = 0;
        let mut last_key = Bytes::new();

        while let Some(entry) = iter.try_next().await.unwrap() {
            // Verify sorted order
            assert!(entry.key > last_key || last_key.is_empty());
            last_key = entry.key.clone();
            count += 1;
        }

        assert_eq!(count, 10);
    }

    #[tokio::test]
    async fn test_iterator_empty_sstable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 0,
            ..Default::default()
        };

        let builder = SSTableBuilder::new(config).await.unwrap();
        builder.finish().await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());
        let mut iter = reader.iter();

        assert!(iter.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_iterator_single_entry() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("single.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 1,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        builder
            .add(&Entry::put(&b"only"[..], &b"value"[..]))
            .await
            .unwrap();
        builder.finish().await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());
        let mut iter = reader.iter();

        let entry = iter.try_next().await.unwrap().unwrap();
        assert_eq!(entry.key.as_ref(), b"only");
        assert_eq!(entry.value.as_ref(), b"value");

        assert!(iter.try_next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_iterator_multiple_blocks() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("multi.sst");

        // Small block size to force multiple blocks
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 50,
            block_size: 512,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..50 {
            let key = format!("key{:04}", i);
            let value = "x".repeat(50);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }
        builder.finish().await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());
        assert!(reader.block_count() > 1);

        let mut iter = reader.iter();
        let mut count = 0;

        while let Some(_entry) = iter.try_next().await.unwrap() {
            count += 1;
        }

        assert_eq!(count, 50);
    }

    #[tokio::test]
    async fn test_iterator_range_scan() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("range.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..100 {
            let key = format!("key{:04}", i);
            let value = format!("value{:04}", i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }
        builder.finish().await.unwrap();

        // Range scan: key0020 to key0030
        let reader = Arc::new(SSTableReader::open(path).await.unwrap());
        let mut iter = reader.iter_range(Bytes::from("key0020"), Bytes::from("key0030"));

        let mut count = 0;
        let mut last_key = String::new();

        while let Some(entry) = iter.try_next().await.unwrap() {
            let key_str = String::from_utf8(entry.key.to_vec()).unwrap();
            assert!(key_str.as_str() >= "key0020");
            assert!(key_str.as_str() < "key0030");

            // Verify order
            if !last_key.is_empty() {
                assert!(key_str > last_key);
            }
            last_key = key_str;
            count += 1;
        }

        assert_eq!(count, 10); // key0020..key0029
    }
}
