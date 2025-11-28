# nori-wal

Production-ready Write-Ahead Log for Rust with automatic recovery, rotation, and configurable durability.

[Quickstart](getting-started/quickstart.md){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[API Reference](api-reference/index.md){: .btn .fs-5 .mb-4 .mb-md-0 }

---

## What is nori-wal?

**nori-wal** is a production-ready write-ahead log (WAL) implementation in Rust. It provides the foundation for durable, crash-safe storage systems.

### Key Features

- **Append-only writes** with sequential I/O (~110K writes/sec)
- **Automatic crash recovery** with prefix-valid truncation
- **CRC32C checksumming** for corruption detection
- **Configurable fsync policies** (every write, batch, async)
- **Automatic rotation** with size/time triggers
- **LZ4/Zstd compression** support
- **Zero-copy reads** with mmap support

---

## Quick Example

```rust
use nori_wal::{Wal, WalConfig, Record};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open WAL with automatic recovery
    let (wal, recovery_info) = Wal::open(WalConfig::default()).await?;

    // Append a record
    let record = Record::put(b"user:42", b"alice@example.com");
    let lsn = wal.append(&record).await?;

    // Iterate over all records
    let mut iter = wal.iter();
    while let Some(entry) = iter.try_next().await? {
        println!("LSN {}: {:?}", entry.lsn, entry.record);
    }

    Ok(())
}
```

---

## Documentation

### Getting Started
Quick tutorials to get up and running with nori-wal.

[Getting Started →](getting-started/index.md)

### Core Concepts
Learn the fundamentals of WALs and how nori-wal implements them.

[Core Concepts →](core-concepts/index.md)

### API Reference
Complete API documentation for all public types and methods.

[API Reference →](api-reference/index.md)

### How It Works
Deep dives into internals, record format, and recovery.

[How It Works →](how-it-works/index.md)

### Performance
Benchmarks and optimization guides.

[Performance →](performance/index.md)

### Recipes
Common patterns and use cases.

[Recipes →](recipes/index.md)

---

## When to Use nori-wal

### Great Fit

- Building **storage engines** (LSM, B-trees)
- **Event sourcing** systems
- **Message queues** and streaming
- Any system requiring **crash consistency**
- **Replication** logs (Raft, Paxos)

### Not the Right Tool

- In-memory only systems (no durability needed)
- Read-heavy workloads (use indexes)
- Random writes (WAL is append-only)

---

## Project Status

**Production-ready** - Used in nori-lsm and norikv-server.

-  108+ tests passing
-  Property testing with deterministic chaos
-  Crash recovery validated
-  Performance benchmarked

---

## Next Steps

**New to WALs?**
Start with [What is a Write-Ahead Log?](core-concepts/what-is-wal.md) to understand the fundamentals.

**Ready to build?**
Jump into the [Quickstart](getting-started/quickstart.md) to get hands-on.

**Want details?**
Check out [How It Works](how-it-works/index.md) for implementation details.
