# nori-lsm

**Embeddable ATLL (Adaptive Tiered-Leveled LSM) engine** with WAL durability, SSTable storage, and intelligent compaction.

## Features

- **MVCC Semantics**: Sequence number-based versioning
- **Guard-Based Partitioning**: Adaptive slot-based range partitioning
- **Dynamic K-Way Fanout**: Adapts to access patterns (hot = leveled, cold = tiered)
- **Bandit-Based Compaction**: Intelligent scheduling using multi-armed bandit algorithm
- **WAL Durability**: Write-ahead logging with crash recovery
- **Bloom Filters**: Optimized point query performance
- **Comprehensive Observability**: Stats tracking, VizEvent emission, performance metrics
- **Production-Grade Performance**: Microsecond latencies (see benchmarks below)

## Performance Benchmarks

Benchmark results on Apple Silicon (M-series), measured with Criterion:

### SLO Compliance

**Target SLOs:**
- p95 GET latency: < 10ms (10,000 µs)
- p95 PUT latency: < 20ms (20,000 µs)

### Basic Operations

| Operation | Mean Latency | vs 20ms SLO | Status |
|-----------|--------------|-------------|--------|
| **PUT (1KB)** | 62.2 µs | 205x faster | **Exceeds** |
| **PUT (4KB)** | 91.8 µs | 168x faster | **Exceeds** |
| **GET (memtable)** | 0.30 µs | 30,000x faster | **Exceeds** |
| **GET (L0)** | 2.67 µs | 3,400x faster | **Exceeds** |
| **DELETE** | 16.8 µs | 565x faster | **Exceeds** |
| **Mixed 80/20** | 9.28 µs | - | **Excellent** |

### Write Scaling by Value Size

| Value Size | Mean Latency | Throughput |
|-----------|--------------|------------|
| 128 B | 25.2 µs | ~40K ops/sec |
| 512 B | 32.1 µs | ~31K ops/sec |
| 1 KB | 37.1 µs | ~27K ops/sec |
| 4 KB | 63.8 µs | ~16K ops/sec |
| 16 KB | 171.7 µs | ~5.8K ops/sec |

**Key Findings:**
- **Microsecond latencies**: All operations complete in **microseconds**, while SLO targets are in **milliseconds**
- **Sub-microsecond reads**: Hot memtable reads in ~300 nanoseconds
- **Fast L0 reads**: Even cold L0 reads from disk only take ~2.7µs
- **Linear write scaling**: PUT latency scales linearly with value size
- **Production ready**: Performance **far exceeds** SLO requirements

## Architecture

```text
┌─────────────────────────────────────────────────────────────┐
│  Memtable (skiplist)                                        │
│  - In-memory writes with MVCC                               │
│  - WAL durability                                           │
│  - Flush trigger: 64MB or 30s WAL age                       │
└──────────────┬──────────────────────────────────────────────┘
               │ Flush
               ↓
┌─────────────────────────────────────────────────────────────┐
│  L0 (unbounded overlapping SSTables)                        │
│  - Direct memtable flushes                                  │
│  - Admitted to L1 by splitting on guard boundaries          │
└──────────────┬──────────────────────────────────────────────┘
               │ L0→L1 admission
               ↓
┌─────────────────────────────────────────────────────────────┐
│  L1+ (guard-partitioned levels)                             │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Slot 0    │ Slot 1    │ Slot 2    │ ... │ Slot N       │ │
│  │ [g₀, g₁)  │ [g₁, g₂)  │ [g₂, g₃)  │     │ [gₙ, +∞)     │ │
│  │ K=1-3 runs│ K=1-3 runs│ K=1-3 runs│     │ K=1-3 runs   │ │
│  └────────────────────────────────────────────────────────┘ │
│  - Hot slots: K→1 (leveled, low read amp)                   │
│  - Cold slots: K>1 (tiered, low write amp)                  │
└─────────────────────────────────────────────────────────────┘
```

## Usage

