# nori-lsm

Embeddable LSM storage engine with ATLL (Adaptive Tiered-Leveled) compaction for heterogeneous workloads.

[Core Concepts](core-concepts/index.md){: .btn .btn-primary .fs-5 .mb-4 .mb-md-0 .mr-2 }
[Design Decisions](design-decisions/index.md){: .btn .fs-5 .mb-4 .mb-md-0 }

---

## What is nori-lsm?

**nori-lsm** is a production-ready Log-Structured Merge (LSM) storage engine that implements ATLL (Adaptive Tiered-Leveled) compaction—a novel hybrid strategy that adapts between tiered and leveled compaction per key range based on access patterns.

### Key Innovation: ATLL

Traditional LSMs force a global choice:
- **Leveled** (RocksDB): Fast reads, slow writes (40-100x write amplification)
- **Tiered** (Cassandra): Fast writes, slow reads (10-15 read amplification)

**ATLL adapts per key range**:
- Hot ranges → Leveled (K=1, fast reads)
- Cold ranges → Tiered (K>1, fast writes)
- Result: 8-20x WA, 5-12 RA (near-Pareto-optimal for Zipfian workloads)

---

## Key Features

- **Adaptive Compaction**: ATLL automatically optimizes per key range
- **Guard-Based Partitioning**: Range-partitioned slots with fixed boundaries
- **EWMA Heat Tracking**: Online access pattern detection with exponential decay
- **Bandit Scheduler**: Reinforcement learning for compaction decisions (epsilon-greedy UCB)
- **Bloom Filters**: 10 bits/key (0.9% FP rate, 460x faster negative lookups)
- **WAL Integration**: Built on nori-wal for durability and recovery
- **Memory Pressure System**: 4-zone adaptive backpressure (green/yellow/orange/red)
- **Snapshot Support**: Point-in-time consistent snapshots

---

## Quick Example

```rust
use nori_lsm::{LsmEngine, ATLLConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Open LSM engine with ATLL
    let config = ATLLConfig::default();
    let engine = LsmEngine::open(config).await?;

    // Write data
    engine.put(b"user:123", b"alice@example.com").await?;
    engine.put(b"user:456", b"bob@example.com").await?;

    // Read data
    if let Some(value) = engine.get(b"user:123").await? {
        println!("Value: {:?}", value);
    }

    // Range scan
    let results = engine.scan(b"user:", b"user:~").await?;
    for (key, value) in results {
        println!("{:?} → {:?}", key, value);
    }

    // Snapshot
    let snapshot = engine.snapshot().await?;
    let snapshot_value = snapshot.get(b"user:123").await?;

    Ok(())
}
```

---

## Performance Characteristics

| Metric | ATLL | Pure Leveled | Pure Tiered |
|--------|------|--------------|-------------|
| **Write Amplification** | 8-20x | 40-100x | 6-8x |
| **Read Amplification** | 5-12 (adaptive) | 5-10 | 10-15 |
| **Space Amplification** | 1.1-1.3x | 1.1x | 1.33x |
| **Read Latency (p95)** | <10ms (hot) | <10ms | 20-50ms |
| **Write Throughput** | High | Low | Very High |

**Benchmark highlights** (Apple M2 Pro):
- Point reads (memtable hit): <1µs
- Point reads (cache hit): ~1µs
- Point reads (cache miss): ~110µs
- Range scans (100 keys): <5ms
- Write latency: 1-2ms (WAL fsync)

---

## Documentation

### Core Concepts
Learn the fundamentals of LSM trees and ATLL's innovations.

[Core Concepts →](core-concepts/index.md)

**Start here** if you're new to LSM trees. Topics include:
- What is an LSM Tree? (history, math, RUM conjecture)
- LSM Compaction Variants (leveled, tiered, universal)
- ATLL Architecture (guard keys, K-way fanout, heat tracking, bandit scheduler)
- Write Path (WAL → memtable → L0 → slots)
- Read Path (memtable → L0 → slot with bloom filters)
- When to Use ATLL (decision tree, migration guides)

### Design Decisions
Deep dives into ATLL's design rationale and trade-offs.

[Design Decisions →](design-decisions/index.md)

Topics include:
- Guard-Based Partitioning (why fixed boundaries?)
- Bandit Scheduler (epsilon-greedy UCB, reward function)
- Amplification Trade-offs (RUM optimization per slot)
- Dynamic K-Fanout (heat → K mapping formula)
- Heat Tracking (EWMA convergence analysis)

