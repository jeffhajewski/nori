---
layout: default
title: When to Use ATLL
parent: Core Concepts
grand_parent: nori-lsm
nav_order: 6
---

# When to Use ATLL
{: .no_toc }

Guidance on when ATLL (Adaptive Tiered-Leveled LSM) is the right choice, when to use alternatives, and how to decide between storage engines.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Quick Decision Tree

```
Do you need transactional guarantees (ACID)?
├─ Yes → Use PostgreSQL, CockroachDB, or TiKV
└─ No  → Continue

Is your data small enough to fit in memory (<100 GB)?
├─ Yes → Use Redis, Memcached, or in-memory B-tree
└─ No  → Continue

Is your workload write-heavy (>80% writes)?
├─ Yes → Continue to "Write-Heavy Workloads"
└─ No  → Continue to "Read-Heavy or Mixed Workloads"

Write-Heavy Workloads:
  Are writes sequential (append-only)?
    ├─ Yes → Use append-only log (e.g., Kafka, write-ahead log)
    └─ No  → Use ATLL or pure tiered LSM (e.g., Cassandra)

Read-Heavy or Mixed Workloads:
  Is access pattern uniform (no hot/cold distinction)?
    ├─ Yes → Use B-tree (e.g., RocksDB leveled) or pure leveled LSM
    └─ No  → Use ATLL (adaptive to hot/cold)

Do you need range scans?
├─ Yes → Use ATLL or B-tree (sorted key order)
└─ No  → Consider hash table (e.g., BitCask, WiscKey)
```

---

## ATLL Excels: Heterogeneous Workloads

### 1. Zipfian Access Patterns (80/20 Rule)

**Problem**: Most workloads have hot and cold data
```
E-commerce database:
  Hot (20% of keys, 80% of accesses):
    - Recent orders (last 30 days)
    - Active user sessions
    - Popular product inventory

  Cold (80% of keys, 20% of accesses):
    - Historical orders (>1 year)
    - Archived user data
    - Deleted product history
```

**Why ATLL wins**:
```
Hot ranges → k_max=1 (leveled):
  Read Amplification: 7 (fast)
  Write Amplification: 20x (acceptable)

Cold ranges → k_max=4 (tiered):
  Read Amplification: 10 (acceptable for rare reads)
  Write Amplification: 6x (low)

Result:
  Overall WA: 11.4x (vs 40-100x for pure leveled)
  Overall RA: 7.6 (vs 10-15 for pure tiered)
```

**Alternatives**:
- **Pure Leveled** (RocksDB): 40-100x WA (wasted I/O on cold data)
- **Pure Tiered** (Cassandra): 10-15 RA (slow hot reads)
- **B-tree** (InnoDB): Random writes (fragmentation)

### 2. Time-Series with Recent-Heavy Reads

**Problem**: Recent data queried frequently, old data rarely

**Example**: Metrics database
```
Recent metrics (last 24 hours):
  - High read frequency (dashboards, alerts)
  - High write frequency (continuous ingestion)

Old metrics (>30 days):
  - Low read frequency (historical analysis)
  - No writes (immutable)
```

**Why ATLL wins**:
```
Recent data → k_max=1:
  Fast queries for dashboards (RA=7)
  Accept higher WA for recent data (20x)

Old data → k_max=4:
  Low WA for compaction (6x)
  Slow reads acceptable (RA=10, rare)

Result:
  Efficient storage (SA=1.1-1.3x)
  Fast recent queries (<10ms p95)
```

**Alternatives**:
- **Time-windowed LSM** (ScyllaDB ICS): Requires manual tuning
- **Pure Leveled**: Wasted I/O compacting old data
- **Columnar storage** (Parquet): Good for analytics, bad for point queries

### 3. Multi-Tenant Systems

**Problem**: Some tenants active, others dormant

**Example**: SaaS database
```
Active tenants (10%):
  - Heavy reads + writes
  - SLO requirements (<10ms p95)

Dormant tenants (90%):
  - Rare reads (login once/month)
  - No writes

Trial tenants:
  - Heavy writes (initial data load)
  - Few reads
```