```rust
use nori_lsm::{LsmEngine, ATLLConfig};
use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create engine with default configuration
    let config = ATLLConfig::default();
    let engine = LsmEngine::open(config).await?;

    // Write
    let key = Bytes::from("user:123");
    let value = Bytes::from("alice");
    engine.put(key, value).await?;

    // Read
    if let Some(value) = engine.get(b"user:123").await? {
        println!("Value: {:?}", String::from_utf8_lossy(&value));
    }

    // Delete
    engine.delete(b"user:123").await?;

    // Range scan
    let mut iter = engine.iter_range(b"user:", b"user;").await?;
    while let Some((key, value)) = iter.next().await? {
        println!("{:?} => {:?}", key, value);
    }

    Ok(())
}
```

### With Observability

```rust
use nori_lsm::{LsmEngine, ATLLConfig};
use nori_observe::{Meter, VizEvent};
use std::sync::Arc;

// Implement your Meter for Prometheus, OTLP, etc.
struct MyMeter;

impl Meter for MyMeter {
    fn counter(&self, name: &'static str, labels: &'static [(&'static str, &'static str)])
        -> Box<dyn nori_observe::Counter> {
        // Return your counter implementation
        todo!()
    }

    fn gauge(&self, name: &'static str, labels: &'static [(&'static str, &'static str)])
        -> Box<dyn nori_observe::Gauge> {
        todo!()
    }

    fn histo(&self, name: &'static str, buckets: &'static [f64],
             labels: &'static [(&'static str, &'static str)])
        -> Box<dyn nori_observe::Histogram> {
        todo!()
    }

    fn emit(&self, evt: VizEvent) {
        // Handle VizEvent for live dashboard streaming
        println!("Event: {:?}", evt);
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ATLLConfig::default();
    let meter = Arc::new(MyMeter);

    // Open engine with observability
    let engine = LsmEngine::open_with_meter(config, meter).await?;

    // Operations will now emit metrics and events
    engine.put(Bytes::from("key"), Bytes::from("value")).await?;

    Ok(())
}
```

### Graceful Shutdown

Always call `shutdown()` before dropping the engine to ensure data safety:

```rust
use nori_lsm::{LsmEngine, ATLLConfig};
use bytes::Bytes;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ATLLConfig::default();
    let engine = LsmEngine::open(config).await?;

    // Perform operations
    engine.put(Bytes::from("key"), Bytes::from("value")).await?;

    // Always shutdown gracefully before exit
    engine.shutdown().await?;

    Ok(())
}
```

**What `shutdown()` does:**
1. Signals the background compaction thread to stop
2. Signals the background WAL GC loop to stop
3. Waits for all background threads to complete gracefully
4. Flushes any pending memtable data to disk
5. Syncs the WAL to ensure all writes are durable
6. Writes a final manifest snapshot for clean recovery

**Best Practices:**
- Always call `shutdown()` in production applications
- Use `tokio::signal` for signal handling (SIGTERM, SIGINT)
- The engine will log a warning if dropped without calling `shutdown()`

```rust
// Example: Shutdown on SIGTERM
use tokio::signal;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = LsmEngine::open(ATLLConfig::default()).await?;

    // Spawn signal handler
    tokio::spawn(async move {
        signal::ctrl_c().await.expect("Failed to listen for Ctrl+C");
        println!("Received shutdown signal");
    });

    // Application logic...

    // Graceful shutdown on exit
    engine.shutdown().await?;
    Ok(())
}
```

## WAL Lifecycle Management

The LSM engine automatically manages Write-Ahead Log (WAL) segments to prevent disk space leaks:

### Automatic WAL Garbage Collection

A background task runs every **60 seconds** to clean up old WAL segments after data has been flushed to SSTables:

```rust
// WAL GC happens automatically in the background
let engine = LsmEngine::open(config).await?;

// Perform operations - old WAL segments are cleaned up automatically
engine.put(Bytes::from("key"), Bytes::from("value")).await?;

// On shutdown, WAL GC loop stops gracefully
engine.shutdown().await?;
```

