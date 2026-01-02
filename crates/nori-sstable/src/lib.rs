//! Immutable sorted string tables (SSTables) with blocks, index, bloom filters, and compression.
//!
//! SSTables are immutable, on-disk data structures that store sorted key-value pairs.
//! They are a fundamental building block for LSM-tree storage engines.
//!
//! # Architecture Overview
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                        Write Path                            │
//! │  User → SSTableBuilder → BlockBuilder → SSTableWriter        │
//! │           │                  │                │              │
//! │           ├─ BloomFilter     ├─ Prefix        └─ File I/O   │
//! │           │  (add keys)      │   Compression                 │
//! │           │                  │   (shared len)                │
//! │           └─ Index           └─ Restart                      │
//! │              (track blocks)     Points                       │
//! └─────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────┐
//! │                         Read Path                            │
//! │  User → SSTableReader → [Bloom Filter] → Index → Block      │
//! │                             ↓              ↓        ↓        │
//! │                          Contains?   Find Block  Binary      │
//! │                          (~65ns)     (O(log B))  Search      │
//! │                                                   (O(log E))  │
//! │         SSTableIterator → load blocks on demand              │
//! │                            ↓                                 │
//! │                         BlockIterator                        │
//! │                          (prefix decompress)                 │
//! └─────────────────────────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     File Layout                              │
//! │  ┌────────────────┐                                          │
//! │  │ Data Block 0   │ ← 4KB blocks with prefix compression    │
//! │  ├────────────────┤                                          │
//! │  │ Data Block 1   │ Entry: shared_len|unshared_len|         │
//! │  ├────────────────┤        value_len|key_suffix|value       │
//! │  │     ...        │                                          │
//! │  ├────────────────┤                                          │
//! │  │ Data Block N   │ Restart points every 16 entries         │
//! │  ├────────────────┤                                          │
//! │  │ Index          │ first_key|block_offset|block_size       │
//! │  ├────────────────┤                                          │
//! │  │ Bloom Filter   │ xxhash64 with double hashing            │
//! │  ├────────────────┤ 10 bits/key → ~0.9% FP rate             │
//! │  │ Footer (64B)   │ Magic|Index offset/size|Bloom offset/size│
//! │  └────────────────┘ CRC32C checksum                         │
//! └─────────────────────────────────────────────────────────────┘
//! ```
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
//! # Performance Characteristics
//!
//! - **Point lookups**: O(log B + log E) where B = number of blocks, E = entries per block
//!   - Bloom filter check: ~65ns (hit or miss)
//!   - Index binary search: O(log B) ≈ 10-100 blocks typical
//!   - Block binary search: O(log E) ≈ 16-256 entries per block
//!   - Total latency: ~50µs (hit), ~20µs (miss with bloom skip)
//! - **Range scans**: O(K) where K = number of entries in range (sequential I/O)
//!   - Throughput: ~1M entries/sec
//! - **Memory overhead**: ~12 bytes per entry (index + bloom filter)
//!
//! # Invariants
//!
//! ## Write Invariants
//! - Entries MUST be added in sorted order (checked at runtime)
//! - Keys MUST be non-empty
//! - Blocks are flushed when exceeding `block_size` (default 4KB)
//! - Restart points created every `restart_interval` entries (default 16)
//! - Bloom filter populated for ALL keys (including tombstones)
//!
//! ## Read Invariants
//! - Footer, index, and bloom filter loaded into memory on open
//! - Blocks loaded on-demand during iteration or point lookups
//! - Bloom filter has NO false negatives (may have ~0.9% false positives)
//! - Iterator returns entries in sorted order
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
mod compress;
mod entry;
mod error;
mod format;
mod index;
mod iterator;
mod quotient_filter;
mod reader;
pub mod vector_block;
mod writer;

pub use block::{Block, BlockBuilder, BlockIterator};
pub use bloom::BloomFilter;
pub use builder::{SSTableBuilder, SSTableConfig, SSTableMetadata};
pub use quotient_filter::{
    Fingerprint, QuotientFilter, QuotientFilterConfig, DEFAULT_LOAD_FACTOR,
    DEFAULT_QUOTIENT_BITS, DEFAULT_REMAINDER_BITS,
};
pub use entry::Entry;
pub use error::{Result, SSTableError};
pub use format::{
    Compression, Footer, BLOOM_BITS_PER_KEY, BLOOM_FP_RATE, DEFAULT_BLOCK_SIZE,
    DEFAULT_QF_REMAINDER_BITS, DEFAULT_RESTART_INTERVAL, FOOTER_SIZE, FORMAT_VERSION_BLOOM,
    FORMAT_VERSION_QUOTIENT, SSTABLE_MAGIC,
};
pub use index::{Index, IndexEntry};
pub use iterator::SSTableIterator;
pub use reader::SSTableReader;
pub use vector_block::{
    DistanceFn, RawVectorBlockBuilder, RawVectorBlockReader, VectorBlockHeader,
    VectorBlockType, VectorEntry, VECTOR_BLOCK_MAGIC, VECTOR_BLOCK_VERSION,
};
pub use writer::SSTableWriter;

// Re-export for convenience
pub use bytes::Bytes;
