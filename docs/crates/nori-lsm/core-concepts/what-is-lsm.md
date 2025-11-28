# What is an LSM Tree?

Understanding the Log-Structured Merge Tree: history, theory, and fundamental concepts.

---

## The Problem: Write Amplification in Traditional Databases

Traditional database systems (B-trees, B+ trees) face a fundamental challenge: **in-place updates**.

### B-Tree Write Pattern

```
User writes 1 KB:
1. Find leaf page (multiple random reads)
2. Read entire page (4 KB) into memory
3. Modify 1 KB in the page
4. Write entire page (4 KB) back to disk
5. Update parent pages for splits (more I/O)

Physical I/O: 12-20 KB for 1 KB logical write
Write Amplification: 12-20x
```

**Why is this problematic?**

- **Random I/O dominates latency** (~100µs per seek on SSD, ~10ms on HDD)
- **SSD wear**: Flash cells have limited P/E cycles (~3,000 for TLC, ~10,000 for MLC)
- **Throughput bottleneck**: Random writes are 10-100x slower than sequential writes

**Concrete example:**

```
Workload: 1M writes/sec of 1KB records
B-tree physical writes: 12-20 GB/sec
Sequential LSM writes: 1-2 GB/sec (6-10x less)

SSD lifetime with B-tree: ~1 year
SSD lifetime with LSM: ~6-10 years
```

---

## The Solution: Log-Structured Merge Trees

**Core insight:** Trade read performance for write performance by **appending** instead of updating in-place.

### LSM-Tree: Invented 1996

**Original paper:** "The Log-Structured Merge-Tree (LSM-Tree)" by Patrick O'Neil, Edward Cheng, Dieter Gawlick, and Elizabeth O'Neil (1996)

**Key innovation:** Defer and batch disk updates

```
┌─────────────────────┐
│   Memory (C0)       │  ← Fast in-memory component
│   (Skiplist/Tree)   │     Absorbs all writes
├─────────────────────┤
│   Disk (C1)         │  ← Level 1: Periodic flushes
│   (Sorted runs)     │
├─────────────────────┤
│   Disk (C2)         │  ← Level 2: Merged periodically
│   (Larger runs)     │
├─────────────────────┤
│   ...               │
└─────────────────────┘
```

**Write path:**

1. **Append to write-ahead log** (WAL) - sequential write, ~100 MB/s
2. **Insert into in-memory tree** - RAM operation, ~1 µs
3. **Flush to disk when full** - sequential batch write, amortizes cost
4. **Merge in background** - batch compaction, not on critical path

**Result:** Writes are sequential, batched, and off the critical path.

---

## Historical Evolution

### 1996: Original LSM-Tree Paper

**O'Neil et al.** introduced the two-component model (C0, C1):
- C0: In-memory component (B-tree or AVL tree)
- C1: Disk component (sorted runs)
- Rolling merge process to maintain sortedness

**Limitation:** Two-level design didn't scale to large datasets.

---

### 2006: Google BigTable

**Paper:** "Bigtable: A Distributed Storage System for Structured Data" (Chang et al., OSDI 2006)

**Innovations:**
- Multi-level LSM (L0, L1, ..., Lk)
- Leveled compaction strategy
- SSTable file format (block-based, indexed, with Bloom filters)
- Memtable + immutable memtable design

**Impact:** Demonstrated LSM-trees at massive scale (petabytes of data, thousands of nodes)

---

### 2011: LevelDB

**Author:** Jeff Dean and Sanjay Ghemawat (Google)

**Contribution:** Open-source embeddable LSM implementation

**Features:**
- Leveled Compaction Strategy (LCS)
- Snappy compression
- Single-writer, multiple-reader concurrency
- Block cache for hot data

**Adoption:** Used in Chrome, Riak, Bitcoin Core, and many others

---

### 2012: RocksDB

**Author:** Facebook (Meta)

**Contribution:** Production-hardened LevelDB fork

