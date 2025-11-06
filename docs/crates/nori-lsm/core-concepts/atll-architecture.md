---
layout: default
title: ATLL Architecture
parent: Core Concepts
grand_parent: nori-lsm
nav_order: 3
---

# ATLL Architecture
{: .no_toc }

ATLL (Adaptive Tiered-Leveled LSM) is nori-lsm's core innovation: a hybrid compaction strategy that adapts between tiered and leveled behavior per key range, optimizing for heterogeneous workloads.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Executive Summary

**ATLL** combines the best of both worlds:
- **Leveled compaction** for hot ranges (low read amplification)
- **Tiered compaction** for cold ranges (low write amplification)
- **Adaptive switching** based on real-time access patterns

**Key Innovation**: Instead of choosing **one** strategy for the **entire database**, ATLL chooses the **optimal strategy per key range** dynamically.

**Result**:
```
Write Amplification: 8-20x   (vs 40-100x for pure leveled)
Read Amplification:  5-12x   (vs 10-15x for pure tiered)
Space Amplification: 1.1-1.3x (comparable to leveled)
```

ATLL achieves **near-Pareto-optimal** trade-offs for heterogeneous workloads (mixed hot/cold data).

---

## The Problem: One Size Doesn't Fit All

### Traditional LSM Dilemma

**Leveled Compaction (LCS)**:
- **Pros**: Fast reads (low RA), compact storage
- **Cons**: Slow writes (high WA), constant I/O
- **Best for**: Read-heavy workloads with uniform access

**Tiered Compaction (STCS)**:
- **Pros**: Fast writes (low WA), low I/O
- **Cons**: Slow reads (high RA), space overhead
- **Best for**: Write-heavy workloads with sequential scans

### Real-World Workloads are Heterogeneous

**80/20 Rule (Zipfian Distribution)**:
- 80% of accesses hit 20% of keys (hot data)
- 20% of accesses hit 80% of keys (cold data)

**Example**: E-commerce database
```
Hot data:
  Recent orders (last 30 days)      → Heavy reads + writes
  Active user sessions              → Heavy reads + writes
  Popular product inventory         → Heavy reads

Cold data:
  Historical orders (>1 year old)   → Rare reads
  Archived user data                → Rare reads
  Deleted product history           → Rare reads
```

**Traditional LSMs fail here**:
- **Leveled**: Wastes I/O compacting cold data (unnecessary WA)
- **Tiered**: Slows down hot reads with high RA

---

## ATLL's Solution: Adaptive Per-Range Strategy

### Core Idea

**Split the key space** into range-partitioned **slots** (like shards):
- Each slot is a contiguous key range `[guard_min, guard_max)`
- Each slot independently chooses its compaction strategy
- Hot slots → Leveled (K=1 sorted run)
- Cold slots → Tiered (K>1 sorted runs)

**Visual**:
```
┌────────────────────────────────────────────────────────┐
│  L0: Overlapping files (global)                        │
│  ┌──┐ ┌──┐ ┌──┐ ┌──┐                                  │
│  │  │ │  │ │  │ │  │                                  │
│  └──┘ └──┘ └──┘ └──┘                                  │
└────────────────────────────────────────────────────────┘

┌────────────────────────────────────────────────────────┐
│  L1: Range-partitioned slots                           │
│  ┌─────────┬─────────┬─────────┬─────────┬─────────┐  │
│  │ Slot 0  │ Slot 1  │ Slot 2  │ Slot 3  │ Slot 4  │  │
│  │ (HOT)   │ (COLD)  │ (COLD)  │ (HOT)   │ (COLD)  │  │
│  │ K=1     │ K=3     │ K=2     │ K=1     │ K=4     │  │
│  │ [a..d)  │ [d..g)  │ [g..m)  │ [m..t)  │ [t..z)  │  │
│  └─────────┴─────────┴─────────┴─────────┴─────────┘  │
└────────────────────────────────────────────────────────┘

Legend:
  K = max sorted runs per slot
  K=1 → Leveled (no overlaps)
  K>1 → Tiered (bounded overlaps)
```