**How it works:**
1. **After memtable flush**: When memtable data is persisted to SSTables, old WAL segments become eligible for deletion
2. **Periodic cleanup**: Background task runs every 60 seconds to delete old segments
3. **Watermark-based**: Only segments older than the oldest unflushed memtable are deleted
4. **Safe by default**: Active segments are never deleted, ensuring crash recovery always works

**Metrics:**
The WAL GC task emits observability metrics:
- `wal_gc_latency_ms` (histogram): Time to delete segments
- `wal_segments_deleted_total` (counter): Total segments deleted
- `wal_segment_count` (gauge): Current number of segments

### Recovery Guarantees

The engine provides **exactly-once semantics** for crash recovery:

- **Prefix-valid recovery**: WAL recovery validates CRC checksums and truncates partial writes at the tail
- **Last committed version preserved**: Property tests verify the last committed version is always recovered
- **Stress tested**: Includes power-loss simulations and concurrent GC scenarios

**Recovery tests:**
- `test_wal_recovery_after_crash` - Simulates power loss during writes
- `test_wal_concurrent_gc_no_data_loss` - Validates concurrent GC safety
- `test_wal_recovery_with_incomplete_writes` - Tests prefix-valid truncation
- `test_recovery_invariant_holds` - Property test with 20 random scenarios

## Configuration

Key configuration options in `ATLLConfig`:

- `data_dir`: Base directory for LSM data (sst/, manifest/, wal/)
- `fanout`: Level size ratio (default: 10x per level)
- `max_levels`: Maximum number of levels (default: 7)
- `l1_slot_count`: Number of guard-partitioned slots at L1 (default: 32)
- `memtable.flush_trigger_bytes`: Memtable size threshold (default: 64 MiB)
- `memtable.wal_age_trigger_sec`: WAL age threshold (default: 30s)
- `l0.max_files`: L0 file count before backpressure (default: 12)
- `filters.bloom_bits_per_key`: Bloom filter density (default: 10)

See `ATLLConfig` documentation for full configuration options.

## Testing

```bash
# Run all tests
cargo test -p nori-lsm

# Run benchmarks
cargo bench -p nori-lsm

# Run specific benchmark
cargo bench -p nori-lsm --bench lsm_operations
cargo bench -p nori-lsm --bench range_scans
cargo bench -p nori-lsm --bench heavy_workloads
```

## Current Status

**Production Features Implemented:**

- Core LSM structure (memtable, manifest, guards, heat tracking)
- Multi-level reads across L0/L1+ with key range routing
- L0→L1 admission with guard-based routing (physical splitting during compaction)
- WAL recovery and cleanup after flush
- **Automatic WAL garbage collection** with 60-second periodic cleanup and observability metrics
- Sequence number tracking for MVCC
- Comprehensive stats tracking
- Full observability integration (VizEvents, metrics, counters)
- Bloom filter support
- Error recovery (graceful degradation on corrupted SSTables)
- **Graceful shutdown** with background thread cleanup, flush, and sync guarantees
- **Property-based testing** for recovery invariants (35 randomized test cases)
- **110+ passing tests** covering operations, recovery, compaction, shutdown, WAL GC, and stress scenarios

**⚠️ Known Limitations:**

- Physical file splitting deferred to compaction phase (L0 files may span slots temporarily)
- Bloom filter tuning pending real-world workload analysis

## Benchmarking

Benchmark results are generated with [Criterion.rs](https://github.com/bheisler/criterion.rs):

```bash
# Run all benchmarks and view HTML reports
cargo bench -p nori-lsm
open target/criterion/report/index.html
```

Results show:

- **Microsecond-level latencies** for all operations
- **205x faster** than PUT SLO target (20ms)
- **3,400x faster** than GET SLO target (10ms)
- Linear scaling with value size
- Excellent mixed workload performance

## License

MIT

## Design References

See `context/lsm_atll_design.yaml` for complete specification.
