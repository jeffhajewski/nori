# Getting Started with nori-sstable

Build your first SSTable in 5 minutes.

---

---

## Installation

Add `nori-sstable` to your `Cargo.toml`:

```toml
[dependencies]
nori-sstable = "0.1"
tokio = { version = "1", features = ["full"] }
```

{: .note }
> nori-sstable requires `tokio` for async I/O. Make sure to include the `full` feature set or at minimum `fs`, `io-util`, and `rt`.

---

## Your First SSTable

### Step 1: Create a Builder

```rust
use nori_sstable::{SSTableBuilder, SSTableConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Configure the SSTable
    let config = SSTableConfig {
        path: PathBuf::from("my_first.sst"),
        estimated_entries: 1000,
        ..Default::default()
    };

    // Create the builder
    let mut builder = SSTableBuilder::new(config).await?;

    Ok(())
}
```

### Step 2: Add Entries

Entries must be added in **sorted order** by key:

```rust
use nori_sstable::Entry;

// Add key-value pairs
builder.add(&Entry::put("apple", "red fruit")).await?;
builder.add(&Entry::put("banana", "yellow fruit")).await?;
builder.add(&Entry::put("cherry", "red fruit")).await?;

// Add a tombstone (deletion marker)
builder.add(&Entry::delete("durian")).await?;
```

{: .warning }
> **Important:** Keys must be added in sorted order. If you try to add keys out of order, you'll get a `KeysNotSorted` error.

### Step 3: Finish Building

```rust
// Finalize the SSTable (writes index, bloom filter, footer)
let metadata = builder.finish().await?;

println!("✅ Created SSTable:");
println!("   Path: {:?}", metadata.path);
println!("   Entries: {}", metadata.entry_count);
println!("   Size: {} bytes", metadata.file_size);
println!("   Blocks: {}", metadata.block_count);
```

### Complete Example

```rust
use nori_sstable::{Entry, SSTableBuilder, SSTableConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Create builder
    let config = SSTableConfig {
        path: PathBuf::from("my_first.sst"),
        estimated_entries: 1000,
        ..Default::default()
    };
    let mut builder = SSTableBuilder::new(config).await?;

    // Add entries (sorted order!)
    builder.add(&Entry::put("apple", "red fruit")).await?;
    builder.add(&Entry::put("banana", "yellow fruit")).await?;
    builder.add(&Entry::put("cherry", "red fruit")).await?;

    // Finish
    let metadata = builder.finish().await?;
    println!("✅ Created SSTable with {} entries", metadata.entry_count);

    Ok(())
}
```

---

## Reading from an SSTable

### Step 1: Open a Reader

```rust
use nori_sstable::SSTableReader;
use std::sync::Arc;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Open the SSTable file
    let reader = Arc::new(
        SSTableReader::open(PathBuf::from("my_first.sst")).await?
    );

    Ok(())
}
```

{: .note }
> We wrap the reader in `Arc` because it's designed to be shared across threads. The reader is immutable and thread-safe.

### Step 2: Point Lookups

```rust
// Look up a single key
if let Some(entry) = reader.get(b"banana").await? {
    println!("Found: key={:?}, value={:?}",
        String::from_utf8_lossy(&entry.key),
        String::from_utf8_lossy(&entry.value)
    );
} else {
    println!("Key not found");
}
```

**What happens:**
1. Bloom filter check (~67ns) - might the key exist?
2. Index lookup (O(log B)) - which block contains the key?
3. Block read (cache or disk) - read the 4KB block
4. Binary search within block - find the exact entry

### Step 3: Range Scans

```rust
// Iterate over all entries
let mut iter = reader.clone().iter();

while let Some(entry) = iter.try_next().await? {
    println!("{:?} = {:?}",
        String::from_utf8_lossy(&entry.key),
        String::from_utf8_lossy(&entry.value)
    );
}
```

### Step 4: Range with Bounds

```rust
// Scan from "apple" to "cherry" (exclusive end)
let mut range_iter = reader.iter_range(
    Bytes::from("apple"),
    Bytes::from("cherry")
);

while let Some(entry) = range_iter.try_next().await? {
    // Only entries with apple <= key < cherry
    println!("{:?}", String::from_utf8_lossy(&entry.key));
}
```

### Complete Read Example

```rust
use nori_sstable::SSTableReader;
use std::sync::Arc;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Open reader
    let reader = Arc::new(SSTableReader::open(PathBuf::from("my_first.sst")).await?);

    // Point lookup
    if let Some(entry) = reader.get(b"banana").await? {
        println!("Found banana: {:?}", String::from_utf8_lossy(&entry.value));
    }

    // Full scan
    let mut iter = reader.iter();
    while let Some(entry) = iter.try_next().await? {
        println!("{:?} = {:?}",
            String::from_utf8_lossy(&entry.key),
            String::from_utf8_lossy(&entry.value)
        );
    }

    Ok(())
}
```