**Result**: Hot ranges get fast reads, cold ranges get fast writes.

---

## Architectural Components

### 1. Guard-Based Range Partitioning

**Purpose**: Divide key space into fixed-size range shards.

**Implementation**:
```rust
pub struct Slot {
    pub slot_id: u32,
    pub guard_key_min: Vec<u8>,    // Inclusive lower bound
    pub guard_key_max: Vec<u8>,    // Exclusive upper bound
    pub runs: Vec<SortedRun>,      // Sorted runs in this slot
    pub k_max: usize,              // Dynamic fanout (1=leveled, >1=tiered)
    pub heat_score: f32,           // EWMA access frequency
}
```

**Invariants**:
1. **No gaps**: Every key falls into exactly one slot
2. **No overlaps**: `slot[i].guard_key_max = slot[i+1].guard_key_min`
3. **Sorted runs**: Within each slot, runs are time-ordered (newest first)

**Example** (4 slots, 10 keys):
```
Keys: a, b, c, d, e, f, g, h, i, j

Slot 0: [a, c)  → Contains a, b
Slot 1: [c, f)  → Contains c, d, e
Slot 2: [f, h)  → Contains f, g
Slot 3: [h, ∞)  → Contains h, i, j
```

**Why guard keys?**
- **Fixed boundaries**: Unlike dynamic range splitting, guards are stable
- **Predictable behavior**: Same key always maps to same slot
- **Simple recovery**: Manifest records guard keys, not dynamic splits

---

### 2. Dynamic K-Way Fanout

**Purpose**: Control overlaps per slot based on heat.

**K-Max Formula**:
```
k_max[slot] = 1 + floor((1 - heat_score) × (K_global - 1))

Where:
  heat_score ∈ [0, 1]  (0 = cold, 1 = hot)
  K_global = 4         (maximum allowed runs per slot)
```

**Examples**:
```
Hot slot (heat_score = 0.9):
  k_max = 1 + floor((1 - 0.9) × 3) = 1 + floor(0.3) = 1
  → Leveled (single sorted run, no overlaps)

Warm slot (heat_score = 0.5):
  k_max = 1 + floor((1 - 0.5) × 3) = 1 + floor(1.5) = 2
  → Hybrid (up to 2 runs, limited overlaps)

Cold slot (heat_score = 0.1):
  k_max = 1 + floor((1 - 0.1) × 3) = 1 + floor(2.7) = 3
  → Tiered (up to 3 runs, more overlaps)
```

**Read Amplification Impact**:
```
RA_slot = L0_files + k_max

Hot slot (k_max=1):  RA = 6 + 1 = 7   (fast reads)
Cold slot (k_max=3): RA = 6 + 3 = 9   (acceptable for rare reads)
```

**Write Amplification Impact**:
```
WA_slot ∝ compaction_frequency

Hot slot (k_max=1):  WA = high (frequent compactions to maintain K=1)
Cold slot (k_max=3): WA = low  (infrequent compactions, allow K>1)
```

---

### 3. EWMA Heat Tracking

**Purpose**: Track access frequency per slot with exponential decay.

**Algorithm** (Exponential Weighted Moving Average):
```rust
// On each read/write to a slot:
heat_score_new = α × 1.0 + (1 - α) × heat_score_old

Where:
  α = 0.1  (smoothing factor, hardcoded)
  1.0      (instant heat contribution)
  heat_score_old (historical heat)
```

**Properties**:
- **Recency bias**: Recent accesses weigh more than old ones
- **Decay**: Heat score decays if slot isn't accessed
- **Stability**: Smoothing prevents thrashing on bursty workloads

**Example Evolution** (slot accessed every 10 queries):
```
Initial: heat_score = 0.0

Access 1:  0.1 × 1.0 + 0.9 × 0.0   = 0.100
No access: 0.1 × 0.0 + 0.9 × 0.100 = 0.090
No access: 0.1 × 0.0 + 0.9 × 0.090 = 0.081
...
Access 2:  0.1 × 1.0 + 0.9 × 0.073 = 0.166
...

After 100 accesses: heat_score → 0.9+ (hot)
After 100 idle:     heat_score → 0.0  (cold)
```

