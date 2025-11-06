---
layout: default
title: Core Concepts
parent: nori-lsm
nav_order: 1
has_children: true
---

# Core Concepts
{: .no_toc }

Foundational knowledge for understanding ATLL (Adaptive Tiered-Leveled LSM) and log-structured storage.
{: .fs-6 .fw-300 }

---

## Overview

This section covers the theoretical foundations and core architectural concepts behind nori-lsm's ATLL implementation. Read these documents in order to build a complete mental model of how LSM trees work and why ATLL's adaptive approach is effective.

---

## Learning Path

### 1. [What is an LSM Tree?](what-is-lsm.md)

**Start here** if you're new to LSM trees.

Learn about:
- Log-structured storage fundamentals
- Historical evolution (1996 LSM paper → modern variants)
- Mathematical foundations (write/read/space amplification)
- RUM conjecture (trade-off analysis)
- LSM vs B-tree comparison

**Key takeaway**: LSM trees trade read performance for write throughput by deferring merge work to background compaction.

### 2. [LSM Compaction Variants](lsm-variants.md)

**Understand the landscape** of compaction strategies.

Learn about:
- Leveled Compaction (LCS) - RocksDB's default
- Size-Tiered Compaction (STCS) - Cassandra's default
- Universal Compaction - RocksDB's hybrid
- Mathematical comparison (WA, RA, SA formulas)
- When to use each strategy

**Key takeaway**: Traditional LSM strategies are global (one strategy for entire database), forcing suboptimal trade-offs for heterogeneous workloads.

### 3. [ATLL Architecture](atll-architecture.md)

**The flagship document** - ATLL's innovation.

Learn about:
- Guard-based range partitioning (slots)
- Dynamic K-way fanout per slot (adaptive tiering)
- EWMA heat tracking (access pattern detection)
- Bandit-based compaction scheduler (reinforcement learning)
- RUM optimization per slot (Pareto-efficient trade-offs)

**Key takeaway**: ATLL adapts compaction strategy per key range, achieving near-optimal performance for Zipfian/skewed workloads.

### 4. [Write Path](write-path.md)

**How data gets into the system.**

Learn about:
- WAL append (durability guarantee)
- Memtable insert (in-memory buffer)
- Memtable flush (L0 SSTable creation)
- L0 compaction (merge to slots)
- Backpressure and flow control

**Key takeaway**: Writes are fast (1-2ms) due to sequential WAL + memtable buffering, with compaction happening asynchronously in the background.

### 5. [Read Path](read-path.md)

**How queries retrieve data.**

Learn about:
- Memtable → L0 → Slot traversal
- Bloom filter optimization (460x faster negative lookups)
- Block cache (200x faster cache hits)
- Read amplification analysis
- Point queries vs range scans

**Key takeaway**: Reads check newest-to-oldest, with bloom filters preventing 99% of unnecessary disk I/O.

### 6. [When to Use ATLL](when-to-use.md)

**Decision guidance** for choosing storage engines.

Learn about:
- Ideal workloads (Zipfian, time-series, multi-tenant)
- Edge cases where ATLL struggles
- Comparison to alternatives (B-tree, pure leveled, pure tiered)
- Migration checklist
- Real-world examples

**Key takeaway**: ATLL excels for heterogeneous workloads with hot/cold data; avoid for uniform access or pure scans.

---

## Quick Reference

### Performance Characteristics

| Metric | ATLL | Pure Leveled | Pure Tiered | B-tree |
|--------|------|--------------|-------------|--------|
| **Write Amplification** | 8-20x | 40-100x | 6-8x | 2-10x |
| **Read Amplification** | 5-12 | 5-10 | 10-15 | 1 |
| **Space Amplification** | 1.1-1.3x | 1.1x | 1.33x | 1.1-2.0x |
| **Read Latency (p95)** | <10ms (hot) | <10ms | 20-50ms | <5ms* |
| **Write Throughput** | High | Low | Very High | Medium |
| **Adaptation** | Dynamic | Manual | Manual | N/A |

*B-tree latency assumes data fits in buffer pool

### Key Formulas

**Write Amplification** (Leveled):
```
WA = T × (L - 1)

Where:
  T = fanout (default: 10)
  L = number of levels
```

**Read Amplification** (ATLL):
```
RA = L0_files + k_max

Where:
  L0_files = 6 (typical)
  k_max = 1 (hot slot) or 4 (cold slot)
```

**ATLL K-Max Formula**:
```
k_max = 1 + floor((1 - heat_score) × (K_global - 1))

Where:
  heat_score ∈ [0, 1]  (0 = cold, 1 = hot)
  K_global = 4 (default)
```

