# Guard-Based Partitioning

Why ATLL uses fixed guard keys for range partitioning instead of dynamic splitting.

---

## Decision

**Use fixed guard keys to partition the key space into slots (range shards) with stable boundaries.**

Each slot has:
```rust
pub struct Slot {
    slot_id: u32,
    guard_key_min: Vec<u8>,  // Inclusive lower bound (fixed)
    guard_key_max: Vec<u8>,  // Exclusive upper bound (fixed)
    runs: Vec<SortedRun>,    // K-way fanout (adaptive)
    k_max: usize,            // Dynamic, based on heat
    heat_score: f32,         // EWMA of access frequency
}
```

**Invariants:**
1. No gaps: Every key falls into exactly one slot
2. No overlaps: `slot[i].guard_key_max = slot[i+1].guard_key_min`
3. Stable: Guard keys never change (except manual rebalancing)

---

## Alternatives Considered

### 1. Dynamic Range Splitting (B-Tree Style)

**Approach:** Split ranges when they grow too large, merge when too small

```
Initial:
  Slot 0: [0x00, ∞)

After 100K writes to [a..z]:
  Split into:
    Slot 0: [0x00, 0x80)  (first half)
    Slot 1: [0x80, ∞)     (second half)

After more writes:
  Split Slot 0 into:
    Slot 0: [0x00, 0x40)
    Slot 2: [0x40, 0x80)
```

**Rejected because:**
- **Complex recovery:** Manifest must track dynamic splits/merges
- **Unpredictable routing:** Key → slot mapping changes over time
- **Cascading updates:** Split affects neighboring slots
- **Learning disruption:** Bandit scheduler loses history on split

**Trade-off:** Better load balancing vs stability

### 2. Hash-Based Partitioning

**Approach:** Hash key to determine slot

```rust
fn key_to_slot(key: &[u8], num_slots: u32) -> u32 {
    let hash = xxhash64(key, seed: 0);
    (hash % num_slots) as u32
}
```

**Rejected because:**
- **No range queries:** Cannot efficiently scan [start, end)
- **Poor locality:** Adjacent keys land in different slots
- **No compaction optimization:** Can't compact range subsets
- **Bloom filter inefficiency:** Must check all slots for range scan

**Trade-off:** Perfect load balancing vs range query support

### 3. Time-Window Bucketing (ScyllaDB ICS)

**Approach:** Partition by time, not key

```
Recent data (last 1 hour):   Slot 0 (leveled, K=1)
Last 24 hours:                Slot 1 (hybrid, K=2)
Last 30 days:                 Slot 2 (tiered, K=4)
Older than 30 days:           Slot 3 (tiered, K=8)
```

**Rejected because:**
- **Assumes time-series:** Not general-purpose
- **Requires timestamp extraction:** Key format dependency
- **Manual configuration:** Window sizes need tuning
- **Access pattern mismatch:** Recent ≠ hot (many workloads)

**Trade-off:** Optimized for time-series vs general key-value

### 4. No Partitioning (Global Compaction)

**Approach:** All data in one level (like traditional LSM)

```
L0: Overlapping files
L1: Single sorted run (entire key space)
L2: Single sorted run (entire key space)
...
```

**Rejected because:**
- **No per-range adaptation:** Hot/cold treated identically
- **Serial compaction:** Can't parallelize across ranges
- **Large compaction jobs:** Merge entire level (slow)
- **No RUM optimization:** Global trade-off (not per-range)

**Trade-off:** Simplicity vs adaptive performance

---

## Rationale

### 1. Predictable Routing

**Fixed boundaries enable deterministic mapping:**

```rust
fn find_slot_for_key(&self, key: &[u8]) -> u32 {
    // Binary search on guard keys (O(log num_slots))
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

**Benefits:**
- **Constant-time routing** (after binary search)
- **Same key → same slot** (always)
- **No routing table** (guard keys stored once in manifest)
- **Cache-friendly** (predictable access patterns)

**Example:**
```
Query: get("user:12345")

Guard keys:
  Slot 0: [0x00, 0x40)
  Slot 1: [0x40, 0x80)
  Slot 2: [0x80, 0xC0)
  Slot 3: [0xC0, ∞)

"user:12345" → 0x75... (hash prefix)
Binary search: 0x40 ≤ 0x75 < 0x80
Result: Slot 1 (always)
```

### 2. Simple Recovery

**Manifest format:**
```rust
pub struct Manifest {
    slots: Vec<SlotMetadata>,
}