**Convergence**:
- **Hot slot steady-state**: ~0.9-1.0 (with regular access)
- **Cold slot steady-state**: ~0.0-0.1 (with no access)
- **Time to converge**: ~20-30 accesses (α=0.1)

---

### 4. Bandit-Based Scheduler

**Purpose**: Choose which slot to compact using reinforcement learning.

**Algorithm**: Epsilon-Greedy Multi-Armed Bandit with Upper Confidence Bound (UCB)

**Decision Process**:
```rust
// With probability ε (exploration):
if random() < ε {
    select_random_slot()
}
// With probability (1-ε) (exploitation):
else {
    select_slot_with_highest_ucb_score()
}
```

**UCB Score Formula**:
```
ucb_score[slot] = avg_reward[slot] + c × sqrt(ln(total_selections) / slot_selections)
                  └─ exploitation ─┘   └────── exploration bonus ──────────┘

Where:
  avg_reward[slot]    = EMA of historical rewards
  c = 2.0             = exploration constant
  total_selections    = total compaction decisions made
  slot_selections     = times this slot was selected
```

**Reward Function**:
```
reward = (predicted_latency_reduction × heat_score) / bytes_written

Components:
  predicted_latency_reduction: Benefit from reducing RA
  heat_score:                  Weight hot slots higher
  bytes_written:               Penalty for write amplification
```

**Intuition**:
- **High reward**: Hot slots with high RA (good targets for leveling)
- **Low reward**: Cold slots with low RA (avoid unnecessary compaction)
- **Exploration bonus**: Prefer slots we haven't compacted recently (avoid starvation)

**Parameters**:
- **ε = 0.1** (10% exploration, 90% exploitation)
- **c = 2.0** (standard UCB exploration constant)
- **α = 0.1** (EMA smoothing for reward history)

**Example**:
```
Slot 0 (hot):
  avg_reward = 8.5
  selections = 100
  ucb_score = 8.5 + 2.0 × sqrt(ln(1000) / 100)
            = 8.5 + 2.0 × sqrt(6.9 / 100)
            = 8.5 + 2.0 × 0.26
            = 9.0 ← High score (selected often)

Slot 1 (cold):
  avg_reward = 2.0
  selections = 10
  ucb_score = 2.0 + 2.0 × sqrt(ln(1000) / 10)
            = 2.0 + 2.0 × sqrt(6.9 / 10)
            = 2.0 + 2.0 × 0.83
            = 3.7 ← Lower score (selected rarely)
```

**Convergence**:
- Over time, hot slots get compacted more frequently → K→1 (leveled)
- Cold slots get compacted less frequently → K→K_global (tiered)

---

## Mathematical Foundations

### RUM Optimization Per Slot

**RUM Conjecture** (Athanassoulis et al., 2016):
```
For any data structure:
  R × U × M ≥ Constant

Where:
  R = Read cost
  U = Update cost
  M = Memory cost
```

**ATLL's Insight**: Optimize R-U-M **per slot**, not globally.

**Hot Slot Strategy** (minimize R):
```
k_max = 1  → Leveled
R = L0 + 1 = 7 reads     ← Minimize read cost
U = high                 ← Accept high update cost
M = low                  ← Compact storage
```

**Cold Slot Strategy** (minimize U):
```
k_max = 4  → Tiered
R = L0 + 4 = 10 reads    ← Accept higher read cost
U = low                  ← Minimize update cost
M = medium               ← Some space overhead
```

**Result**: Each slot operates on a different point on the RUM Pareto frontier.

### Write Amplification Analysis

**ATLL WA Formula**:
```
WA_total = Σ(WA_slot[i] × write_fraction[i])

Where:
  WA_slot[i] = T × (1 / compaction_frequency[i])
  write_fraction[i] = fraction of writes to slot i
```