---

## Enabling Compression

### Why Compress?

- **Save storage:** 2-3x size reduction (LZ4) or 3-5x (Zstd)
- **Fast decompression:** LZ4 decompresses at 3.9 GB/s
- **Cache-friendly:** Decompressed blocks are cached

### Using LZ4 Compression (Recommended)

```rust
use nori_sstable::{SSTableConfig, Compression};

let config = SSTableConfig {
    path: PathBuf::from("compressed.sst"),
    estimated_entries: 1000,
    compression: Compression::Lz4,  // ✨ Enable compression
    ..Default::default()
};

let mut builder = SSTableBuilder::new(config).await?;
```

**Reading is automatic:**
```rust
// Reader automatically detects compression from footer
let reader = Arc::new(SSTableReader::open("compressed.sst".into()).await?);

// Reads work exactly the same - compression is transparent!
let entry = reader.get(b"key").await?;
```

{: .highlight }
> **Pro tip:** Combine compression with caching for best results. The default 64MB cache will store decompressed blocks, giving you storage savings + fast reads!

### Using Zstd Compression (Higher Ratio)

For cold storage or archival data:

```rust
let config = SSTableConfig {
    path: PathBuf::from("archive.sst"),
    estimated_entries: 10_000,
    compression: Compression::Zstd,  // Higher compression ratio
    block_cache_mb: 0,               // Disable cache for cold data
    ..Default::default()
};
```

[Learn more about compression →](compression)

---

## Configuring the Cache

### Default Cache (64MB)

The default configuration includes a 64MB LRU cache:

```rust
// This automatically includes 64MB cache
let reader = SSTableReader::open("data.sst".into()).await?;
```

### Custom Cache Size

For hot workloads, increase the cache:

```rust
use nori_observe::NoopMeter;

// 256MB cache for hot data
let reader = SSTableReader::open_with_config(
    "hot_data.sst".into(),
    Arc::new(NoopMeter),
    256  // MB
).await?;
```

### Disable Cache (Cold Storage)

For infrequently accessed data:

```rust
let config = SSTableConfig {
    block_cache_mb: 0,  // Disable caching
    ..Default::default()
};
```

[Learn more about caching →](caching)

---

## Error Handling

### Common Errors

```rust
use nori_sstable::{SSTableError, Result};

match builder.add(&Entry::put("key", "value")).await {
    Ok(_) => println!("Added successfully"),

    Err(SSTableError::KeysNotSorted(prev, current)) => {
        eprintln!("Error: Keys must be sorted!");
        eprintln!("Previous: {:?}, Current: {:?}", prev, current);
    }

    Err(SSTableError::Io(e)) => {
        eprintln!("I/O error: {}", e);
    }

    Err(e) => {
        eprintln!("Other error: {}", e);
    }
}
```

### Using the ? Operator

Most examples use `?` for error propagation:

```rust
async fn build_sstable() -> Result<()> {
    let mut builder = SSTableBuilder::new(config).await?;
    builder.add(&Entry::put("key", "value")).await?;
    builder.finish().await?;
    Ok(())
}
```

---

## Working with Entries

### PUT Entries

```rust
// From string slices
let entry = Entry::put("key", "value");

// From byte slices
let entry = Entry::put(b"key", b"value");

// From byte vectors
let entry = Entry::put(vec![1, 2, 3], vec![4, 5, 6]);

// From Bytes (zero-copy)
use bytes::Bytes;
let entry = Entry::put(Bytes::from("key"), Bytes::from("value"));
```

### DELETE Entries (Tombstones)

```rust
// Mark a key as deleted
let entry = Entry::delete("key_to_delete");

// When reading, tombstones are still returned
if let Some(entry) = reader.get(b"key_to_delete").await? {
    if entry.tombstone {
        println!("Key was deleted");
    }
}
```

### Checking for Tombstones

```rust
let entry = reader.get(b"key").await?;

match entry {
    Some(e) if e.tombstone => println!("Key deleted"),
    Some(e) => println!("Value: {:?}", e.value),
    None => println!("Key not found"),
}
```

---

## Configuration Options

### SSTableConfig

```rust
use nori_sstable::{SSTableConfig, Compression};

let config = SSTableConfig {
    // Required
    path: PathBuf::from("data.sst"),           // Output file path
    estimated_entries: 10_000,                 // For bloom filter sizing

    // Optional (with defaults)
    block_size: 4096,                          // 4KB blocks (default)
    restart_interval: 16,                      // Prefix compression restarts
    compression: Compression::Lz4,             // None, Lz4, or Zstd
    bloom_bits_per_key: 10,                    // ~0.9% false positive rate
    block_cache_mb: 64,                        // LRU cache size in MB
};
```

