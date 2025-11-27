# API Reference

Complete API documentation for nori-sstable.

---

---

## Overview

This section provides complete API documentation for all public types, methods, and traits in nori-sstable. For usage examples and guides, see [Getting Started](../getting-started).

{: .note }
> **Auto-generated API docs:** For exhaustive rustdoc documentation with all method signatures and examples, see the [rustdoc →](../rustdoc/)

---

## Quick Reference

### Core Types

| Type | Purpose | Page |
|------|---------|------|
| [`SSTableBuilder`](#sstablebuilder) | Builds immutable SSTables | [Builder API →](builder) |
| [`SSTableReader`](#sstablereader) | Reads from SSTables | [Reader API →](reader) |
| [`SSTableConfig`](#sstableconfig) | Configuration for building | [Config API →](config) |
| [`Entry`](#entry) | Key-value entry with tombstone | [Entry API →](entry) |
| [`SSTableIterator`](#sstableiterator) | Sequential iteration | [Iterator API →](iterator) |

### Enums

| Type | Purpose | Values |
|------|---------|--------|
| [`Compression`](#compression) | Block compression algorithm | `None`, `Lz4`, `Zstd` |

### Metadata Types

| Type | Purpose | Page |
|------|---------|------|
| [`SSTableMetadata`](#sstablemetadata) | Build result information | [Builder API →](builder) |
| [`Footer`](#footer) | SSTable file footer | [Internal] |
| [`Index`](#index) | Block index | [Internal] |
| [`BloomFilter`](#bloomfilter) | Bloom filter | [Internal] |

---

## Import Paths

```rust
// Main types
use nori_sstable::{
    SSTableBuilder,
    SSTableReader,
    SSTableConfig,
    SSTableMetadata,
    Entry,
    SSTableIterator,
};

// Enums
use nori_sstable::Compression;

// Errors
use nori_sstable::{Result, SSTableError};

// Re-exported for convenience
use nori_sstable::Bytes;  // From bytes crate
```

---

## API Conventions

### Async Methods

All I/O methods are async and require a Tokio runtime:

```rust
#[tokio::main]
async fn main() {
    builder.add(&entry).await?;  // Async!
    reader.get(b"key").await?;   // Async!
}
```

### Error Handling

All fallible operations return `Result<T, SSTableError>`:

```rust
// Propagate errors with ?
let position = builder.add(&entry).await?;

// Or match explicitly
match reader.get(b"key").await {
    Ok(Some(entry)) => println!("Found: {:?}", entry.value),
    Ok(None) => println!("Not found"),
    Err(e) => eprintln!("Error: {}", e),
}
```

### Thread Safety

All types are `Send + Sync` and can be shared:

```rust
// Readers are designed to be shared
let reader = Arc::new(SSTableReader::open(path).await?);

for _ in 0..4 {
    let reader_clone = reader.clone();
    tokio::spawn(async move {
        reader_clone.get(b"key").await
    });
}
```

---

## SSTableBuilder

**Purpose:** Builds immutable SSTables from sorted key-value entries.

### Methods

#### `new`

Creates a new builder with default metrics (NoopMeter).

```rust
pub async fn new(config: SSTableConfig) -> Result<Self>
```

**Example:**
```rust
let config = SSTableConfig {
    path: "data.sst".into(),
    estimated_entries: 1000,
    ..Default::default()
};
let mut builder = SSTableBuilder::new(config).await?;
```

---

#### `new_with_meter`

Creates a new builder with custom metrics.

```rust
pub async fn new_with_meter(
    config: SSTableConfig,
    meter: Box<dyn Meter>
) -> Result<Self>
```

**Example:**
```rust
use nori_observe_prom::PrometheusMeter;

let meter = Box::new(PrometheusMeter::new());
let mut builder = SSTableBuilder::new_with_meter(config, meter).await?;
```

---

#### `add`

Adds an entry to the SSTable. Entries must be added in sorted order.

```rust
pub async fn add(&mut self, entry: &Entry) -> Result<()>
```

**Errors:**
- `SSTableError::KeysNotSorted` - If entry key ≤ previous key

**Example:**
```rust
builder.add(&Entry::put("key1", "value1")).await?;
builder.add(&Entry::put("key2", "value2")).await?;  // Must be sorted!
```

---

#### `finish`

Finishes building the SSTable and writes all metadata.

```rust
pub async fn finish(self) -> Result<SSTableMetadata>
```

**Returns:** Metadata about the completed SSTable

**Example:**
```rust
let metadata = builder.finish().await?;
println!("Created {} entries in {} bytes",
    metadata.entry_count, metadata.file_size);
```

---

#### `entry_count`

Returns the number of entries added so far.

```rust
pub fn entry_count(&self) -> u64
```

---

#### `file_size`

Returns the current file size in bytes.

```rust
pub fn file_size(&self) -> u64
```

---

### SSTableMetadata

Metadata returned by `finish()`:

```rust
pub struct SSTableMetadata {
    pub path: PathBuf,           // Path to SSTable file
    pub entry_count: u64,        // Total entries written
    pub file_size: u64,          // Total file size in bytes
    pub block_count: usize,      // Number of data blocks
    pub compression: Compression, // Compression used
}
```

---

## SSTableReader

**Purpose:** Reads data from immutable SSTables with bloom filter and cache optimization.

### Methods

#### `open`

Opens an SSTable with default configuration (64MB cache).

```rust
pub async fn open(path: PathBuf) -> Result<Self>
```

**Example:**
```rust
let reader = Arc::new(SSTableReader::open("data.sst".into()).await?);
```

---

#### `open_with_meter`

Opens an SSTable with custom metrics (64MB cache).

```rust
pub async fn open_with_meter(
    path: PathBuf,
    meter: Arc<dyn Meter>
) -> Result<Self>
```

---

#### `open_with_config`

Opens an SSTable with custom cache size and metrics.

```rust
pub async fn open_with_config(
    path: PathBuf,
    meter: Arc<dyn Meter>,
    block_cache_mb: usize
) -> Result<Self>
```

**Example:**
```rust
use nori_observe::NoopMeter;

// 256MB cache for hot data
let reader = SSTableReader::open_with_config(
    "hot.sst".into(),
    Arc::new(NoopMeter),
    256
).await?;
```

---

#### `get`

Performs a point lookup for a key.

```rust
pub async fn get(&self, key: &[u8]) -> Result<Option<Entry>>
```

**Returns:**
- `Some(entry)` - Key found (check `entry.tombstone` for deletes)
- `None` - Key not found (bloom filter rejected or not in index)

**Example:**
```rust
if let Some(entry) = reader.get(b"key").await? {
    if entry.tombstone {
        println!("Key was deleted");
    } else {
        println!("Value: {:?}", entry.value);
    }
}
```

**Performance:**
- Bloom filter check: ~67ns
- Cache hit: ~5µs
- Cache miss: ~15µs (includes decompression)

---

#### `iter`

Returns an iterator over all entries in the SSTable.

```rust
pub fn iter(self: Arc<Self>) -> SSTableIterator
```

**Example:**
```rust
let mut iter = reader.clone().iter();
while let Some(entry) = iter.try_next().await? {
    println!("{:?} = {:?}", entry.key, entry.value);
}
```

---

#### `iter_range`

Returns an iterator over entries in a specific range.

```rust
pub fn iter_range(
    self: Arc<Self>,
    start_key: Bytes,
    end_key: Bytes
) -> SSTableIterator
```

**Range:** `[start_key, end_key)` - inclusive start, exclusive end

**Example:**
```rust
let mut iter = reader.iter_range(
    Bytes::from("key_0000"),
    Bytes::from("key_1000")
);
while let Some(entry) = iter.try_next().await? {
    // Only entries where key_0000 <= key < key_1000
}
```

---

#### `block_count`

Returns the number of data blocks in the SSTable.

```rust
pub fn block_count(&self) -> usize
```

---

## SSTableConfig

**Purpose:** Configuration for building SSTables.

### Fields

```rust
pub struct SSTableConfig {
    /// Path where the SSTable file will be written
    pub path: PathBuf,

    /// Estimated number of entries (for bloom filter sizing)
    pub estimated_entries: usize,

    /// Target block size in bytes (default: 4096)
    pub block_size: u32,

    /// Restart interval for prefix compression (default: 16)
    pub restart_interval: usize,

    /// Compression algorithm (default: None)
    pub compression: Compression,

    /// Bits per key for bloom filter (default: 10)
    pub bloom_bits_per_key: usize,

    /// Block cache size in MB for readers (default: 64)
    /// Set to 0 to disable caching
    pub block_cache_mb: usize,
}
```

### Default Configuration

```rust
impl Default for SSTableConfig {
    fn default() -> Self {
        Self {
            path: PathBuf::from("sstable.sst"),
            estimated_entries: 1000,
            block_size: 4096,              // 4KB
            restart_interval: 16,
            compression: Compression::None,
            bloom_bits_per_key: 10,        // ~0.9% FP rate
            block_cache_mb: 64,            // 64MB cache
        }
    }
}
```

### Configuration Examples

**Balanced (Production):**
```rust
SSTableConfig {
    path: "prod.sst".into(),
    estimated_entries: 100_000,
    compression: Compression::Lz4,
    block_cache_mb: 128,
    ..Default::default()
}
```

**High Compression (Archival):**
```rust
SSTableConfig {
    path: "archive.sst".into(),
    estimated_entries: 1_000_000,
    compression: Compression::Zstd,
    block_cache_mb: 0,  // Disable cache
    ..Default::default()
}
```

[See full configuration guide →](config)

---

## Entry

**Purpose:** Represents a key-value entry with optional tombstone marker.

### Structure

```rust
pub struct Entry {
    pub key: Bytes,        // Entry key
    pub value: Bytes,      // Entry value (empty for tombstones)
    pub tombstone: bool,   // true = deletion marker
}
```

### Methods

#### `put`

Creates a PUT entry (normal key-value pair).

```rust
pub fn put<K, V>(key: K, value: V) -> Self
where
    K: Into<Bytes>,
    V: Into<Bytes>,
```

**Example:**
```rust
let entry = Entry::put("key", "value");
let entry = Entry::put(b"key", b"value");
let entry = Entry::put(vec![1, 2], vec![3, 4]);
```

---

#### `delete`

Creates a DELETE entry (tombstone).

```rust
pub fn delete<K>(key: K) -> Self
where
    K: Into<Bytes>,
```

**Example:**
```rust
let entry = Entry::delete("key_to_delete");
```

---

[See full entry API →](entry)

---

## SSTableIterator

**Purpose:** Sequential iteration over SSTable entries.

### Methods

#### `try_next`

Returns the next entry in the SSTable.

```rust
pub async fn try_next(&mut self) -> Result<Option<Entry>>
```

**Returns:**
- `Some(entry)` - Next entry
- `None` - End of iteration

**Example:**
```rust
while let Some(entry) = iter.try_next().await? {
    println!("{:?}", entry.key);
}
```

---

#### `seek`

Seeks to the first key >= target.

```rust
pub async fn seek(&mut self, target: &[u8]) -> Result<()>
```

**Example:**
```rust
let mut iter = reader.iter();
iter.seek(b"key_0500").await?;

// Next entry will be >= "key_0500"
while let Some(entry) = iter.try_next().await? {
    println!("{:?}", entry.key);
}
```

---

[See full iterator API →](iterator)

---

## Compression

**Purpose:** Block compression algorithm selection.

### Variants

```rust
pub enum Compression {
    None = 0,   // No compression
    Lz4 = 1,    // LZ4 compression (fast, 3.9 GB/s decompress)
    Zstd = 2,   // Zstd compression (higher ratio, 1.2 GB/s decompress)
}
```

### Comparison

| Algorithm | Compression | Decompression | Ratio | Use Case |
|-----------|-------------|---------------|-------|----------|
| **None** | Instant | Instant | 1x | Testing, already compressed |
| **Lz4** | 750 MB/s | 3,900 MB/s | 2-3x | **Production (recommended)** |
| **Zstd** | 400 MB/s | 1,200 MB/s | 3-5x | Archival, cold storage |

[See compression guide →](../compression)

---

## Error Types

### SSTableError

```rust
pub enum SSTableError {
    /// I/O error from filesystem operations
    Io(io::Error),

    /// CRC checksum mismatch (data corruption)
    CrcMismatch { expected: u32, actual: u32 },

    /// Keys not inserted in sorted order
    KeysNotSorted(Vec<u8>, Vec<u8>),

    /// Compression operation failed
    CompressionFailed(String),

    /// Decompression operation failed
    DecompressionFailed(String),

    /// Invalid SSTable format or magic number
    InvalidFormat(String),

    /// SSTable file not found
    NotFound(String),

    /// Invalid configuration parameter
    InvalidConfig(String),

    /// Key not found in SSTable
    KeyNotFound,

    /// Incomplete data structure
    Incomplete,
}
```

### Result Type

```rust
pub type Result<T> = std::result::Result<T, SSTableError>;
```

---

## Observability

### Metrics via Meter Trait

nori-sstable emits metrics via the vendor-neutral `Meter` trait from `nori-observe`:

```rust
pub trait Meter: Send + Sync {
    fn counter(&self, name: &str, labels: &[(&str, &str)]) -> Box<dyn Counter>;
    fn histo(&self, name: &str, buckets: &[f64], labels: &[(&str, &str)]) -> Box<dyn Histogram>;
}
```

### Tracked Metrics

**Builder:**
- `sstable_build_duration_ms` - Build time histogram
- `sstable_entries_written` - Entry counter
- `sstable_blocks_flushed` - Block counter
- `sstable_bytes_written` - Byte counter
- `sstable_compression_ratio` - Compression ratio histogram (when enabled)

**Reader:**
- `sstable_open_duration_ms` - Open time histogram
- `sstable_get_duration_ms` - Lookup duration with `outcome` label
- `sstable_bloom_checks` - Bloom filter checks with `outcome` label
- `sstable_block_reads` - Disk I/O counter
- `sstable_block_cache_hits` - Cache hit counter 🆕
- `sstable_block_cache_misses` - Cache miss counter 🆕

---

## Type Categories

### Primary API (Start Here)

Most use cases only need these types:

- [`SSTableBuilder`](#sstablebuilder) - Build SSTables
- [`SSTableReader`](#sstablereader) - Read SSTables
- [`SSTableConfig`](#sstableconfig) - Configure behavior
- [`Entry`](#entry) - Create entries

### Advanced API

For fine-grained control:

- [`SSTableIterator`](#sstableiterator) - Custom iteration
- [`Compression`](#compression) - Algorithm selection
- [`SSTableMetadata`](#sstablemetadata) - Build results

### Internal Types

Not typically used directly:

- `Block` - Internal block structure
- `Index` - Block index
- `BloomFilter` - Bloom filter
- `Footer` - File footer

---

## Usage Patterns

### Basic Write-Read Cycle

```rust
// Write
let mut builder = SSTableBuilder::new(config).await?;
builder.add(&Entry::put("key", "value")).await?;
let metadata = builder.finish().await?;

// Read
let reader = Arc::new(SSTableReader::open(metadata.path).await?);
let entry = reader.get(b"key").await?;
```

### Batch Writes

```rust
let mut builder = SSTableBuilder::new(config).await?;

// Sort your data first!
let mut entries: Vec<_> = data.into_iter().collect();
entries.sort_by(|a, b| a.0.cmp(&b.0));

for (key, value) in entries {
    builder.add(&Entry::put(key, value)).await?;
}

builder.finish().await?;
```

### Concurrent Reads

```rust
let reader = Arc::new(SSTableReader::open(path).await?);

let mut handles = vec![];
for key in keys {
    let reader_clone = reader.clone();
    let handle = tokio::spawn(async move {
        reader_clone.get(&key).await
    });
    handles.push(handle);
}

// All tasks share the same cache
for handle in handles {
    handle.await??;
}
```

---

## Related Documentation

- **[Getting Started](../getting-started)** - Quick tutorial
- **[Compression](../compression)** - Compression deep dive
- **[Caching](../caching)** - Cache tuning guide
- **[Architecture](../architecture)** - File format details
- **[Performance](../performance/benchmarks)** - Benchmark results

---

## Summary

**nori-sstable API:**
- ✅ **Simple:** 5 core types cover 95% of use cases
- ✅ **Async:** All I/O is async via Tokio
- ✅ **Type-safe:** No unsafe code in public API
- ✅ **Thread-safe:** Share readers via `Arc`
- ✅ **Documented:** 108 tests + comprehensive docs

**Most common API:**
```rust
// Build
let mut builder = SSTableBuilder::new(config).await?;
builder.add(&Entry::put("key", "value")).await?;
builder.finish().await?;

// Read
let reader = Arc::new(SSTableReader::open(path).await?);
let entry = reader.get(b"key").await?;
```

For detailed method documentation, see the individual API pages or [rustdoc →](../rustdoc/)
