# Read Path

How reads flow through nori-lsm from `get()` call through memtable, L0, and slots, including bloom filter optimization and block caching.

---

## Overview

**Read path stages** (newest to oldest):
1. **Check memtable** → In-memory buffer (hot data)
2. **Check L0 files** → Recently flushed SSTables (overlapping)
3. **Check slot** → Range-partitioned SSTables (K-way fanout)
4. **Return None** → Key doesn't exist

**Latency breakdown** (typical values):
```
get("key"):
  ├─ Memtable lookup:     50ns     (in-memory skiplist)
  ├─ L0 bloom filters:    400ns    (6 files × 67ns each)
  ├─ Slot bloom filter:   67ns     (1 slot)
  ├─ Block index lookup:  100ns    (binary search)
  ├─ Block read (cached): 500ns    (LRU cache hit)
  └─ Block read (disk):   100µs    (SSD read)

Total (cache hit):  ~1-2µs
Total (cache miss): ~100µs
```

**Key optimization**: Bloom filters prevent 99% of unnecessary disk reads.

---

## Stage 1: Memtable Lookup

### Purpose

**Fastest path**: Check in-memory buffer for recent writes

**Data Structure**: Skip list (sorted, lock-free)
```
┌────────────────────────────────────────┐
│  Active Memtable (current writes)      │
│  Skip List: [a][b][c][g][m][z]        │
└────────────────────────────────────────┘
```

**Lookup Algorithm**:
```rust
pub fn get(&self, key: &[u8]) -> Option<Vec<u8>> {
    // O(log N) expected, lock-free
    self.skiplist.get(key)
}
```

**Result Handling**:
```rust
match memtable.get(key) {
    Some(value) => return Ok(Some(value)),  // Cache hit (fast path)
    None => continue to L0,                   // Cache miss
}
```

**Performance**:
- **Latency**: 50-100ns (in-memory, no I/O)
- **Hit Rate**: Depends on workload
  - Write-heavy: 20-40% (recent writes in memtable)
  - Read-heavy: 5-10% (most reads hit older data)

---

## Stage 2: L0 Bloom Filters

### Problem: L0 Overlaps

**L0 files overlap** (contain arbitrary key ranges):
```
L0-001.sst: [a, g, m, z]  (4 keys)
L0-002.sst: [b, c, n]     (3 keys)
L0-003.sst: [d, p, q]     (3 keys)
```

**Naive Approach** (without bloom filters):
```rust
// Check every L0 file (slow!)
for l0_file in l0_files {
    if let Some(value) = l0_file.get(key)? {
        return Ok(Some(value));
    }
}
// Worst case: 6 disk reads for negative lookup
```

### Bloom Filter Optimization

**Bloom Filter**: Probabilistic set membership test
```
Bloom filter for L0-001.sst: [a, g, m, z]

Query "m":  bloom.contains("m") → true   (might exist)
Query "x":  bloom.contains("x") → false  (definitely doesn't exist)
```

**False Positive Rate**: 0.9% (10 bits/key)
```
FP rate ≈ 0.6185^(m/n)
        ≈ 0.6185^10
        ≈ 0.009 (0.9%)
```

**Optimized Lookup**:
```rust
for l0_file in l0_files {
    // 1. Check bloom filter (67ns, in-memory)
    if !l0_file.bloom.contains(key) {
        continue;  // Skip this file (definite miss)
    }

    // 2. Read SSTable (100µs, disk I/O)
    if let Some(value) = l0_file.get(key)? {
        return Ok(Some(value));
    }
}
```

**Performance**:
- **Bloom check**: 67ns per file (in-memory)
- **Disk reads avoided**: 99.1% of negative lookups
- **Total bloom overhead**: 6 files × 67ns = 400ns

**Example** (6 L0 files):
```
Without blooms:
  Negative lookup: 6 disk reads × 100µs = 600µs

With blooms:
  Bloom checks: 6 × 67ns = 400ns
  Disk reads: 0 (0.9% false positive rate)
  Average latency: 400ns + (0.009 × 100µs) ≈ 1.3µs

Improvement: 600µs / 1.3µs ≈ 460x faster
```