**Why ATLL wins**:
```
Active tenant ranges → k_max=1:
  Meet SLO targets (fast reads)

Dormant tenant ranges → k_max=4:
  Low compaction overhead (WA=6x)

Trial tenant ranges → k_max=3:
  Balance write throughput and read latency

Result:
  Cost-efficient (low I/O for dormant tenants)
  SLO compliant (fast reads for active tenants)
```

**Alternatives**:
- **Separate databases per tenant**: High operational overhead
- **Pure Leveled**: Wasted I/O on dormant tenants
- **Sharded MySQL**: Complex sharding logic

### 4. General-Purpose Key-Value Store

**Problem**: Unpredictable access patterns

**Example**: Session store, cache backend
```
Workload characteristics:
  - Mixed reads/writes (40-60% split)
  - No clear hot/cold pattern
  - Range scans + point queries
  - Variable value sizes (1 KB - 1 MB)
```

**Why ATLL wins**:
```
Adaptive behavior:
  - System learns access patterns via EWMA heat tracking
  - Bandit scheduler optimizes compaction decisions
  - No manual tuning required

Result:
  Near-optimal for any workload (Pareto-efficient)
  Resilient to workload shifts (online learning)
```

**Alternatives**:
- **Redis**: In-memory only (expensive for large data)
- **RocksDB**: Requires manual tuning (bloom bits, compaction style)
- **B-tree**: Random write overhead

---

## ATLL Struggles: Edge Cases

### 1. Uniform Access Patterns

**Problem**: All keys accessed equally often

**Example**: Random UUID key-value store
```
Workload:
  - Uniform key distribution (no hot/cold)
  - Equal read frequency across all keys
  - 50/50 read/write mix
```

**Why ATLL doesn't help**:
```
All slots converge to same k_max:
  - No differentiation between hot/cold
  - Bandit scheduler provides no benefit
  - ATLL overhead (heat tracking, scheduling) wasted

Result:
  ATLL ≈ Pure Leveled (no adaptive advantage)
```

**Better alternative**:
- **Pure Leveled** (RocksDB): Simpler, less overhead
- **B-tree** (if writes are sequential)

### 2. Pure Sequential Scans

**Problem**: Only range scans, no point queries

**Example**: Log analytics, data warehouse ETL
```
Workload:
  - 100% range scans (SELECT * WHERE timestamp > ...)
  - No point queries (get by key)
  - Batch writes (bulk load every hour)
```

**Why ATLL doesn't help**:
```
Bloom filters useless:
  - Range scans must read all blocks in range
  - No benefit from bloom filter skipping

Slot partitioning useless:
  - Scans cross multiple slots
  - No benefit from k_max optimization

Result:
  ATLL overhead (bloom filters, bandit) wasted
```

**Better alternative**:
- **Columnar storage** (Parquet, ORC): Optimized for scans
- **Pure Tiered** (Cassandra STCS): Low WA, scans already slow

### 3. Tiny Datasets (<100 MB)

**Problem**: Entire dataset fits in memory

**Example**: User session cache
```
Dataset:
  - 100K keys × 1 KB = 100 MB
  - Fits in memtable + block cache
  - 99.9% cache hit rate
```

**Why ATLL doesn't help**:
```
All reads hit cache:
  - No disk I/O (no benefit from RA optimization)
  - Compaction overhead still exists (WA penalty)

Result:
  ATLL complexity not justified
```

**Better alternative**:
- **In-memory hash table** (Redis, Memcached): Simpler, faster
- **Embedded B-tree** (SQLite in-memory mode)

### 4. Extremely Skewed Writes

**Problem**: Writes concentrated in one range

**Example**: Monotonically increasing timestamp keys
```
Workload:
  - All writes to newest time range (append-only)
  - Older ranges never written (immutable)
  - Reads scattered across all ranges
```

**Why ATLL doesn't help**:
```
Slot partitioning ineffective:
  - Only one slot receives writes (hot)
  - Other slots idle (no adaptive benefit)

Bandit scheduler wasted:
  - Only one slot to compact (no choice)

Result:
  ATLL overhead without adaptive benefit
```

**Better alternative**:
- **Append-only log** (Kafka, write-ahead log): Optimized for sequential writes
- **Time-windowed LSM** (ScyllaDB ICS): Purpose-built for time-series

---

## ATLL vs Alternatives

