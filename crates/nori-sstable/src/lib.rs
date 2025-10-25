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
