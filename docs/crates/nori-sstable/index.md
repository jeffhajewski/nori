# nori-sstable

Immutable sorted string tables with blocks, index, bloom filters, and compression.

[Get Started](getting-started.md){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API Reference](api-reference/index.md){: .btn .fs-5 .mb-4 .mb-md-0 .mr-2 }
[View on GitHub](https://github.com/jeffhajewski/norikv/tree/main/crates/nori-sstable){: .btn .fs-5 .mb-4 .mb-md-0 }

---

---

## What is nori-sstable?

**nori-sstable** provides a production-ready implementation of SSTables (Sorted String Tables) for building LSM-tree storage engines. SSTables are immutable, on-disk data structures that store sorted key-value pairs with efficient lookups and range scans.

This crate is a core building block of the NoriKV distributed key-value store, but can be used standalone in any Rust project that needs persistent sorted storage.

---

## Key Features

### Storage & Format
-  **Block-based storage**: 4KB blocks with prefix compression for space efficiency
-  **Two-level index**: Sparse block index for fast key lookups with minimal I/O
-  **Bloom filters**: Probabilistic membership testing (~0.9% false positive rate) to avoid unnecessary disk reads
-  **CRC32C checksums**: Data integrity validation at block level

### Performance & Optimization
-  **Block compression**: Optional LZ4 (fast, 3.9 GB/s decompress) or Zstd (higher ratio) compression 🆕
-  **LRU block cache**: Configurable in-memory cache for hot data blocks (default 64MB) 🆕
-  **Async I/O**: Built on Tokio for high-performance asynchronous operations
-  **~67ns bloom filter checks**: Ultra-fast negative lookups

### Developer Experience
-  **Tombstones**: Explicit delete markers preserved through the storage layer
-  **Observability**: Vendor-neutral metrics via `nori-observe::Meter` trait
-  **Thread-safe**: Concurrent reads supported via `Arc<SSTableReader>`
-  **100% Safe Rust**: No unsafe code in the public API
-  **108 tests passing**: Comprehensive test suite including compression and property tests

{: .new }
> **Recent Updates:** nori-sstable now includes production-ready LZ4/Zstd compression and an LRU block cache that delivers 18x performance improvements for hot key workloads!

---

## Quick Example

```rust
use nori_sstable::{Compression, Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    // Build an SSTable with LZ4 compression
    let config = SSTableConfig {
        path: PathBuf::from("data.sst"),
        estimated_entries: 1000,
        compression: Compression::Lz4,  // Enable compression
        block_cache_mb: 64,              // 64MB cache (default)
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    // Add entries in sorted order
    builder.add(&Entry::put("key1", "value1")).await?;
    builder.add(&Entry::put("key2", "value2")).await?;
    builder.add(&Entry::delete("key3")).await?; // Tombstone

    let metadata = builder.finish().await?;
    println!("Created SSTable with {} entries", metadata.entry_count);

    // Read data back
    let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);

    // Point lookup (uses bloom filter + index + cache)
    if let Some(entry) = reader.get(b"key1").await? {
        println!("Found: {:?}", String::from_utf8_lossy(&entry.value));
    }

    // Range scan
    let mut iter = reader.iter();
    while let Some(entry) = iter.try_next().await? {
        println!("{:?} = {:?}", entry.key, entry.value);
    }

    Ok(())
}
```

---

## Performance Highlights

Benchmarked on Apple M1 (release build):

| Operation | Performance | Notes |
|-----------|-------------|-------|
| **Bloom filter check** | ~67ns | In-memory, ultra-fast |
| **Point lookup (hit)** | ~5µs | With cache enabled |
| **Point lookup (miss)** | <20µs | Bloom filter skip |
| **Sequential scan** | ~1M entries/sec | Streaming iteration |
| **Hot key workload** | **18x faster** (14ms → 777µs) | With 64MB cache |
| **Compression ratio (LZ4)** | 2-3x typical | 14x on highly compressible data |

[See detailed benchmarks →](performance/benchmarks.md)

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        Write Path                            │
│  User → SSTableBuilder → BlockBuilder → Compress → Writer   │
│           │                  │              │          │     │
│           ├─ BloomFilter     ├─ Prefix      │          └─ File I/O
│           │  (add keys)      │   Compression│                │
│           │                  │   (shared len)                │
│           └─ Index           └─ Restart                      │
│              (track blocks)     Points                       │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│                         Read Path                            │
│  User → SSTableReader → [Cache?] → [Bloom?] → Index → Block │
│                             ↓          ↓         ↓        ↓  │
│                          Hit/Miss  Contains?  Find    Binary │
│                          (~18x)    (~67ns)    Block   Search │
│                                                (log B)  (log E)│
│         SSTableIterator → load blocks on demand              │
│                            ↓                                 │
│                         BlockIterator                        │
│                          (decompress + decode)               │
└─────────────────────────────────────────────────────────────┘
```

### File Layout

```
┌────────────────┐
│ Data Block 0   │ ← 4KB blocks (compressed with LZ4/Zstd)
├────────────────┤
│ Data Block 1   │   Entry: shared_len | unshared_len |
├────────────────┤          value_len | key | value
│     ...        │
├────────────────┤   Restart points every 16 entries
│ Data Block N   │
├────────────────┤
│ Index          │ first_key | block_offset | block_size (per block)
├────────────────┤
│ Bloom Filter   │ xxhash64 with double hashing
├────────────────┤ 10 bits/key → ~0.9% FP rate
│ Footer (64B)   │ Magic | Index offset/size | Bloom offset/size |
└────────────────┘ Compression type | CRC32C checksum
```

[Learn more about the architecture →](how-it-works/index.md)

---

## When to Use nori-sstable

###  Great Fit

- Building **LSM-tree storage engines** (like RocksDB, LevelDB)
- **Time-series databases** with immutable time-ordered data
- **Log storage** with sorted logs and range queries
- **Snapshot storage** for point-in-time database snapshots
- Need **fast reads** with bloom filter optimization
- **Hot key workloads** where caching provides 10x+ speedups
- Workloads where **compression** saves significant storage costs

###  Not the Right Tool

- Need **mutable data structures** (use in-memory B-trees instead)
- **Random writes** (SSTables are write-once, immutable)
- Ultra-low latency **< 1µs** (use in-memory stores)
- Very small datasets **< 1MB** (overhead not worth it)

---

## Documentation Sections

### Getting Started
Learn how to install and use nori-sstable in your project.

[Getting Started →](getting-started.md)

### Core Concepts
Understand the fundamental concepts behind SSTables: immutability, block-based storage, bloom filters, and when to use them.

[Core Concepts →](core-concepts/index.md)

### Compression 🆕
Deep dive into LZ4 and Zstd compression: when to use each, performance tradeoffs, and configuration.

[Compression Guide →](compression.md)

### Caching 🆕
Learn how the LRU block cache works, how to tune it for hot workloads, and achieve 18x speedups.

[Caching Guide →](caching.md)

### How It Works
Detailed internals: file format, block format, bloom filters, index structure, compression, and cache implementation.

[How It Works →](how-it-works/index.md)

### API Reference
Complete API documentation for builders, readers, configuration, and iterators.

[API Reference →](api-reference/index.md)

### Performance
Benchmarks, tuning guides, compression ratio analysis, and profiling.

[Performance →](performance/index.md)

### Design Decisions
Rationale behind key design choices: block-based organization, immutability, compression strategy, and more.

[Design Decisions →](design-decisions/index.md)

### Recipes
Common patterns and use cases with code examples.

[Recipes →](recipes/index.md)

### Internals
Deep implementation details for contributors and advanced users.

[Internals →](internals/index.md)

---

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
nori-sstable = "0.1"
tokio = { version = "1", features = ["full"] }
```

Then import in your code:

```rust
use nori_sstable::{
    SSTableBuilder, SSTableReader, SSTableConfig,
    Entry, Compression,
};
```

---

## Project Status

| Aspect | Status |
|--------|--------|
| **Core functionality** |  Production-ready |
| **Tests** |  108 tests passing |
| **Compression** |  LZ4 + Zstd supported |
| **Caching** |  LRU cache implemented |
| **Documentation** |  Complete |
| **Benchmarks** |  Comprehensive |
| **Published** | 🚧 Preparing for crates.io |

---

## Contributing

nori-sstable is part of the [NoriKV](https://github.com/jeffhajewski/norikv) project and welcomes contributions!

- **Found a bug?** [Open an issue](https://github.com/jeffhajewski/norikv/issues)
- **Have a question?** [Start a discussion](https://github.com/jeffhajewski/norikv/discussions)
- **Want to contribute?** Check the [Contributing Guide](https://github.com/jeffhajewski/norikv/blob/main/CONTRIBUTING.md)

---

## License

MIT License - see [LICENSE](https://github.com/jeffhajewski/norikv/blob/main/LICENSE) for details.

---

## Next Steps

<div class="code-example" markdown="1">

**New to SSTables?**
Start with [Getting Started](getting-started.md) to build your first SSTable in 5 minutes.

**Want to optimize performance?**
Check out [Compression](compression.md) and [Caching](caching.md) to learn about our newest performance features.

**Need the API?**
Jump to [API Reference](api-reference/index.md) for complete method documentation.

**Curious about internals?**
Dive into [How It Works](how-it-works/index.md) for implementation details.

</div>