---

## Visual Overview

### ATLL Architecture Diagram

```
┌────────────────────────────────────────────────────────┐
│  L0: Overlapping files (global)                        │
│  ┌──┐ ┌──┐ ┌──┐ ┌──┐ ┌──┐ ┌──┐                        │
│  │  │ │  │ │  │ │  │ │  │ │  │                        │
│  └──┘ └──┘ └──┘ └──┘ └──┘ └──┘                        │
└────────────────────────────────────────────────────────┘

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

Legend:
  K = max sorted runs per slot
  RA = read amplification (L0 + K)
  WA = write amplification (depends on compaction frequency)
```

### Write Path Flow

```
put(key, value)
  ↓
┌──────────────────┐
│ 1. WAL Append    │  ← Durability (fsync, 1-2ms)
│    (sequential)  │
└──────────────────┘
  ↓
┌──────────────────┐
│ 2. Memtable      │  ← In-memory (skiplist, 50ns)
│    Insert        │
└──────────────────┘
  ↓
[Background Tasks]
  ↓
┌──────────────────┐
│ 3. Memtable      │  ← Async (50-200ms)
│    Flush → L0    │
└──────────────────┘
  ↓
┌──────────────────┐
│ 4. L0 → Slot     │  ← Async (200-500ms)
│    Compaction    │
└──────────────────┘
  ↓
┌──────────────────┐
│ 5. Slot-Local    │  ← Async (500-2000ms)
│    Tiering       │     Adaptive (hot slots more frequent)
└──────────────────┘
```

### Read Path Flow

```
get(key)
  ↓
┌──────────────────┐
│ 1. Check         │  ← In-memory (50ns)
│    Memtable      │
└──────────────────┘
  ↓ (miss)
┌──────────────────┐
│ 2. L0 Bloom      │  ← In-memory (400ns)
│    Filters       │     99% skip disk reads
└──────────────────┘
  ↓ (maybe)
┌──────────────────┐
│ 3. Find Slot     │  ← Binary search (10ns)
│    (guard keys)  │
└──────────────────┘
  ↓
┌──────────────────┐
│ 4. Slot Bloom    │  ← In-memory (67-268ns)
│    Filters       │     Check K runs
└──────────────────┘
  ↓ (maybe)
┌──────────────────┐
│ 5. Block Cache   │  ← Cache hit: 500ns
│    or Disk       │     Cache miss: 100µs
└──────────────────┘
```

---

## Common Questions

### Why not just use RocksDB?

RocksDB's leveled compaction is excellent for uniform workloads, but:
- High write amplification (40-100x) wastes I/O on cold data
- Manual tuning required (bloom bits, compaction threads, L0 thresholds)
- No adaptation to changing access patterns

ATLL provides:
- 2-5x lower write amplification via adaptive per-slot strategy
- Automatic tuning via bandit scheduler
- Online adaptation to workload shifts

### How does ATLL compare to ScyllaDB ICS?

ScyllaDB's Incremental Compaction Strategy (ICS) uses time-window bucketing:
- Assumes time-series workload (recent = hot, old = cold)
- Requires manual time-window configuration
- Not general-purpose

ATLL:
- Access-pattern-based (not time-based, works for any workload)
- Automatic adaptation via EWMA heat tracking
- General-purpose key-value store

### Does ATLL support transactions?

No. ATLL provides:
- **Single-key atomicity**: `put()`/`delete()` are atomic per key
- **Durability**: WAL fsync before returning
- **Isolation**: No read-your-writes guarantees across keys
- **Consistency**: Last-write-wins semantics

For full ACID transactions, use:
- PostgreSQL (SQL, relational)
- CockroachDB (distributed SQL)
- TiKV (distributed key-value)

### Can I tune ATLL parameters?

Yes, but typically not needed. Tunable parameters:
- `num_slots`: 16 (default), increase for larger datasets
- `k_global`: 4 (default), increase for write-heavy workloads
- `heat_alpha`: 0.1 (default), increase for rapidly changing patterns
- `compaction_epsilon`: 0.1 (default), increase for non-stationary workloads

See [Performance Tuning](../performance/tuning.md) for detailed guidance.

---

## Next Steps

After completing Core Concepts:
1. **[Design Decisions](../design-decisions/)** - Dive deeper into ATLL's design rationale
2. **[How It Works](../how-it-works/)** - Implementation details and algorithms
3. **[Performance](../performance/)** - Benchmarking, tuning, and optimization
4. **[Recipes](../recipes/)** - Common use cases and integration patterns

---

*Last Updated: 2025-10-31*
*Total Reading Time: ~2 hours*
