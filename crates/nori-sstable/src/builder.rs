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
    Compression, Footer, BLOOM_BITS_PER_KEY, DEFAULT_BLOCK_SIZE, DEFAULT_RESTART_INTERVAL,
};
use crate::index::Index;
use crate::writer::SSTableWriter;
use bytes::Bytes;
use nori_observe::{Meter, NoopMeter};
use std::path::PathBuf;

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
    /// Bits per key for bloom filter (default: 10).
    pub bloom_bits_per_key: usize,
    /// Block cache size in MB for readers (default: 64). Set to 0 to disable caching.
    pub block_cache_mb: usize,
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
    bloom: BloomFilter,
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
        let block_builder = BlockBuilder::new(config.restart_interval);
        let index = Index::new();
        let bloom = BloomFilter::new(config.estimated_entries, config.bloom_bits_per_key);

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

        // Add to bloom filter (include tombstones - they exist in the table)
        self.bloom.add(&entry.key);

        // Try to add to current block
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
    /// This flushes any remaining data, writes the index, bloom filter, and footer,
    /// then syncs the file to disk.
    pub async fn finish(mut self) -> Result<SSTableMetadata> {
        // Flush any remaining block
        if self.block_builder.entry_count() > 0 {
            self.flush_block().await?;
        }

        let block_count = self.index.len();

        // Write index
        let (index_offset, index_size) = self.writer.write_index(&self.index).await?;

        // Write bloom filter
        let (bloom_offset, bloom_size) = self.writer.write_bloom(&self.bloom).await?;

        // Track metadata bytes written
        self.meter
            .counter("sstable_bytes_written", &[])
            .inc(index_size + bloom_size);

        // Write footer
        let footer = Footer {
            index_offset,
            index_size,
            bloom_offset,
            bloom_size,
            compression: self.config.compression,
            block_size: self.config.block_size,
            entry_count: self.entry_count,
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
}