### How It Works
Implementation details, algorithms, and internals.

[How It Works →](how-it-works/index.md)

Topics include:
- L0 Admission Control (backpressure, soft throttling)
- Slot-Local Tiering (size-tiered merging within slots)
- Manifest Format (slot metadata, guard keys)
- Compaction Triggering (bandit selection, UCB scoring)
- Bloom Filter Implementation (xxHash64, double hashing)

### Performance
Benchmarks, optimization techniques, and tuning guides.

Topics include:
- Write Amplification Analysis (per-slot WA, weighted average)
- Read Amplification Analysis (bloom filter impact, cache hit rates)
- Tuning Guide (num_slots, k_global, heat_alpha, epsilon)
- Benchmark Results (Zipfian workloads, sustained writes, p95 latency)

### Recipes
Common usage patterns and integration examples.

Topics include:
- Time-Series Data (recent-heavy reads, TTL integration)
- Hot-Cold Separation (multi-tenant systems)
- Basic Key-Value Store (session store, cache backend)
- Migration Patterns (from RocksDB, Cassandra, B-trees)

---

## When to Use nori-lsm

### Great Fit

-  **Skewed access patterns** (80/20 rule, Zipfian distribution)
-  **Mixed workloads** (40-60% reads/writes)
-  **Time-series with recent-heavy reads** (metrics, logs)
-  **Multi-tenant systems** (active + dormant tenants)
-  **Large datasets** (>100 GB)
-  **SSD wear concerns** (lower WA than pure leveled)
-  **Need automatic adaptation** (no manual tuning)

### Not the Right Tool

-  **Uniform access** (all keys equally hot → use pure leveled)
-  **Pure scans** (no point queries → use columnar storage)
-  **Tiny datasets** (<100 MB → use in-memory hash table)
-  **Need transactions** (use SQL database with ACID)
-  **Append-only writes** (use log, not LSM)

---

## Architecture Overview

### ATLL Structure

```
┌────────────────────────────────────────────────────────┐
│  L0: Overlapping files (global)                        │
│  ┌──┐ ┌──┐ ┌──┐ ┌──┐ ┌──┐ ┌──┐                        │
│  │  │ │  │ │  │ │  │ │  │ │  │                        │
│  └──┘ └──┘ └──┘ └──┘ └──┘ └──┘                        │
└────────────────────────────────────────────────────────┘
           ↓ L0 Compaction
┌────────────────────────────────────────────────────────┐
│  L1+: Range-partitioned slots (adaptive K-way fanout) │
│                                                        │
│  ┌─────────┬─────────┬─────────┬─────────┬─────────┐  │
│  │ Slot 0  │ Slot 1  │ Slot 2  │ Slot 3  │ Slot 4  │  │
│  │ (HOT)   │ (COLD)  │ (COLD)  │ (HOT)   │ (COLD)  │  │
│  │ K=1     │ K=3     │ K=2     │ K=1     │ K=4     │  │
│  │ [a..d)  │ [d..g)  │ [g..m)  │ [m..t)  │ [t..z)  │  │
│  │         │         │         │         │         │  │
│  │ RA=7    │ RA=9    │ RA=8    │ RA=7    │ RA=10   │  │
│  │ WA=20x  │ WA=8x   │ WA=12x  │ WA=20x  │ WA=6x   │  │
│  └─────────┴─────────┴─────────┴─────────┴─────────┘  │
└────────────────────────────────────────────────────────┘
```

### Write Path Flow

```
put(key, value)
  ↓
1. WAL Append (1-2ms, fsync)
  ↓
2. Memtable Insert (50ns, skiplist)
  ↓
[Background Tasks]
  ↓
3. Memtable Flush → L0 (50-200ms)
  ↓
4. L0 → Slot Compaction (200-500ms)
  ↓
5. Slot-Local Tiering (500-2000ms, adaptive frequency)
```

### Read Path Flow

```
get(key)
  ↓
1. Check Memtable (50ns)
  ↓ (miss)
2. L0 Bloom Filters (400ns, 6 files)
  ↓ (maybe)
3. Find Slot (10ns binary search)
  ↓
4. Slot Bloom Filters (67-268ns, K runs)
  ↓ (maybe)
5. Block Cache or Disk (500ns cached, 100µs disk)
```

---

## Dependencies