---

## Stage 3: Slot Selection

### Guard Key Routing

**Problem**: Which slot contains the key?

**Guard Keys**: Fixed boundaries for each slot
```
Slot 0: [0x00, 0x40)  (a-d)
Slot 1: [0x40, 0x80)  (e-m)
Slot 2: [0x80, 0xC0)  (n-t)
Slot 3: [0xC0, ∞)     (u-z)
```

**Binary Search** (O(log num_slots)):
```rust
fn find_slot_for_key(&self, key: &[u8]) -> u32 {
    // Binary search on guard keys
    self.slots
        .binary_search_by(|slot| {
            if key < slot.guard_key_min {
                Ordering::Greater
            } else if key >= slot.guard_key_max {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .unwrap()
}
```

**Example**:
```
Query: get("m")

Binary search:
  Compare "m" vs Slot 1 ([e..m))  → "m" >= 0x80 (Slot 2)
  Compare "m" vs Slot 2 ([n..t))  → "m" < 0x80 (Slot 1)
  Found: Slot 1

Result: Check Slot 1 SSTables only (not Slots 0, 2, 3)
```

**Performance**:
- **Latency**: 10-20ns (binary search on 16 slots)
- **Benefit**: Reduces search space from all slots to 1 slot

---

## Stage 4: Slot Bloom Filters

### K-Way Fanout

**Slot structure** (K sorted runs):
```
Slot 1 (k_max=3):
  Run 0 (newest): [e, f, g]
  Run 1:          [h, i, j]
  Run 2 (oldest): [k, l, m]
```

**Bloom Filter Check** (same as L0):
```rust
for run in slot.runs {
    // 1. Check bloom filter (67ns)
    if !run.bloom.contains(key) {
        continue;
    }

    // 2. Read SSTable if bloom says "maybe"
    if let Some(value) = run.get(key)? {
        return Ok(Some(value));
    }
}
```

**Read Amplification**:
```
Hot slot (k_max=1):
  RA = 1 bloom check + 1 disk read (worst case)

Cold slot (k_max=4):
  RA = 4 bloom checks + 4 disk reads (worst case)
```

**ATLL Advantage**: Hot slots converge to k_max=1
```
get("hot_key"):
  Slot bloom checks: 1 × 67ns = 67ns
  Disk reads (worst): 1 × 100µs = 100µs

get("cold_key"):
  Slot bloom checks: 4 × 67ns = 268ns
  Disk reads (worst): 4 × 100µs = 400µs
```

---

## Stage 5: SSTable Lookup

### Block Index

**SSTable Structure** (refresher):
```
┌────────────────────────────────────────┐
│  Data Blocks (4KB each)                │
│   ├─ Block 0: [a..f]   offset=0       │
│   ├─ Block 1: [g..m]   offset=4096    │
│   └─ Block 2: [n..z]   offset=8192    │
├────────────────────────────────────────┤
│  Block Index (in-memory)               │
│   ├─ Block 0: first_key="a"           │
│   ├─ Block 1: first_key="g"           │
│   └─ Block 2: first_key="n"           │
├────────────────────────────────────────┤
│  Bloom Filter (loaded on open)         │
└────────────────────────────────────────┘
```

**Index Lookup** (binary search):
```rust
fn find_block_for_key(&self, key: &[u8]) -> usize {
    // Binary search on block index (O(log num_blocks))
    self.index
        .binary_search_by(|block| block.first_key.cmp(key))
        .unwrap_or_else(|i| i - 1)
}
```

**Example**:
```
Query: get("m")

Block index:
  Block 0: first_key="a"
  Block 1: first_key="g"
  Block 2: first_key="n"

Binary search:
  "m" >= "g" (Block 1) and "m" < "n" (Block 2)
  → Read Block 1

Result: Read 1 block (4KB) instead of entire SSTable (64KB)
```