**Enhancements:**
- Universal Compaction (tiered strategy)
- Column families (multiple LSMs in one DB)
- Merge operators (atomic read-modify-write)
- Transactions and optimistic concurrency control
- Extensive tuning knobs (100+ configuration options)

**Scale:** Handles 100s of TB per node, millions of QPS

---

### 2014-Present: Modern Variants

- **Cassandra** (2010): Size-Tiered Compaction Strategy (STCS)
- **ScyllaDB** (2015): Incremental Compaction Strategy (ICS)
- **WiredTiger** (2014, MongoDB): LSM + B-tree hybrid
- **Pebble** (2019, CockroachDB): RocksDB-inspired Go implementation
- **BadgerDB** (2017): LSM with value log separation

**Trend:** Workload-specific optimizations (time-series, key-value, wide-column)

---

## Fundamental Concepts

### 1. Log-Structured Storage

**Definition:** All writes are appended sequentially to a log, never updated in-place.

```
Time:  T0      T1      T2      T3
       ───────────────────────────────→
Log:   [A:1] [B:2] [A:3] [C:4] [A:5]
          ↑            ↑            ↑
       First      Second       Third
       version    version      version
                              (newest)
```

**Key property:** Older versions remain until compacted away.

**Implication:** Reads must find the **newest version** by scanning multiple log segments.

---

### 2. Merge (Compaction)

**Problem:** Unbounded log growth → need to reclaim space.

**Solution:** Periodically **merge** sorted runs, keeping only the newest version of each key.

```
Before merge:
  Run 1: [A:1, C:3, E:5]
  Run 2: [A:2, B:4, D:6]
  Run 3: [A:3, F:7]

After merge:
  Run: [A:3, B:4, C:3, D:6, E:5, F:7]
       ↑ newest version kept
```

**Merge algorithm:** Multi-way k-way merge (priority queue-based)

**Cost:** Reads all input runs, writes one output run → Write Amplification

---

### 3. Levels and Fanout

**Multi-level organization:** Each level is ~T times larger than the previous level.

```
Level 0:     10 MB   (unsorted, overlapping)
Level 1:    100 MB   (sorted, non-overlapping, T=10)
Level 2:  1,000 MB   (T=10 growth)
Level 3: 10,000 MB   (T=10 growth)
```

**Fanout (T):** Ratio of level sizes, typically 10.

**Formula:** `Level_i_size = Level_(i-1)_size * T`

**Total levels:** `L = ceil(log_T(Total_Data_Size / Memtable_Size))`

---

### 4. Sorted String Tables (SSTables)

**Definition:** Immutable, sorted file containing key-value pairs.

**Structure:**

```
SSTable file:
┌────────────────┐
│  Data Blocks   │ ← Sorted key-value pairs (compressed)
├────────────────┤
│  Index Block   │ ← Block offsets (first key → offset)
├────────────────┤
│  Filter Block  │ ← Bloom filter (is key present?)
├────────────────┤
│  Footer        │ ← Metadata (index/filter offsets)
└────────────────┘
```

**Key properties:**
- **Immutable**: Once written, never modified (enables caching, no locks)
- **Sorted**: Enables binary search and range queries
- **Compressed**: LZ4 or Zstd reduces disk space and I/O
- **Indexed**: Fast lookups via sparse index
- **Filtered**: Bloom filters skip missing keys (67ns vs 100µs disk read)

---

## Mathematical Foundations

### Write Amplification (WA)

**Definition:** Ratio of physical bytes written to logical bytes written.

**Formula for Leveled Compaction:**

```
WA = T * (L - 1)

Where:
  T = fanout (default: 10)
  L = number of levels

Example:
  T = 10, L = 5
  WA = 10 * (5 - 1) = 40

Interpretation: Each 1 KB logical write → 40 KB physical writes
```