nori-lsm is built on:
- **[nori-wal](../nori-wal/index.md)** - Write-ahead log for durability
- **[nori-sstable](../nori-sstable/index.md)** - Immutable sorted tables
- **[nori-observe](../nori-observe/index.md)** - Vendor-neutral observability

All dependencies are production-ready.

---

## Configuration Example

```rust
use nori_lsm::{ATLLConfig, L0Config, ResourceConfig};

let config = ATLLConfig {
    // L0 configuration
    l0: L0Config {
        max_files: 12,                        // Hard stall threshold
        soft_throttle_threshold: 6,           // 50% of max_files
        soft_throttle_base_delay_ms: 1,
    },

    // Slot configuration
    num_slots: 16,                            // Range partitions
    k_global: 4,                              // Max runs per cold slot

    // Heat tracking
    heat_alpha: 0.1,                          // EWMA smoothing
    heat_decay_interval_secs: 60,

    // Bandit scheduler
    compaction_epsilon: 0.1,                  // 10% exploration
    compaction_ucb_c: 2.0,                    // UCB exploration constant

    // Resources
    resources: ResourceConfig {
        block_cache_mib: 1024,                // 1 GB block cache
        index_cache_mib: 128,                 // 128 MB index cache
        memtables_mib: 512,                   // 512 MB memtable budget
        filters_mib: 256,                     // 256 MB bloom filters
    },

    ..Default::default()
};

let engine = LsmEngine::open(config).await?;
```

---

## Status

nori-lsm is **production-ready** (as of 2025-10-31).

**Completed features:**
-  ATLL compaction with guard-based partitioning
-  EWMA heat tracking and dynamic K-way fanout
-  Bandit-based compaction scheduler (epsilon-greedy UCB)
-  Bloom filters (10 bits/key, xxHash64, double hashing)
-  Memory pressure system with 4-zone backpressure
-  WAL integration for durability
-  Snapshot support
-  Range scans and iterators
-  Comprehensive test suite (108 tests passing)
-  Benchmarks (Zipfian workloads, sustained writes)

**Planned features:**
- 🚧 Dynamic guard key adjustment (adaptive rebalancing)
- 🚧 Multi-dimensional heat tracking (read/write/scan heat)
- 🚧 Contextual bandit scheduler (system state as context)
- 🚧 Learned guard keys (ML-based key space partitioning)

---

## Real-World Examples

### E-Commerce Order Database

```rust
// 10M orders, 100 GB data
// Hot: Recent orders (last 30 days, 20% data, 80% reads)
// Cold: Historical orders (>1 year, 80% data, 20% reads)

let config = ATLLConfig {
    num_slots: 32,      // More slots for large dataset
    k_global: 4,        // Allow cold slots to tier
    ..Default::default()
};

let engine = LsmEngine::open(config).await?;

// Recent orders → k_max=1 (leveled, fast reads)
// Historical orders → k_max=4 (tiered, low WA)

// Result:
// - p95 latency: <10ms (hot orders)
// - Write throughput: 10K orders/day sustained
// - Space amplification: 1.2x
```

### IoT Sensor Metrics

```rust
// 1000 sensors × 1 metric/sec = 86M metrics/day
// Hot: Last 24 hours (dashboards, alerts)
// Cold: Last 30 days (historical charts)

let config = ATLLConfig {
    num_slots: 64,      // Fine-grained time ranges
    k_global: 8,        // Higher K for write-heavy
    compaction_epsilon: 0.15,  // More exploration (shifting patterns)
    ..Default::default()
};

let engine = LsmEngine::open(config).await?;

// Recent metrics → k_max=1 (fast dashboard queries)
// Old metrics → k_max=8 (low compaction overhead)

// Result:
// - Write throughput: 1K writes/sec sustained
// - Query latency: <5ms (last 24h), <50ms (last 30d)
```

---

## Next Steps

**New to LSM trees?**
Start with [What is an LSM Tree?](core-concepts/what-is-lsm.md) to build foundational knowledge.

**Understand ATLL's innovation?**
Read [ATLL Architecture](core-concepts/atll-architecture.md) for the full design.

**Ready to use nori-lsm?**
Check out the Recipes section above for common patterns and integration examples.

**Migrating from another LSM?**
See [When to Use ATLL](core-concepts/when-to-use.md) for migration checklists.

---

*Last Updated: 2025-10-31*
*License: MIT*