**Performance**:
- **Latency**: 50-100ns (binary search on ~16 blocks)
- **I/O Saved**: 94% (read 1 block instead of 16)

---

## Stage 6: Block Cache

### LRU Cache

**Purpose**: Cache hot blocks in memory (avoid repeated disk reads)

**Configuration**:
```rust
pub struct ResourceConfig {
    pub block_cache_mib: usize,  // Default: 1024 MB
}
```

**Lookup**:
```rust
async fn read_block(&self, block_id: u64) -> Result<Block> {
    // 1. Check cache (fast path)
    if let Some(block) = self.cache.get(&block_id) {
        return Ok(block);  // Cache hit (~500ns)
    }

    // 2. Read from disk (slow path)
    let block = self.read_block_from_disk(block_id).await?;  // ~100µs

    // 3. Cache for future reads
    self.cache.put(block_id, block.clone());

    Ok(block)
}
```

**Cache Hit Rate**:
```
Working set < cache size:
  Hit rate: 95-99% (steady state)
  Average latency: ~500ns

Working set > cache size:
  Hit rate: 50-80% (cache thrashing)
  Average latency: ~50µs (50% cache miss)
```

**Performance**:
- **Cache hit**: 500ns (in-memory read)
- **Cache miss**: 100µs (SSD read)
- **Improvement**: 200x faster when cached

---

## Read Amplification Analysis

### Formula

**Read Amplification (RA)** = Number of SSTables checked for one key

### Per-Stage RA

**1. Memtable**:
```
RA_memtable = 0 (in-memory, no I/O)
```

**2. L0 Files**:
```
RA_l0 = l0_file_count

Example:
  6 L0 files → RA = 6
  (Bloom filters avoid actual reads 99% of the time)
```

**3. Slot Runs**:
```
RA_slot = k_max

Hot slot (k_max=1):  RA = 1
Cold slot (k_max=4): RA = 4
```

### Total RA

**Formula**:
```
RA_total = RA_l0 + RA_slot
         = l0_file_count + k_max
```

**Example** (6 L0 files, hot slot):
```
RA_total = 6 + 1 = 7 SSTables checked

With bloom filters:
  Bloom checks: 7 (all in-memory)
  Disk reads: 1-2 (bloom filters eliminate 5-6)
```

**Comparison**:
```
ATLL (hot slot, k_max=1):
  RA = 6 + 1 = 7

Leveled Compaction:
  RA = L0 + L ≈ 6 + 5 = 11

Tiered Compaction:
  RA = 10-15 (multiple runs per level)
```

---

## Point Queries vs Range Scans

### Point Query (get)

**Optimized path**:
```rust
pub async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
    // 1. Check memtable (50ns)
    if let Some(value) = self.memtable.get(key) {
        return Ok(Some(value));
    }

    // 2. Check L0 with bloom filters (400ns + 1-2 disk reads)
    for l0_file in &self.l0_files {
        if l0_file.bloom.contains(key) {
            if let Some(value) = l0_file.get(key).await? {
                return Ok(Some(value));
            }
        }
    }

    // 3. Find slot (10ns binary search)
    let slot_id = self.find_slot_for_key(key);

    // 4. Check slot runs with bloom filters (67ns + 1 disk read)
    for run in &self.slots[slot_id].runs {
        if run.bloom.contains(key) {
            if let Some(value) = run.get(key).await? {
                return Ok(Some(value));
            }
        }
    }

    // 4. Not found
    Ok(None)
}
```

**Performance**:
- **Memtable hit**: 50ns
- **L0/Slot hit (cached)**: 1-2µs
- **L0/Slot hit (disk)**: 100-200µs
- **Miss (not found)**: 500ns (bloom filters only)

### Range Scan (scan)

**Problem**: Bloom filters don't help
```
Scan [a..z]:
  → Must read all blocks in range (no bloom filter skip)
```