**Why?** Each level compaction merges data with the next level:
- L1→L2: Read L1 run + overlapping L2 runs (T times larger) → Write merged output (T+1 data)
- Repeat for L2→L3, L3→L4, ...

**Detailed derivation:**

```
Assume 1 MB flush from memtable:

L0→L1: Write 1 MB to L1                        Cost: 1 MB
L1→L2: Merge 10 MB (L1) + 100 MB (L2) = 110 MB Cost: 110 MB written
L2→L3: Merge 100 MB (L2) + 1 GB (L3) = 1.1 GB  Cost: 1.1 GB written
...

Total physical writes for 1 MB logical: ~1.21 GB
WA ≈ 1,210
```

**Optimization:** Size-tiered compaction reduces WA to `O(log_T(N))` but increases read amplification.

---

### Read Amplification (RA)

**Definition:** Number of disk reads (SSTables accessed) per point query.

**Formula for Leveled Compaction:**

```
RA = L0_files + Σ(runs_per_level)

For pure leveled (1 run per level):
  RA = L0_files + L

Example:
  L0 files: 4
  Levels: 5
  RA = 4 + 5 = 9

Interpretation: Must check 9 SSTables to guarantee key is not present
```

**Best case:** Key found in memtable → RA = 0
**Worst case:** Key not present → RA = L0_files + L

**Bloom filter impact:**

```
Without blooms: 9 disk reads * 100µs = 900µs
With blooms (10 bits/key, 0.9% FP):
  Expected disk reads: 9 * 0.009 ≈ 0.08
  Time: 9 * 67ns (bloom checks) + 0.08 * 100µs ≈ 8µs

Speedup: 112x
```

---

### Space Amplification (SA)

**Definition:** Ratio of physical storage to logical data size.

**Formula:**

```
SA = Physical_Bytes / Logical_Bytes

Sources of space amplification:
1. Obsolete versions (not yet compacted)
2. Tombstones (deleted keys awaiting compaction)
3. Compression overhead (metadata, block alignment)
4. Bloom filters and indexes

Typical SA for LSM: 1.1 - 1.5x
```

**Example:**

```
Logical data: 100 GB
L0 (uncompacted): 5 GB
L1-L5: 100 GB (current versions)
Obsolete versions: 10 GB (being compacted)
Indexes + Blooms: 2 GB

Physical: 117 GB
SA: 1.17x
```

---

## The RUM Conjecture

**Paper:** "Designing Access Methods: The RUM Conjecture" (Athanassoulis et al., EDBT 2016)

**Conjecture:** For any access method, one can optimize at most two out of three:
- **R**ead overhead
- **U**pdate overhead
- **M**emory (space) overhead

**Implication:** LSM-trees make a specific trade-off position:
-  **Low Update overhead** (sequential writes)
-  **Low Memory overhead** (compact storage)
-  **Higher Read overhead** (multiple levels to check)

**Pareto frontier:**

```
         Read Overhead
              ↑
              │     B-tree ●
              │         (low read, high update)
              │
              │
              │             ● LSM
              │          (medium read, low update)
              │
              │  Hash table ●
              │  (lowest read, highest space)
              └──────────────────────→
                     Update Overhead
```

**Key insight:** No single data structure dominates all workloads. LSM is optimal for write-heavy, read-tolerant workloads.

---

## Why LSM Trees Outperform B-Trees

### 1. Sequential vs Random I/O

**SSD Performance Characteristics:**

```
Operation              Latency    Bandwidth
────────────────────────────────────────────
Random 4KB read        100µs      40 MB/s
Sequential 1MB read     10µs     500 MB/s
Random 4KB write       100µs      40 MB/s
Sequential 1MB write    10µs     500 MB/s

Ratio: Sequential is 12.5x faster
```

**LSM write pattern:**

```
1. Append to WAL (sequential)
2. Insert to memtable (RAM)
3. Flush to L0 (sequential, batched)
4. Compact (sequential merge)

All disk writes are sequential!
```

