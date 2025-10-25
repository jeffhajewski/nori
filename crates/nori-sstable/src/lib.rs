//! Immutable sorted string tables (SSTables) with blocks, index, bloom filters, and compression.
//!
//! SSTables are immutable, on-disk data structures that store sorted key-value pairs.
//! They are a fundamental building block for LSM-tree storage engines.
//!
//! # Features
//!
//! - **Block-based storage**: 4KB blocks with prefix compression
//! - **Two-level index**: Fast key lookups with minimal I/O
//! - **Bloom filters**: Reduce disk I/O for non-existent keys (~0.9% false positive rate)
//! - **Compression**: Optional LZ4 or Zstd compression
//! - **CRC32C checksums**: Data integrity validation
//! - **Observability**: Vendor-neutral metrics via `nori-observe::Meter` trait
//!
//! # Example
//!
//! ```
//! use nori_sstable::{Entry, BlockBuilder};
//!
//! // Create a block with entries
//! let mut builder = BlockBuilder::new(16);
//! builder.add(&Entry::put(&b"key1"[..], &b"value1"[..])).unwrap();
//! builder.add(&Entry::put(&b"key2"[..], &b"value2"[..])).unwrap();
//!
//! // Finish the block
//! let block_data = builder.finish();
//! println!("Block size: {} bytes", block_data.len());
//! ```
//!
//! # Observability
//!
//! The crate supports vendor-neutral observability via the `nori-observe::Meter` trait.
//! Metrics are disabled by default (using `NoopMeter`) for zero overhead.
//!
//! ## Tracked Metrics
//!
//! ### Builder Metrics
//!
//! - `sstable_build_duration_ms` (histogram): Total time to build an SSTable
//! - `sstable_entries_written` (counter): Number of entries added
//! - `sstable_blocks_flushed` (counter): Number of blocks written
//! - `sstable_bytes_written` (counter): Total bytes written (data + metadata + footer)
//!
//! ### Reader Metrics
//!
//! - `sstable_open_duration_ms` (histogram): Time to open an SSTable
//! - `sstable_get_duration_ms` (histogram): Lookup duration with outcome label:
//!   - `outcome=bloom_skip`: Bloom filter rejected the key
//!   - `outcome=hit`: Key found in SSTable
//!   - `outcome=miss`: Key not found
//! - `sstable_bloom_checks` (counter): Bloom filter checks with outcome:
//!   - `outcome=skip`: Bloom filter rejected (fast path)
//!   - `outcome=pass`: Bloom filter passed (requires disk I/O)
//! - `sstable_block_reads` (counter): Number of blocks read from disk
//!
//! ## Example with Metrics
//!
//! ```no_run
//! use nori_sstable::{SSTableBuilder, SSTableConfig, SSTableReader, Entry};
//! use nori_observe::NoopMeter;
//! use std::sync::Arc;
//! use std::path::PathBuf;
//!
//! # async fn example() -> nori_sstable::Result<()> {
//! // Create a builder with custom meter (or use ::new() for NoopMeter)
//! let config = SSTableConfig {
//!     path: PathBuf::from("/tmp/test.sst"),
//!     estimated_entries: 1000,
//!     ..Default::default()
//! };
//!
//! let meter = Box::new(NoopMeter);
//! let mut builder = SSTableBuilder::new_with_meter(config, meter).await?;
//!
//! builder.add(&Entry::put("key1", "value1")).await?;
//! builder.add(&Entry::put("key2", "value2")).await?;
//!
//! let metadata = builder.finish().await?;
//!
//! // Open reader with custom meter (or use ::open() for NoopMeter)
//! let meter = Arc::new(NoopMeter);
//! let reader = SSTableReader::open_with_meter(
//!     PathBuf::from("/tmp/test.sst"),
//!     meter,
//! ).await?;
//!
//! // Lookups are automatically instrumented
//! if let Some(entry) = reader.get(b"key1").await? {
//!     println!("Found: {:?}", entry.value);
//! }
//! # Ok(())
//! # }
//! ```

mod block;
mod bloom;
mod builder;
mod entry;
mod error;
mod format;
mod index;
mod iterator;
mod reader;
mod writer;

pub use block::{Block, BlockBuilder, BlockIterator};
pub use bloom::BloomFilter;
pub use builder::{SSTableBuilder, SSTableConfig, SSTableMetadata};
pub use entry::Entry;
pub use error::{Result, SSTableError};
pub use format::{
    Compression, Footer, BLOOM_BITS_PER_KEY, BLOOM_FP_RATE, DEFAULT_BLOCK_SIZE,
    DEFAULT_RESTART_INTERVAL, FOOTER_SIZE, SSTABLE_MAGIC,
};
pub use index::{Index, IndexEntry};
pub use iterator::SSTableIterator;
pub use reader::SSTableReader;
pub use writer::SSTableWriter;

// Re-export for convenience
pub use bytes::Bytes;