### ATLL vs Pure Leveled Compaction (RocksDB)

**Choose Pure Leveled when**:
- Uniform access patterns (no hot/cold distinction)
- Read-heavy workload (>80% reads)
- Small dataset (<10 GB, fits in cache)
- SSD with good random write performance

**Choose ATLL when**:
- Skewed access patterns (Zipfian, 80/20 rule)
- Mixed workload (40-60% reads/writes)
- Large dataset (>100 GB)
- Want low WA without sacrificing read performance

**Comparison**:
```
Metric               Pure Leveled  ATLL
─────────────────────────────────────────
Write Amplification  40-100x       8-20x
Read Amplification   5-10          5-12 (adaptive)
Space Amplification  1.1x          1.1-1.3x
Configuration        Simple        Adaptive (no tuning)
```

### ATLL vs Pure Tiered Compaction (Cassandra STCS)

**Choose Pure Tiered when**:
- Write-heavy workload (>80% writes)
- Large value sizes (>10 KB)
- Range scans common (not point queries)
- Can tolerate slow reads (100ms+ p95)

**Choose ATLL when**:
- Mixed workload (both reads and writes)
- Small-medium value sizes (<10 KB)
- Point queries common
- Need fast reads (<10ms p95)

**Comparison**:
```
Metric               Pure Tiered  ATLL
─────────────────────────────────────────
Write Amplification  6-8x         8-20x
Read Amplification   10-15        5-12 (adaptive)
Space Amplification  1.33x        1.1-1.3x
Read Latency (p95)   20-50ms      <10ms (hot)
```

### ATLL vs B-Tree (InnoDB, SQLite)

**Choose B-tree when**:
- Transactional guarantees required (ACID)
- In-place updates common (not append-only)
- Sequential writes (e.g., auto-increment primary key)
- Need secondary indexes

**Choose ATLL when**:
- Append-heavy workload (insert + delete, rare updates)
- Write throughput critical (>10K writes/sec)
- Large dataset (>100 GB)
- SSD wear concerns (lower write amplification)

**Comparison**:
```
Metric               B-tree        ATLL
─────────────────────────────────────────
Write Amplification  2-10x*        8-20x
Read Amplification   1 (worst)     5-12
Space Amplification  1.1-2.0x      1.1-1.3x
Transactions         Full ACID     None (single-key atomic)
Random Writes        Slow**        Fast

* Depends on page size, fragmentation
** Requires random I/O, page rewrites
```

### ATLL vs Log-Structured Merge-Bush (Monkey)

**Choose Monkey when**:
- Static workload (predictable access patterns)
- Can afford offline tuning phase
- Academic research (explore RUM frontier)

**Choose ATLL when**:
- Dynamic workload (changing access patterns)
- Online adaptation required (no downtime)
- Production system (simplicity, reliability)

**Comparison**:
```
Metric               Monkey        ATLL
─────────────────────────────────────────
Tuning               Offline ML    Online bandit
Adaptation           Static        Dynamic
Complexity           High          Medium
Production-Ready     Research      Yes
```

---

## Migration Checklist

### From RocksDB (Leveled Compaction)

