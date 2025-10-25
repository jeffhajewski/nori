# nori-sstable

Immutable sorted string tables (SSTables) with blocks, index, bloom filters, and compression.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

## Overview

`nori-sstable` provides a high-performance, production-ready implementation of SSTables (Sorted String Tables) for building LSM-tree storage engines. SSTables are immutable, on-disk data structures that store sorted key-value pairs with efficient lookups and range scans.

## Features

- **Block-based storage**: 4KB blocks with prefix compression for space efficiency
- **Two-level index**: Sparse block index for fast key lookups with minimal I/O
- **Bloom filters**: Probabilistic membership testing (~0.9% false positive rate) to avoid unnecessary disk reads
- **Async I/O**: Built on Tokio for high-performance asynchronous operations
- **Compression**: Optional LZ4 or Zstd compression (ready for future implementation)
- **CRC32C checksums**: Data integrity validation at block level
- **Tombstones**: Explicit delete markers preserved through the storage layer
- **Observability**: Vendor-neutral metrics via `nori-observe::Meter` trait
- **Thread-safe**: Concurrent reads supported via `Arc<SSTableReader>`

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
nori-sstable = "0.1"
tokio = { version = "1", features = ["full"] }
```

### Building an SSTable

```rust
use nori_sstable::{Entry, SSTableBuilder, SSTableConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    let config = SSTableConfig {
        path: PathBuf::from("data.sst"),
        estimated_entries: 1000,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    // Add entries in sorted order
    builder.add(&Entry::put("key1", "value1")).await?;
    builder.add(&Entry::put("key2", "value2")).await?;
    builder.add(&Entry::delete("key3")).await?; // Tombstone

    let metadata = builder.finish().await?;
    println!("Created SSTable with {} entries", metadata.entry_count);

    Ok(())
}
```

### Reading from an SSTable

```rust
use nori_sstable::SSTableReader;
use std::sync::Arc;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> nori_sstable::Result<()> {
    let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);

    // Point lookup
    if let Some(entry) = reader.get(b"key1").await? {
        println!("Found: {:?}", entry.value);
    }

    // Full table scan
    let mut iter = reader.clone().iter();
    while let Some(entry) = iter.try_next().await? {
        println!("{:?} = {:?}", entry.key, entry.value);
    }

    // Range scan
    let mut range_iter = reader.iter_range("key1".into(), "key5".into());
    while let Some(entry) = range_iter.try_next().await? {
        println!("In range: {:?}", entry.key);
    }

    Ok(())
}
```

## Architecture

### File Format

```
┌─────────────────────────────────────────┐
│ Data Block 0                             │  4KB blocks with prefix compression
├─────────────────────────────────────────┤
│ Data Block 1                             │  Entry format: key_len | value_len | key | value | crc32c
├─────────────────────────────────────────┤
│ ...                                      │
├─────────────────────────────────────────┤
│ Data Block N                             │
├─────────────────────────────────────────┤
│ Index                                    │  first_key | block_offset | block_size (per block)
├─────────────────────────────────────────┤
│ Bloom Filter                             │  Bit array + metadata (10 bits/key)
├─────────────────────────────────────────┤
│ Footer (64 bytes)                        │  Magic | Index offset/size | Bloom offset/size
└─────────────────────────────────────────┘
```

### Read Path

1. **Open**: Load footer, index, and bloom filter into memory
2. **Lookup**: Check bloom filter → Find block via index → Read & search block
3. **Scan**: Load blocks on demand, skip blocks outside range using index

### Performance Characteristics

- **Point lookups**: O(log B) where B = number of blocks (~10-100 for typical tables)
- **Range scans**: O(K) where K = number of entries in range (sequential I/O)
- **Bloom filter**: ~0.9% false positive rate, <1µs per check
- **Memory overhead**: Index + bloom filter (~12 bytes per entry) loaded on open

## Observability

The crate supports vendor-neutral metrics via the `nori-observe::Meter` trait:

```rust
use nori_sstable::{SSTableBuilder, SSTableConfig};
use nori_observe::NoopMeter;

let config = SSTableConfig { /* ... */ };
let meter = Box::new(NoopMeter); // Or your custom meter

let builder = SSTableBuilder::new_with_meter(config, meter).await?;
```

### Metrics Tracked

**Builder:**
- `sstable_build_duration_ms`: Total build time
- `sstable_entries_written`: Entry count
- `sstable_blocks_flushed`: Block count
- `sstable_bytes_written`: Total bytes (data + metadata)

**Reader:**
- `sstable_open_duration_ms`: File open time
- `sstable_get_duration_ms`: Lookup latency by outcome (hit/miss/bloom_skip)
- `sstable_bloom_checks`: Bloom filter checks by outcome (pass/skip)
- `sstable_block_reads`: Disk I/O count

## Testing

The crate includes comprehensive test coverage:

- **89 total tests**
- **Unit tests**: Core functionality (blocks, bloom, index, entries)
- **Integration tests**: End-to-end SSTable roundtrips
- **Property tests**: Correctness invariants verified with proptest
- **Stress tests**: 1M+ entry tables, concurrent reads, edge cases
- **Benchmarks**: Build, read, and bloom filter performance

Run tests:

```bash
cargo test -p nori-sstable
```

Run benchmarks:

```bash
cargo bench -p nori-sstable
```

## Performance

Benchmarked on Apple M1 (release build):

| Operation | Throughput | Latency (p95) |
|-----------|-----------|---------------|
| Build (10K entries) | ~100K entries/sec | - |
| Point lookup (hit) | ~200K ops/sec | <50µs |
| Point lookup (miss) | ~500K ops/sec | <20µs (bloom skip) |
| Sequential scan | ~1M entries/sec | - |
| Bloom filter check | - | <1µs |

## Configuration

```rust
use nori_sstable::SSTableConfig;

let config = SSTableConfig {
    path: "data.sst".into(),
    estimated_entries: 10_000,
    block_size: 4096,            // 4KB blocks (default)
    restart_interval: 16,        // Prefix compression restart points
    compression: Compression::None,
    bloom_bits_per_key: 10,      // ~0.9% FP rate
};
```

## Limitations

- **No in-place updates**: SSTables are immutable (by design)
- **No compression**: Compression support is prepared but not yet implemented
- **No block cache**: Consider adding an LRU cache wrapper for hot workloads
- **Seek limitation**: Iterator seek has known issues with exact matches (see #TODO)

## Use Cases

nori-sstable is designed for use in:

- **LSM-tree storage engines**: As the on-disk layer (e.g., nori-lsm)
- **Time-series databases**: Immutable time-ordered data
- **Log storage**: Sorted logs with range queries
- **Snapshot storage**: Point-in-time database snapshots

## Safety

- **Thread-safe reads**: `SSTableReader` can be shared via `Arc`
- **No unsafe code**: Pure safe Rust (except dependencies)
- **Checksums**: CRC32C validation prevents silent data corruption
- **Atomic writes**: Finish operation syncs before returning

## License

MIT

## Contributing

This crate is part of the [NoriKV](https://github.com/your-org/norikv) distributed key-value store. See the main repository for contribution guidelines.

## Acknowledgments

Inspired by:
- LevelDB's SSTable format
- RocksDB's BlockBasedTable
- The Log-Structured Merge-tree paper (O'Neil et al., 1996)