**B-tree write pattern:**

```
1. Read page (random)
2. Modify page (RAM)
3. Write page (random)
4. Update parent (random)

Most disk operations are random!
```

---

### 2. Write Batching

**LSM batches writes:**

```
1,000 individual writes:
  B-tree: 1,000 random I/Os
  LSM:    1,000 RAM inserts → 1 sequential flush

Latency improvement: 100-1,000x
```

**Amortization:** Cost of compaction is spread across many writes.

---

### 3. Compression Efficiency

**LSM advantages:**
- Immutable files → compress entire SSTable at write time
- Sorted data → better compression ratios (delta encoding, prefix compression)
- Batch compression → amortize compression overhead

**B-tree limitations:**
- Mutable pages → compression complicates updates
- Fragmentation → pages not fully utilized (50-70% typical)

**Empirical results:**

```
Dataset: 100 GB logical data

B-tree on disk:       140 GB (1.4x)
LSM (uncompressed):   110 GB (1.1x)
LSM (LZ4):            55 GB (0.55x)
LSM (Zstd):           40 GB (0.4x)

LSM with Zstd uses 3.5x less space than B-tree
```

---

### 4. Read-Ahead Friendly

**Sequential access pattern:**
- OS readahead works perfectly (prefetches next blocks)
- Block cache hit rates higher (sequential locality)

**Random access pattern:**
- Readahead misses (unpredictable next access)
- Cache thrashing (random evictions)

---

## When LSM Trees Excel

###  Write-Heavy Workloads

```
Write:Read ratio > 1:1
Examples:
  - Logging systems (append-only)
  - Time-series databases (metrics, events)
  - Message queues (enqueue → dequeue)
  - Session storage (frequent updates)
```

**Why:** Sequential writes dominate, read latency tolerable.

---

###  Large Datasets (> 1 TB)

```
Memory: 64 GB
Dataset: 10 TB

B-tree working set doesn't fit in RAM:
  Cache hit rate: 0.6%
  Most reads hit disk: 100µs * 0.994 = 99.4µs avg

LSM with Bloom filters:
  Bloom checks: 67ns (always in RAM)
  Disk reads (after bloom): 100µs * 0.009 (FP rate) = 0.9µs avg
  Total: 67ns + 0.9µs = 0.967µs

LSM is 100x faster for point queries on large datasets!
```

---

###  SSD-Optimized Systems

```
SSD wear leveling:
  B-tree: Random writes → wear amplification → shorter lifespan
  LSM: Sequential writes → even wear → longer lifespan

Cost savings:
  B-tree SSD replacement: Every 1 year ($1,000/year)
  LSM SSD replacement: Every 6 years ($167/year)

6x longer SSD life = 6x lower TCO
```

---

###  High Compression Requirements

```
Compressed size:
  B-tree (mutable): 1.4x logical size
  LSM (LZ4): 0.55x logical size
  LSM (Zstd): 0.4x logical size

Storage cost at $0.02/GB/month:
  1 TB logical:
    B-tree: $28/month
    LSM (LZ4): $11/month
    LSM (Zstd): $8/month

LSM saves $240/year per TB
```

---

## When LSM Trees Struggle

###  Read-Heavy Workloads

```
Write:Read ratio < 1:10
Examples:
  - Caching layers (read-dominated)
  - Reference data (rarely updated)
  - Materialized views
```

**Why:** Read amplification dominates, B-tree's O(1) reads win.

---

###  Small Datasets (< 100 GB)

```
Dataset: 10 GB
Memory: 64 GB

B-tree: Entire dataset fits in RAM
  Read latency: ~100ns (RAM lookup)

LSM: Still checks multiple levels
  Read latency: ~1µs (bloom + levels)

B-tree is 10x faster for in-memory workloads
```

---

###  Random Access Patterns