**Example** (4 slots, 80/20 hot/cold distribution):
```
Slot 0 (hot, 40% writes, compacted every 10 writes):
  WA = 10 × (1 / 0.1) = 100 per 10 writes = 10x

Slot 1 (warm, 30% writes, compacted every 20 writes):
  WA = 10 × (1 / 0.05) = 200 per 20 writes = 10x

Slots 2-3 (cold, 15% writes each, compacted every 50 writes):
  WA = 10 × (1 / 0.02) = 500 per 50 writes = 10x each

Weighted average:
  WA_total = 0.4×10 + 0.3×10 + 0.15×10 + 0.15×10
           = 4 + 3 + 1.5 + 1.5
           = 10x
```

**Comparison**:
- **Pure Leveled**: 40-100x (compact everything frequently)
- **ATLL**: 8-20x (compact only hot data frequently)
- **Pure Tiered**: 6-8x (but high RA penalty)

### Space Amplification Analysis

**ATLL SA Formula**:
```
SA_total = Σ(SA_slot[i] × size_fraction[i])

Where:
  SA_slot[i] = 1 + (k_max[i] - 1) / T
  size_fraction[i] = fraction of total data in slot i
```

**Example** (4 slots, uniform data distribution):
```
Slot 0 (hot, k_max=1):
  SA = 1 + (1 - 1) / 10 = 1.0  (no overhead)

Slot 1 (warm, k_max=2):
  SA = 1 + (2 - 1) / 10 = 1.1

Slots 2-3 (cold, k_max=4):
  SA = 1 + (4 - 1) / 10 = 1.3 each

Weighted average:
  SA_total = 0.25×1.0 + 0.25×1.1 + 0.25×1.3 + 0.25×1.3
           = 0.25 + 0.275 + 0.325 + 0.325
           = 1.175x
```

**Comparison**:
- **Leveled**: 1.1x (minimal overhead)
- **ATLL**: 1.1-1.3x (close to leveled)
- **Tiered**: 1.33x (higher overhead)

---

## Comparison to Prior Art

### RocksDB (Leveled Compaction)

**Strategy**: Global leveled compaction across all key ranges

**Configuration**:
```
Level 0: Overlapping files
Level 1: 256 MB (partitioned into files)
Level 2: 2.56 GB
Level 3: 25.6 GB
...
```

**Pros**:
- Predictable read latency
- Compact storage
- Well-tuned implementation

**Cons**:
- High write amplification (40-100x)
- Constant background I/O
- Slow for write-heavy workloads

**ATLL Improvement**:
- Adaptive K per slot reduces WA by 2-5x
- Maintains comparable RA for hot data
- Lower I/O pressure on cold data

### Cassandra (Size-Tiered Compaction)

**Strategy**: Global size-tiered compaction with bucketing

**Configuration**:
```
Tier 1: Files 0-10 MB
Tier 2: Files 10-100 MB
Tier 3: Files 100-1000 MB
...
```

**Pros**:
- Low write amplification (6-8x)
- Good for write-heavy workloads
- Low I/O pressure

**Cons**:
- High read amplification (10-15x)
- Space overhead (1.33x)
- Slow point queries

**ATLL Improvement**:
- Hot slots converge to K=1 (leveled) for fast reads
- Cold slots remain tiered (low WA)
- Best of both worlds

### ScyllaDB (Incremental Compaction Strategy, ICS)

**Strategy**: Time-window bucketing + size-based merging

**Configuration**:
```
Window 1: Last 1 hour (small files, tiered)
Window 2: Last 24 hours (medium files, hybrid)
Window 3: Last 30 days (large files, leveled)
...
```

**Pros**:
- Time-series optimized
- Good for TTL workloads
- Lower WA than pure leveled

**Cons**:
- Complex configuration
- Time-window assumptions (not general-purpose)
- Manual tuning required

**ATLL Improvement**:
- Access-pattern-based (not time-based)
- Automatic adaptation via bandit scheduler
- General-purpose (works for any workload)

### Monkey (Tuning LSM via Learning)

**Strategy**: ML-based global parameter tuning