### Common Configurations

**Development (no compression, small cache):**
```rust
let config = SSTableConfig {
    path: "dev.sst".into(),
    estimated_entries: 100,
    compression: Compression::None,
    block_cache_mb: 16,
    ..Default::default()
};
```

**Production (LZ4 + cache):**
```rust
let config = SSTableConfig {
    path: "prod.sst".into(),
    estimated_entries: 100_000,
    compression: Compression::Lz4,
    block_cache_mb: 256,
    ..Default::default()
};
```

**Archival (Zstd, no cache):**
```rust
let config = SSTableConfig {
    path: "archive.sst".into(),
    estimated_entries: 1_000_000,
    compression: Compression::Zstd,
    block_cache_mb: 0,
    ..Default::default()
};
```

---

## Thread Safety

### Sharing Readers

Readers are designed to be shared via `Arc`:

```rust
let reader = Arc::new(SSTableReader::open("data.sst".into()).await?);

// Spawn multiple tasks sharing the same reader
for i in 0..4 {
    let reader_clone = reader.clone();
    tokio::spawn(async move {
        let key = format!("key{}", i);
        reader_clone.get(key.as_bytes()).await
    });
}
```

**Benefits:**
- Cache is shared across all tasks
- No duplication of index/bloom filter in memory
- Thread-safe concurrent reads

### Builders are NOT Thread-Safe

```rust
// ❌ Wrong - don't share builders
let builder = Arc::new(Mutex::new(builder));

// ✅ Correct - one builder per SSTable
let mut builder = SSTableBuilder::new(config).await?;
```

---

## Best Practices

### ✅ Do

- **Sort keys before adding** - use a BTreeMap or sort your data first
- **Estimate entries accurately** - improves bloom filter sizing
- **Use compression** - LZ4 is nearly free with caching
- **Enable caching** - 64MB default is reasonable
- **Wrap readers in Arc** - share across threads
- **Use `?` for error handling** - simplifies code

### ❌ Don't

- Don't add keys out of order (will error)
- Don't forget to call `finish()` on builder
- Don't share builders across threads
- Don't open multiple readers for the same file (share one)
- Don't disable cache unless memory is very limited

---

## Next Steps

### Learn More

- **[Compression Guide](compression)** - Deep dive into LZ4/Zstd
- **[Caching Guide](caching)** - Optimize for hot workloads
- **[Architecture](architecture)** - Understand the file format
- **[API Reference](api-reference/)** - Complete API documentation

### Examples

- **[Basic Usage](recipes/basic-usage)** - More complete examples
- **[Hot Workloads](recipes/hot-workloads)** - Cache tuning
- **[Cold Storage](recipes/cold-storage)** - Archival patterns

### Performance

- **[Benchmarks](performance/benchmarks)** - See performance numbers
- **[Tuning Guide](performance/tuning)** - Optimize for your workload

---

## Complete Example: Build and Read

Here's a complete program that builds and reads an SSTable:

```rust
use nori_sstable::{
    Entry, SSTableBuilder, SSTableReader, SSTableConfig, Compression
};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Build SSTable
    let config = SSTableConfig {
        path: PathBuf::from("example.sst"),
        estimated_entries: 100,
        compression: Compression::Lz4,
        block_cache_mb: 64,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    // Add sorted entries
    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = format!("value_{:04}", i);
        builder.add(&Entry::put(key, value)).await?;
    }

    let metadata = builder.finish().await?;
    println!("✅ Built SSTable: {} entries, {} bytes",
        metadata.entry_count, metadata.file_size);

    // Read SSTable
    let reader = Arc::new(SSTableReader::open(PathBuf::from("example.sst")).await?);

    // Point lookup
    if let Some(entry) = reader.get(b"key_0042").await? {
        println!("Found key_0042: {:?}", String::from_utf8_lossy(&entry.value));
    }

    // Range scan (first 10 keys)
    let mut iter = reader.iter();
    let mut count = 0;
    while let Some(entry) = iter.try_next().await? {
        println!("{:?} = {:?}",
            String::from_utf8_lossy(&entry.key),
            String::from_utf8_lossy(&entry.value)
        );
        count += 1;
        if count >= 10 {
            break;
        }
    }

    Ok(())
}
```

Run this with:
```bash
cargo run --example build_and_read
```

---

## Summary

You've learned how to:

- ✅ Install nori-sstable
- ✅ Build an SSTable with sorted entries
- ✅ Read data with point lookups and range scans
- ✅ Enable LZ4/Zstd compression
- ✅ Configure the LRU cache
- ✅ Handle errors properly
- ✅ Share readers across threads

**Next:** Explore [compression](compression) and [caching](caching) to optimize performance, or dive into the [API reference](api-reference/) for complete documentation.

Happy coding! 🚀