**Merge-Sort Algorithm**:
```rust
pub async fn scan(&self, start: &[u8], end: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    // 1. Create iterators for each source
    let mut iterators = vec![];

    // Memtable iterator
    iterators.push(self.memtable.range(start, end));

    // L0 iterators (all files overlap range)
    for l0_file in &self.l0_files {
        iterators.push(l0_file.range(start, end).await?);
    }

    // Slot iterator (only relevant slot)
    let slot_id = self.find_slot_for_key(start);
    for run in &self.slots[slot_id].runs {
        iterators.push(run.range(start, end).await?);
    }

    // 2. K-way merge (keep newest version)
    let mut merger = KWayMerge::new(iterators);
    let mut results = vec![];

    while let Some((key, value)) = merger.next().await? {
        if key >= end {
            break;
        }
        results.push((key, value));
    }

    Ok(results)
}
```

**Read Amplification**:
```
Scan [a..z]:
  Sources = 1 memtable + 6 L0 + k_max slot runs
          = 1 + 6 + 1 (hot slot)
          = 8 sources

K-way merge: O(N × log(8)) comparisons
```

**Performance**:
- **Small scans** (<100 keys): 1-10ms
- **Large scans** (1M keys): 100-1000ms
- **Bottleneck**: Disk I/O (sequential reads faster than random)

---

## Caching Strategy

### Three-Level Cache Hierarchy

**1. Memtable** (in-memory, no cache):
```
Size: 64 MB (configurable)
Purpose: Recent writes
Eviction: Flush to L0 when full
```

**2. Block Cache** (LRU, configurable):
```
Size: 1024 MB (default)
Purpose: Hot data blocks
Eviction: LRU (least recently used)
```

**3. Index Cache** (LRU, configurable):
```
Size: 128 MB (default)
Purpose: Block index metadata
Eviction: LRU
```

### Cache Warming

**Problem**: Cold cache after restart (all misses)

**Solution**: Proactive warming
```rust
pub async fn warm_cache(&self, hot_keys: &[Vec<u8>]) -> Result<()> {
    for key in hot_keys {
        // Trigger reads to populate cache
        let _ = self.get(key).await?;
    }
    Ok(())
}
```

**Example**:
```
Startup:
  1. Load bloom filters (125 KB per 100K keys)
  2. Load block indexes (10 KB per SSTable)
  3. Warm cache with recent keys (proactive reads)

Result:
  95% cache hit rate within 1 minute
```

---

## Performance Optimization Techniques

### 1. Bloom Filter Tuning

**Trade-off**:
```
10 bits/key:
  Size: 125 KB per 100K keys
  FP rate: 0.9%
  Wasted reads: 900 per 100K misses

12 bits/key:
  Size: 150 KB per 100K keys (+20%)
  FP rate: 0.3%
  Wasted reads: 300 per 100K misses

Decision: 10 bits/key (default)
  → 20% more memory not worth 600 fewer wasted reads
```

### 2. Block Size Tuning

**Trade-off**:
```
Small blocks (2 KB):
   Lower read amplification (read less data)
   More index overhead (more blocks)
   Worse compression (less context)

Large blocks (16 KB):
   Better compression (more context)
   Less index overhead
   Higher read amplification (read more data)

Default: 4 KB
  → Good balance for SSD page size
```

### 3. Prefetching

**Idea**: Speculatively load next block during sequential scans
```rust
async fn scan_with_prefetch(&self, start: &[u8], end: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let block_id = self.find_block_for_key(start);

    // Prefetch next N blocks in background
    for i in 1..=prefetch_distance {
        tokio::spawn(self.read_block(block_id + i));
    }

    // Read current block
    let block = self.read_block(block_id).await?;
    // ... process block ...
}
```

**Performance**:
- **Random reads**: No benefit (unpredictable pattern)
- **Sequential scans**: 2-3x faster (hide disk latency)

### 4. Adaptive Read-Ahead

**Idea**: Increase prefetch distance for sequential patterns
```rust
// Detect sequential access pattern
if access_pattern == Sequential {
    prefetch_distance = 8;  // Aggressive
} else {
    prefetch_distance = 2;  // Conservative
}
```