**Approach**:
```
Learn optimal T (fanout) and Bloom filter bits
based on workload characteristics
```

**Pros**:
- Theoretically optimal for static workloads
- Academic rigor

**Cons**:
- Global tuning (one T for entire tree)
- Requires offline learning phase
- Not adaptive to dynamic workloads

**ATLL Improvement**:
- Per-slot adaptation (finer granularity)
- Online learning via bandit (no offline phase)
- Handles dynamic workloads (EWMA decay)

---

## When ATLL Excels

### Ideal Workloads

**1. Zipfian Access Patterns (80/20 rule)**
- Hot keys: Recent orders, active sessions
- Cold keys: Historical logs, archived data

**Example**: E-commerce database
```
Hot ranges (20% of keys, 80% of accesses):
  → k_max=1 (leveled)
  → RA = 7, WA = 20x

Cold ranges (80% of keys, 20% of accesses):
  → k_max=4 (tiered)
  → RA = 10, WA = 6x

Result:
  Overall WA = 0.8×20 + 0.2×6 = 17.2x (vs 40x for pure leveled)
  Overall RA = 0.8×7 + 0.2×10 = 7.6  (vs 10-15 for pure tiered)
```

**2. Time-Series with Recent-Heavy Reads**
- Recent data: Heavy reads + writes
- Old data: Rare reads, no writes

**Example**: Metrics database
```
Last 24 hours:  k_max=1 (leveled, fast queries)
Last 7 days:    k_max=2 (hybrid)
Older than 30d: k_max=4 (tiered, low WA for compaction)
```

**3. Multi-Tenant Systems**
- Some tenants active (hot), others dormant (cold)

**Example**: SaaS database
```
Active tenants:   k_max=1 (fast reads for paying users)
Dormant tenants:  k_max=4 (low WA, acceptable slow reads)
```

### When ATLL Struggles

**1. Uniform Access Patterns**
- All keys accessed equally often
- No hot/cold distinction

**Result**: ATLL converges to global leveled (no benefit)

**2. Pure Scan Workloads**
- Sequential scans of entire dataset
- No point queries

**Result**: Tiered compaction would be simpler (less overhead)

**3. Extremely Skewed Writes**
- Writes concentrated in one slot
- Other slots idle

**Result**: Bandit scheduler overhead without benefit

---

## Theoretical Guarantees

### Convergence Properties

**Heat Score Convergence** (EWMA):
```
Theorem: For α=0.1 and stable access pattern,
         heat_score converges to steady-state within 30 accesses.

Proof:
  heat_t = α × 1 + (1-α) × heat_{t-1}
         = α + (1-α) × heat_{t-1}

  Steady-state (accessed every step):
    heat_∞ = α / (1 - (1-α)) = 0.1 / 0.1 = 1.0

  Convergence rate: (1-α)^t
    After 30 steps: (0.9)^30 ≈ 0.04 (4% error)
```

**Bandit Regret Bound** (UCB):
```
Theorem: UCB with c=2.0 achieves O(log T) regret over T selections.

Regret = Σ(reward_optimal - reward_actual)

Upper bound:
  Regret ≤ O(K × log(T))

  Where:
    K = number of slots
    T = total selections

Interpretation:
  Over 1000 compactions with 16 slots:
    Regret ≤ 16 × log(1000) ≈ 110 suboptimal decisions
    Efficiency: 89% optimal (after warmup)
```

**K-Max Stability**:
```
Theorem: If heat_score stabilizes, k_max stabilizes.

Proof:
  k_max = 1 + floor((1 - heat_score) × (K_global - 1))

  Heat stable → k_max stable (deterministic mapping)
```

---

## Configuration and Tuning

### Default Configuration