pub struct SlotMetadata {
    slot_id: u32,
    guard_key_min: Vec<u8>,
    guard_key_max: Vec<u8>,
    runs: Vec<RunMetadata>,  // SSTable file paths
}
```

**On crash:**
1. Read manifest (guard keys + SSTable list)
2. Reconstruct slots (no dynamic state to rebuild)
3. Resume compaction (bandit state resets, learns quickly)

**Contrast with dynamic splitting:**
```rust
// Dynamic system would need:
pub struct Manifest {
    split_history: Vec<SplitEvent>,  // When/how slots split
    merge_history: Vec<MergeEvent>,  // When/how slots merged
    current_boundaries: Vec<(Vec<u8>, Vec<u8>)>,  // Rebuild from history
}
```

**Complexity:** Fixed guard keys → simple manifest format

### 3. Stable Boundaries for Learning

**Bandit scheduler tracks per-slot rewards:**

```rust
pub struct BanditArm {
    slot_id: u32,
    avg_reward: f64,         // EMA of historical rewards
    selection_count: u64,    // Times this slot was compacted
}
```

**Fixed boundaries enable:**
- **Cumulative learning:** Slot 0 today = Slot 0 tomorrow
- **Reward convergence:** EMA stabilizes over 10-20 selections
- **UCB score accuracy:** Exploration bonus based on selection count

**Contrast with dynamic splitting:**
```
Initial:
  Slot 0: [a..z], compacted 100 times, avg_reward = 8.5

After split:
  Slot 0: [a..m], compacted 0 times (reset!)
  Slot 1: [m..z], compacted 0 times (reset!)

Problem: Lost 100 compactions worth of learning
```

**Benefit:** Stable boundaries preserve learned behavior

### 4. Range Query Locality

**Range queries benefit from contiguous slots:**

```rust
async fn scan(&self, start: &[u8], end: &[u8]) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    // Find slots overlapping [start, end)
    let start_slot = self.find_slot_for_key(start);
    let end_slot = self.find_slot_for_key(end);

    // Scan contiguous slot range
    for slot_id in start_slot..=end_slot {
        // Merge iterators from slot's runs
    }
}
```

**Example:**
```
Query: scan("user:", "user:~")

Guard keys:
  Slot 0: [0x00, 0x40)  (not users)
  Slot 1: [0x40, 0x80)  (users a-m)
  Slot 2: [0x80, 0xC0)  (users n-z)
  Slot 3: [0xC0, ∞)     (not users)

Result: Scan slots 1-2 only (skip 0, 3)
```

**Contrast with hash partitioning:**
```
With hash:
  Must scan all 16 slots (no locality)
  10x more I/O for range queries
```

---

## Trade-offs

### What We Gained

**1. Predictability**
- Same key always routes to same slot
- No surprise routing changes
- Deterministic recovery

**2. Simplicity**
- Manifest stores guard keys once
- No split/merge history tracking
- Binary search for routing

**3. Learning Stability**
- Bandit scheduler accumulates knowledge
- Reward convergence guaranteed
- No learning resets

**4. Range Query Efficiency**
- Scan contiguous slots
- Skip irrelevant ranges
- Locality benefits

### What We Gave Up

**1. Load Balancing**
- Skewed writes create imbalanced slots
- No automatic rebalancing
- Manual intervention needed for severe skew

**2. Storage Overhead**
- Small slots waste space (metadata overhead)
- Large slots slow compaction (merge entire range)
- Fixed granularity (can't adjust dynamically)

**3. Flexibility**
- Can't optimize per-workload (time-series, geo, etc.)
- Guard keys hardcoded (not configurable at runtime)
- Rebalancing requires data migration

---

## Guard Key Selection Strategies

### Uniform Split (Default)

**Approach:** Divide key space evenly

```rust
fn uniform_guard_keys(num_slots: u32) -> Vec<Vec<u8>> {
    let mut guards = vec![];
    let step = 256 / num_slots;

    for i in 0..num_slots {
        guards.push(vec![(i * step) as u8]);
    }

    guards.push(vec![0xFF]);  // Final boundary
    guards
}
```

**Example (16 slots):**
```
Slot 0:  [0x00, 0x10)
Slot 1:  [0x10, 0x20)
...
Slot 15: [0xF0, 0xFF]
```

**Pros:**
- Simple implementation
- Predictable boundaries
- No sampling required

**Cons:**
- Assumes uniform key distribution
- Skewed workloads create imbalance
- No adaptation to actual data

### Data-Driven Split (Future)

**Approach:** Sample existing data, split by size

```rust
async fn data_driven_guard_keys(num_slots: u32) -> Vec<Vec<u8>> {
    // 1. Sample 10K random keys
    let samples = self.sample_keys(10_000).await?;

    // 2. Sort samples
    samples.sort();

    // 3. Choose percentile boundaries
    let mut guards = vec![];
    for i in 0..num_slots {
        let percentile = (i as f64) / (num_slots as f64);
        let idx = (percentile * samples.len() as f64) as usize;
        guards.push(samples[idx].clone());
    }

    guards
}
```

**Pros:**
- Balanced slot sizes
- Adapts to actual key distribution
- Better for skewed workloads

**Cons:**
- Requires sampling (slow on startup)
- Changes with data (not stable)
- Complex recovery (must resample)

### Workload-Specific (ScyllaDB)

**Approach:** Partition by semantic meaning

```rust
// Time-series workload
Slot 0: timestamps [0, 3600)        (last hour)
Slot 1: timestamps [3600, 86400)    (last day)
Slot 2: timestamps [86400, 2592000) (last 30 days)
Slot 3: timestamps [2592000, ∞)     (older)