---

## Latency Targets

### p95 Latency Goals

**Point Queries**:
```
p50: < 1ms    (memtable or cache hit)
p95: < 10ms   (1-2 disk reads)
p99: < 20ms   (multiple disk reads)
```

**Range Scans** (100 keys):
```
p50: < 5ms    (mostly cached)
p95: < 50ms   (some disk reads)
p99: < 100ms  (cold cache)
```

### Breakdown (p95)

**Cache hit**:
```
Memtable lookup:     50ns
L0 bloom checks:     400ns (6 files)
Slot bloom check:    67ns
Block cache read:    500ns
─────────────────────────
Total:               ~1µs
```

**Cache miss** (hot slot):
```
Memtable lookup:     50ns
L0 bloom checks:     400ns
Slot bloom check:    67ns
Disk read:           100µs  (SSD)
Block decompression: 10µs
─────────────────────────
Total:               ~110µs
```

**Cache miss** (cold slot, k_max=4):
```
Memtable lookup:     50ns
L0 bloom checks:     400ns
Slot bloom checks:   268ns (4 runs)
Disk reads:          400µs  (4 SSTables)
Block decompression: 40µs   (4 blocks)
─────────────────────────
Total:               ~441µs
```

---

## Monitoring and Diagnostics

### Key Metrics

**Read Amplification**:
```rust
pub struct ReadStats {
    pub memtable_checks: u64,   // Always 1
    pub l0_files_checked: u64,  // Bloom filter checks
    pub slot_runs_checked: u64, // Bloom filter checks
    pub disk_reads: u64,        // Actual disk I/O
}
```

**Cache Metrics**:
```rust
pub struct CacheStats {
    pub block_cache_hits: u64,
    pub block_cache_misses: u64,
    pub index_cache_hits: u64,
    pub index_cache_misses: u64,
}
```

**Bloom Filter Effectiveness**:
```rust
pub struct BloomStats {
    pub bloom_checks: u64,        // Total bloom queries
    pub bloom_negatives: u64,     // Definite misses (saved disk reads)
    pub bloom_false_positives: u64, // Wasted disk reads
}
```

**Dashboard Queries**:
```sql
-- Cache hit rate
SELECT
  block_cache_hits / (block_cache_hits + block_cache_misses) AS hit_rate
FROM cache_stats;

-- Bloom filter false positive rate
SELECT
  bloom_false_positives / bloom_checks AS fp_rate
FROM bloom_stats;

-- Read amplification trend
SELECT
  AVG(l0_files_checked + slot_runs_checked) AS avg_ra
FROM read_stats
WHERE timestamp > NOW() - INTERVAL '1 hour';
```

---

## Summary

**Read Path Stages**:
1. **Memtable** (50ns) → Recent writes
2. **L0 bloom filters** (400ns) → 6 files, 99% skip
3. **Slot selection** (10ns) → Binary search
4. **Slot bloom filters** (67-268ns) → K-way fanout
5. **Block cache** (500ns) or disk (100µs)

**Read Amplification**:
- **ATLL (hot slot)**: RA = 7 (6 L0 + 1 slot)
- **ATLL (cold slot)**: RA = 10 (6 L0 + 4 slot)
- **Leveled**: RA = 11 (6 L0 + 5 levels)
- **Tiered**: RA = 10-15 (multiple runs)

**Optimization**:
- **Bloom filters**: 460x faster negative lookups
- **Block cache**: 200x faster cache hits
- **Slot partitioning**: 4x fewer SSTables checked

**Latency**:
- **Cache hit**: ~1µs (99th percentile)
- **Cache miss (hot)**: ~110µs
- **Cache miss (cold)**: ~441µs
- **Target p95**: < 10ms

**Next**: Read [When to Use ATLL](when-to-use.md) for workload guidance.

---

*Last Updated: 2025-10-31*
*See Also: [ATLL Architecture](atll-architecture.md), [Write Path](write-path.md), [Bloom Filter Strategy](../design-decisions/bloom-strategy.md)*