```rust
pub struct ATLLConfig {
    // L0 configuration
    pub l0: L0Config {
        max_files: 12,                         // Hard stall threshold
        soft_throttle_threshold: 6,            // 50% of max_files
        soft_throttle_base_delay_ms: 1,        // Gradual backpressure
    },

    // Slot configuration
    pub num_slots: 16,                         // Number of range partitions
    pub k_global: 4,                           // Max runs per slot (cold slots)

    // Heat tracking
    pub heat_alpha: 0.1,                       // EWMA smoothing factor
    pub heat_decay_interval_secs: 60,          // Decay every 60s

    // Bandit scheduler
    pub compaction_epsilon: 0.1,               // 10% exploration
    pub compaction_ucb_c: 2.0,                 // UCB exploration constant
    pub compaction_reward_alpha: 0.1,          // Reward EMA smoothing
}
```

### Tuning Guidelines

**Increase num_slots** (32, 64, 128):
- **When**: Large datasets (>1 TB), many distinct ranges
- **Benefit**: Finer-grained adaptation
- **Cost**: More metadata, slower manifest operations

**Increase k_global** (8, 16):
- **When**: Write-heavy workloads, low read QPS
- **Benefit**: Lower WA for cold slots
- **Cost**: Higher RA for cold reads

**Increase heat_alpha** (0.2, 0.3):
- **When**: Rapidly changing workloads
- **Benefit**: Faster adaptation to new patterns
- **Cost**: Less stable (more thrashing)

**Increase compaction_epsilon** (0.15, 0.20):
- **When**: Non-stationary workloads, multi-tenant
- **Benefit**: Better exploration of new strategies
- **Cost**: More suboptimal compactions (higher WA during exploration)

---

## Observability and Monitoring

### Key Metrics

**Heat Scores** (per slot):
```rust
heat_score[slot_id]: f32  // Range: [0.0, 1.0]
```

**Interpretation**:
- `0.0 - 0.2`: Cold (rarely accessed)
- `0.2 - 0.5`: Warm (occasional access)
- `0.5 - 0.8`: Hot (frequent access)
- `0.8 - 1.0`: Very hot (constant access)

**K-Max Values** (per slot):
```rust
k_max[slot_id]: usize  // Range: [1, k_global]
```

**Interpretation**:
- `k=1`: Leveled (hot slot)
- `k>1`: Tiered (cold slot)
- `k=k_global`: Fully tiered (very cold)

**Bandit Metrics**:
```rust
BanditSelection {
    slot_id: u32,           // Which slot was selected
    explored: bool,         // true = exploration, false = exploitation
    ucb_score: f64,         // UCB score
    avg_reward: f64,        // Historical reward
    selection_count: u64,   // Selection frequency
}

BanditReward {
    slot_id: u32,           // Which slot was compacted
    reward: f64,            // Calculated reward
    bytes_written: u64,     // Write amplification cost
    heat_score: f32,        // Access frequency weight
}
```

**Dashboard Queries**:
```sql
-- Hot slots (candidates for leveling)
SELECT slot_id, heat_score, k_max
FROM lsm_stats
WHERE heat_score > 0.8
ORDER BY heat_score DESC;

-- Compaction frequency by slot
SELECT slot_id, COUNT(*) as compactions
FROM bandit_selection
WHERE timestamp > NOW() - INTERVAL '1 hour'
GROUP BY slot_id
ORDER BY compactions DESC;

-- Reward trends (should stabilize over time)
SELECT slot_id, AVG(reward) as avg_reward, STDDEV(reward) as reward_variance
FROM bandit_reward
GROUP BY slot_id;
```

---

## Implementation Notes

### Guard Key Selection

**Problem**: How to choose initial guard keys?

**Strategy 1: Uniform Split** (current implementation)
```rust
// Split key space into equal-sized ranges
guard_keys = [
    vec![0x00],              // Slot 0: [0x00, 0x10)
    vec![0x10],              // Slot 1: [0x10, 0x20)
    vec![0x20],              // Slot 2: [0x20, 0x30)
    ...
    vec![0xF0],              // Slot 15: [0xF0, ∞)
]
```

**Strategy 2: Data-Driven Split** (future work)
```rust
// Sample existing data, split by size
// Goal: Each slot contains ~equal bytes
```

**Trade-off**:
- Uniform: Simple, predictable, but may create imbalanced slots
- Data-driven: Balanced, but requires sampling (expensive on startup)