**Reasons to migrate**:
- [ ] Write amplification too high (>40x)
- [ ] Compaction backlog (can't keep up with writes)
- [ ] Skewed access patterns (hot/cold data)
- [ ] SSD wear concerns

**Migration steps**:
1. Measure baseline metrics (WA, RA, p95 latency)
2. Export RocksDB data to SSTable format
3. Import into ATLL with `num_slots=16`, `k_global=4`
4. Monitor heat scores and k_max convergence (1-2 weeks)
5. Compare metrics (expect 2-5x lower WA)

**Expected improvements**:
- Write amplification: 40-100x → 8-20x
- Write throughput: +50-200%
- Read latency (hot data): Same or better

### From Cassandra (Size-Tiered Compaction)

**Reasons to migrate**:
- [ ] Read latency too high (>20ms p95)
- [ ] Point queries common (not just scans)
- [ ] Space amplification too high (>1.5x)
- [ ] Need faster hot data reads

**Migration steps**:
1. Measure baseline metrics (RA, read latency)
2. Export Cassandra SSTables to ATLL format
3. Import into ATLL with `num_slots=32`, `k_global=8`
4. Monitor slot k_max convergence
5. Compare metrics (expect 2x lower RA for hot data)

**Expected improvements**:
- Read amplification (hot): 10-15 → 5-7
- Read latency (hot): 20-50ms → <10ms
- Write amplification: 6-8x → 8-12x (slight increase)

### From B-Tree (MySQL InnoDB, PostgreSQL)

**Reasons to migrate**:
- [ ] Write throughput bottleneck (random I/O)
- [ ] Large dataset (>100 GB, doesn't fit in buffer pool)
- [ ] Append-heavy workload (rare updates)
- [ ] Don't need transactions

**Migration steps**:
1. Assess transaction requirements (ATLL has no ACID)
2. Export B-tree data to key-value pairs
3. Bulk load into ATLL (batch writes)
4. Benchmark write throughput (expect 5-10x improvement)
5. Monitor read latency (may increase 2-5x)

**Expected trade-offs**:
- Write throughput: +500-1000%
- Write amplification: 2-10x → 8-20x (but sequential I/O)
- Read latency: +2-5x (vs fully cached B-tree)
- Transactions: Lost (implement at application layer)

---

## Real-World Examples

### E-Commerce Order Database

**Workload**:
- 10M orders, 100 GB data
- Hot: Recent orders (last 30 days, 20% of data, 80% of reads)
- Cold: Historical orders (>1 year, 80% of data, 20% of reads)
- Writes: 10K orders/day (inserts + status updates)

**Why ATLL**:
```
Recent orders → k_max=1:
  Fast order lookups for checkout, tracking (RA=7)
  Accept higher WA for recent data (20x)

Historical orders → k_max=4:
  Low WA for archival (6x)
  Slow reads acceptable (rare, analytics only)

Result:
  p95 latency: <10ms (hot orders)
  Write throughput: 10K orders/day sustained
  Storage efficiency: 1.2x space amplification
```

### IoT Sensor Metrics

**Workload**:
- 1000 sensors × 1 metric/sec = 86M metrics/day
- Hot: Last 24 hours (dashboards, alerts)
- Cold: Last 30 days (historical charts)
- Scans: Range queries by time

**Why ATLL**:
```
Recent metrics → k_max=1:
  Fast dashboard queries (RA=7)
  Handle write burst (86M/day = 1K/sec sustained)

Old metrics → k_max=4:
  Low compaction overhead (WA=6x)
  Range scans still fast (sequential I/O)

Result:
  Write throughput: 1K writes/sec sustained
  Query latency: <5ms (last 24h), <50ms (last 30d)
  Cost: Low (minimal compaction I/O for old data)
```

### User Session Store

**Workload**:
- 1M active users, 100M total users
- Hot: Active sessions (1M users, 60-min TTL)
- Cold: Dormant users (99M users, rare login)
- Reads: Session validation (every request)

**Why ATLL**:
```
Active users → k_max=1:
  Fast session validation (<1ms p95)
  High read/write frequency

Dormant users → k_max=4:
  Low compaction cost (rare writes)
  Slow reads acceptable (login once/month)

Result:
  p95 latency: <1ms (active), <100ms (dormant)
  Cost: Low (99% of users don't trigger compaction)
```

---

## Summary

**Use ATLL when**:
- ✅ Skewed access patterns (hot/cold data)
- ✅ Mixed workload (40-60% reads/writes)
- ✅ Large dataset (>100 GB)
- ✅ Need adaptive performance (no manual tuning)
- ✅ SSD wear concerns (lower WA than pure leveled)

**Avoid ATLL when**:
- ❌ Uniform access (all keys equally hot)
- ❌ Pure scans (no point queries)
- ❌ Tiny dataset (<100 MB, fits in memory)
- ❌ Need transactions (use SQL database instead)
- ❌ Append-only writes (use log instead)

**Key insight**: ATLL optimizes for **heterogeneous workloads** where different key ranges have different access patterns.

**Next steps**: See [Recipes](../recipes/) for implementation patterns and [Performance](../performance/) for tuning guidance.

---

*Last Updated: 2025-10-31*
*See Also: [ATLL Architecture](atll-architecture.md), [LSM Variants](lsm-variants.md)*