// Geo workload
Slot 0: latitude [-90, -45)  (southern hemisphere)
Slot 1: latitude [-45, 0)    (southern temperate)
Slot 2: latitude [0, 45)     (northern temperate)
Slot 3: latitude [45, 90]    (northern hemisphere)
```

**Pros:**
- Optimized for specific use case
- Aligns with query patterns
- Domain knowledge encoded

**Cons:**
- Not general-purpose
- Requires workload analysis
- Brittle (breaks if assumptions change)

**Decision:** Use uniform split by default, allow override via configuration

---

## Rebalancing Strategy (Future Work)

**Problem:** Skewed writes create imbalanced slots

```
After 1M writes to "user:" prefix:

Slot 0 [0x00, 0x40):  100 MB   (cold, few writes)
Slot 1 [0x40, 0x80):  5 GB     (hot, many writes to "user:")
Slot 2 [0x80, 0xC0):  200 MB   (cold)
Slot 3 [0xC0, ∞):     150 MB   (cold)

Problem: Slot 1 is 50x larger than others
```

**Proposed Solution:** Periodic rebalancing

```rust
async fn rebalance_slots(&mut self) -> Result<()> {
    // 1. Identify imbalanced slots (size > 2× median)
    let median_size = self.median_slot_size();
    let large_slots: Vec<u32> = self.slots
        .iter()
        .filter(|s| s.total_size() > median_size * 2)
        .map(|s| s.slot_id)
        .collect();

    // 2. For each large slot, split into two
    for slot_id in large_slots {
        let mid_key = self.find_median_key(slot_id).await?;
        self.split_slot(slot_id, mid_key).await?;
    }

    // 3. Update manifest with new guard keys
    self.manifest.update_guard_keys(&self.slots).await?;

    Ok(())
}
```

**Trade-off:**
- **Benefit:** Balanced slot sizes, better parallelism
- **Cost:** Data migration, learning reset, complex implementation

**Status:** Not implemented (manual rebalancing via configuration)

---

## Configuration

**Default configuration:**

```rust
pub struct ATLLConfig {
    /// Number of range-partitioned slots
    /// Default: 16
    /// Recommendation: 16-64 for most workloads
    pub num_slots: u32,

    /// Guard key selection strategy
    /// Default: Uniform split
    pub guard_key_strategy: GuardKeyStrategy,
}

pub enum GuardKeyStrategy {
    /// Evenly divide key space (default)
    Uniform,

    /// Sample existing data, split by size (future)
    DataDriven,

    /// Manual guard keys (expert mode)
    Manual(Vec<Vec<u8>>),
}
```

**Tuning guidelines:**

**Small datasets (<1 GB):**
```rust
ATLLConfig {
    num_slots: 8,  // Lower overhead
    ..Default::default()
}
```

**Large datasets (>100 GB):**
```rust
ATLLConfig {
    num_slots: 64,  // Finer granularity
    ..Default::default()
}
```

**Skewed workloads:**
```rust
// Option 1: More slots (finer partitioning)
ATLLConfig {
    num_slots: 128,
    ..Default::default()
}

// Option 2: Manual guard keys (expert mode)
ATLLConfig {
    guard_key_strategy: GuardKeyStrategy::Manual(vec![
        vec![0x00],
        vec![0x30],  // Split hot range finer
        vec![0x40],
        vec![0x50],
        vec![0x60],  // Hot range ends
        vec![0x80],
        vec![0xC0],
        vec![0xFF],
    ]),
    ..Default::default()
}
```

---

## Summary

**Guard-based partitioning with fixed boundaries is the right choice for ATLL because:**

1. **Predictable routing** - Same key → same slot (always)
2. **Simple recovery** - Manifest stores guard keys once
3. **Learning stability** - Bandit scheduler accumulates knowledge
4. **Range query efficiency** - Scan contiguous slots

**We accept these trade-offs:**
- No automatic rebalancing (manual intervention for severe skew)
- Fixed granularity (can't adjust dynamically)
- Storage overhead (metadata per slot)

**Future improvements:**
- Data-driven guard key selection (sampling-based)
- Periodic rebalancing (split large slots)
- Workload-specific strategies (time-series, geo)

**For most workloads, the defaults (16-64 uniform slots) provide the right balance of adaptability and simplicity.**

---

*Last Updated: 2025-10-31*
*See Also: [ATLL Architecture](../core-concepts/atll-architecture.md), [Adaptive K-Way Fanout](adaptive-k-fanout.md)*