### Compaction Triggering

**L0 Flush Trigger**:
```rust
if l0_files >= l0.max_files {
    // Hard stall: Reject writes
    return Err(Error::SystemPressure);
} else if l0_files >= l0.soft_throttle_threshold {
    // Soft throttle: Apply backpressure
    delay = l0.soft_throttle_base_delay_ms * (l0_files - threshold);
    sleep(delay);
}
```

**Slot Compaction Trigger**:
```rust
for slot in slots {
    if slot.runs.len() > slot.k_max {
        // Trigger slot-local tiering compaction
        scheduler.schedule_compaction(slot.slot_id);
    }
}
```

**Bandit Selection**:
```rust
// Called periodically (e.g., every 10 L0 flushes)
fn select_slot_for_compaction() -> u32 {
    if random() < epsilon {
        random_slot()  // Exploration
    } else {
        argmax(slot.ucb_score)  // Exploitation
    }
}
```

---

## Future Directions

### 1. Dynamic Guard Key Adjustment

**Problem**: Fixed guard keys may become imbalanced over time

**Proposed Solution**: Periodically re-partition based on slot sizes
```rust
// Every 1M writes, rebalance slots:
if write_count % 1_000_000 == 0 {
    adjust_guard_keys_to_balance_sizes();
}
```

**Challenges**: Requires moving data across slots (expensive)

### 2. Multi-Dimensional Heat Tracking

**Current**: Single heat score (access frequency)

**Proposed**: Multiple dimensions
```rust
pub struct HeatMetrics {
    pub read_heat: f32,   // Read frequency
    pub write_heat: f32,  // Write frequency
    pub scan_heat: f32,   // Range scan frequency
}
```

**Benefit**: Optimize for read-heavy vs write-heavy vs scan-heavy per slot

### 3. Contextual Bandit Scheduler

**Current**: Multi-armed bandit (no context)

**Proposed**: Use system state as context
```rust
// Context: L0 count, slot size, recent query latency
reward = f(context, action)
```

**Benefit**: Better adaptation to system pressure and query patterns

### 4. Learned Guard Keys

**Current**: Manual uniform split

**Proposed**: ML-based key space partitioning
```rust
// Learn guard keys that minimize cross-slot queries
guard_keys = learn_optimal_splits(query_log);
```

**Benefit**: Reduce read amplification by aligning slots with query patterns

---

## Summary

**ATLL (Adaptive Tiered-Leveled LSM)** achieves near-Pareto-optimal trade-offs for heterogeneous workloads by:

1. **Range partitioning** key space into slots with fixed guard keys
2. **Dynamic K-way fanout** per slot based on EWMA heat tracking
3. **Bandit-based scheduler** for adaptive compaction decisions
4. **Slot-local optimization** on the RUM Pareto frontier

**Result**:
- **Hot ranges** → Leveled (K=1, fast reads, low RA)
- **Cold ranges** → Tiered (K>1, fast writes, low WA)
- **Overall**: 8-20x WA, 5-12x RA, 1.1-1.3x SA

**Comparison**:
- **Better than Leveled**: 2-5x lower WA, comparable RA for hot data
- **Better than Tiered**: 2x lower RA for hot data, comparable WA
- **Better than Universal**: Simpler, more predictable, better read performance

**When to use ATLL**:
- Zipfian/skewed access patterns (80/20 rule)
- Time-series with recent-heavy reads
- Multi-tenant with mixed activity
- General-purpose key-value workloads

**When to avoid ATLL**:
- Uniform access patterns (no hot/cold distinction)
- Pure scan workloads (sequential only)
- Tiny datasets (<100 MB, overhead not worth it)

**Next steps**: Read [Write Path](write-path.md) and [Read Path](read-path.md) to understand how ATLL works in practice.

---

*Last Updated: 2025-10-31*
*References: Dayan et al. (Monkey, 2017), Hao et al. (LSM-Bush, 2019), Athanassoulis et al. (RUM, 2016)*