```
Workload: Random point queries (no locality)

B-tree: O(log N) with good cache behavior
LSM: L0_files + L levels to check, cache thrashing

If no bloom filters:
  LSM: 9 random reads vs B-tree: 3 random reads
  LSM is 3x slower
```

---

###  Strict Latency SLOs

```
SLO: p99 < 1ms for all operations

LSM challenges:
  - Compaction pauses (10-100ms spikes)
  - L0 backlog → read amplification spikes
  - Write stalls (L0 > threshold)

B-tree: More predictable latency (no background compaction)
```

---

## LSM vs B-Tree: Summary

| Dimension | LSM-Tree | B-Tree | Winner |
|-----------|----------|--------|--------|
| **Write Throughput** | 10-100x higher | Baseline | **LSM** |
| **Write Latency (avg)** | ~50µs | ~500µs | **LSM** |
| **Write Latency (p99)** | ~5ms (stalls) | ~1ms | **B-tree** |
| **Point Query (in-mem)** | ~1µs | ~100ns | **B-tree** |
| **Point Query (on-disk)** | ~10µs (w/ blooms) | ~100µs | **LSM** |
| **Range Scan** | Similar (both sorted) | Similar | **Tie** |
| **Space Amplification** | 1.1-1.5x | 1.4-2.0x | **LSM** |
| **Write Amplification** | 10-40x | 1-5x | **B-tree** |
| **SSD Lifespan** | 6+ years | 1-2 years | **LSM** |
| **Operational Complexity** | High (tuning) | Low (mature) | **B-tree** |

---

## Key Takeaways

1. **LSM-trees trade read performance for write performance**
   - Sequential writes → 10-100x higher write throughput
   - Multiple levels → higher read amplification

2. **Write amplification is the key metric**
   - Formula: `WA = T * (L - 1)` for leveled compaction
   - Directly impacts SSD lifespan and I/O cost

3. **Bloom filters are essential**
   - Without blooms: LSM is slower than B-tree for reads
   - With blooms: LSM competitive or faster (large datasets)

4. **LSM is a family, not a single algorithm**
   - Leveled compaction: Low read amp, high write amp
   - Tiered compaction: Low write amp, high read amp
   - Hybrid strategies: Adapt per workload

5. **The RUM Conjecture applies**
   - Can't optimize read, write, and space simultaneously
   - LSM chooses: optimize write + space, sacrifice read

6. **Workload determines winner**
   - Write-heavy → LSM
   - Read-heavy → B-tree
   - Mixed → Depends on ratio and dataset size

---

## What's Next?

Now that you understand LSM fundamentals, explore:

- **[LSM Variants](lsm-variants.md)** - Leveled, Tiered, Universal compaction strategies
- **[ATLL Architecture](atll-architecture.md)** - Our adaptive approach (guard partitioning, dynamic K)
- **[Write Path](write-path.md)** - How writes flow through memtable → L0 → L1+
- **[Read Path](read-path.md)** - How reads check memtable → L0 → levels with Bloom filters

---

## Further Reading

**Foundational Papers:**
- [The Log-Structured Merge-Tree (LSM-Tree)](https://www.cs.umb.edu/~poneil/lsmtree.pdf) - O'Neil et al., 1996
- [Bigtable: A Distributed Storage System](https://static.googleusercontent.com/media/research.google.com/en//archive/bigtable-osdi06.pdf) - Chang et al., 2006
- [Designing Access Methods: The RUM Conjecture](http://daslab.seas.harvard.edu/rum-conjecture/) - Athanassoulis et al., 2016

**Implementations:**
- [LevelDB](https://github.com/google/leveldb) - Google's embeddable LSM
- [RocksDB](https://github.com/facebook/rocksdb) - Meta's production LSM
- [Pebble](https://github.com/cockroachdb/pebble) - CockroachDB's Go LSM

**Books:**
- "Database Internals" by Alex Petrov (Chapter 7: LSM Trees)
- "Designing Data-Intensive Applications" by Martin Kleppmann (Chapter 3: Storage)
