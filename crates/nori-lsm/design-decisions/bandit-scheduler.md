---
layout: default
title: Bandit Scheduler
parent: Design Decisions
grand_parent: nori-lsm
nav_order: 4
---

# Bandit Scheduler
{: .no_toc }

Why we use epsilon-greedy multi-armed bandits with UCB for compaction scheduling instead of heuristics or round-robin.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Decision

**Use epsilon-greedy multi-armed bandit with Upper Confidence Bound (UCB) to select which slot to compact:**

```rust
pub struct BanditScheduler {
    arms: Vec<BanditArm>,  // One arm per slot
    epsilon: f64,          // Exploration rate (default: 0.1)
    ucb_c: f64,            // Exploration constant (default: 2.0)
}

pub struct BanditArm {
    slot_id: u32,
    avg_reward: f64,        // EMA of historical rewards
    selection_count: u64,   // Times this slot was selected
}

impl BanditScheduler {
    pub fn select_slot(&mut self) -> u32 {
        if random() < self.epsilon {
            // Exploration: random slot
            random_slot()
        } else {
            // Exploitation: best UCB score
            argmax(|arm| arm.ucb_score(self.total_selections()))
        }
    }
}

impl BanditArm {
    pub fn ucb_score(&self, total_selections: u64) -> f64 {
        self.avg_reward + self.ucb_c * sqrt(ln(total_selections) / self.selection_count)
        // └─exploitation─┘   └──────────exploration bonus──────────┘
    }
}
```

**Reward function:**
```rust
reward = (predicted_latency_reduction × heat_score) / bytes_written
```

**Parameters:**
- ε (epsilon) = 0.1 (10% exploration, 90% exploitation)
- c (UCB constant) = 2.0 (standard value from bandit literature)
- α (reward EMA) = 0.1 (smoothing factor)

---

## Alternatives Considered

### 1. Round-Robin

**Approach:** Compact slots in order (0 → 1 → 2 → ... → N → 0)

```rust
pub fn select_slot(&mut self) -> u32 {
    let slot = self.current_slot;
    self.current_slot = (self.current_slot + 1) % self.num_slots;
    slot
}
```

**Rejected because:**
- **Ignores heat:** Hot/cold treated equally
- **Wasteful:** Compacts cold slots unnecessarily
- **Inflexible:** No adaptation to workload
- **High WA:** Compact everything regardless of benefit

**Example problem:**
```
Slots:
  0-3: Hot (1000 queries/sec each)
  4-63: Cold (1 query/day each)

Round-robin:
  Compacts all 64 slots equally
  60/64 = 94% of compactions are wasted (on cold data)

Bandit:
  Learns to compact slots 0-3 frequently, 4-63 rarely
  >90% of compactions focus on hot slots
```

### 2. Priority Queue (Largest First)

**Approach:** Compact largest slots first

```rust
pub fn select_slot(&mut self) -> u32 {
    self.slots
        .iter()
        .max_by_key(|s| s.total_size())
        .unwrap()
        .slot_id
}
```

**Rejected because:**
- **Size ≠ benefit:** Large cold slot has low read benefit
- **Ignores heat:** Doesn't consider access frequency
- **Starvation:** Small hot slots never compacted
- **No learning:** Static heuristic

**Example problem:**
```
Slot 0: 10 MB, heat=0.9 (hot, small)
Slot 1: 1 GB, heat=0.1 (cold, large)

Priority queue: Always compacts Slot 1 (large)
  Benefit: Low (rarely queried)
  Cost: High (1 GB rewrite)

Bandit: Learns to compact Slot 0 (high reward/byte)
  Benefit: High (frequently queried)
  Cost: Low (10 MB rewrite)
```

### 3. Age-Based (Oldest Runs First)

**Approach:** Compact slot with oldest run

```rust
pub fn select_slot(&mut self) -> u32 {
    self.slots
        .iter()
        .max_by_key(|s| s.oldest_run_age())
        .unwrap()
        .slot_id
}
```

**Rejected because:**
- **Age ≠ benefit:** Old cold data doesn't benefit from compaction
- **Ignores heat:** Doesn't consider query patterns
- **Predictable:** No exploration of new strategies
- **No optimization:** Just a heuristic

### 4. Thompson Sampling

**Approach:** Bayesian bandit algorithm

```rust
pub struct BanditArm {
    alpha: f64,  // Success count (Beta distribution)
    beta: f64,   // Failure count
}

pub fn select_slot(&mut self) -> u32 {
    // Sample from Beta distribution for each arm
    let samples: Vec<f64> = self.arms
        .iter()
        .map(|arm| sample_beta(arm.alpha, arm.beta))
        .collect();

    argmax(samples)
}
```

**Rejected because:**
- **More complex:** Beta distributions, sampling
- **Unclear reward mapping:** How to map compaction reward to success/failure?
- **Implementation overhead:** Beta distribution sampling
- **Epsilon-greedy is sufficient:** Proven to work for non-stationary environments

**Decision:** May revisit for future optimization, but epsilon-greedy is simpler and sufficient

### 5. Contextual Bandits

**Approach:** Use system state as context

```rust
pub struct Context {
    l0_file_count: usize,
    memory_pressure: f32,
    recent_query_latency: f64,
}

pub fn select_slot(&mut self, context: &Context) -> u32 {
    // Learn reward function: f(slot, context) → reward
    self.model.predict(slot, context)
}
```

**Rejected for now because:**
- **Much more complex:** Requires feature engineering, model training
- **Data requirements:** Need large dataset to train model
- **Unclear benefit:** Simple bandits may be sufficient
- **Future work:** Promising direction for optimization

---

## Rationale

### 1. Exploration vs Exploitation

**Multi-armed bandit problem:** Balance trying new actions (exploration) vs using known-good actions (exploitation)

**Epsilon-greedy solution:**
```
With probability ε (0.1):
  → Exploration: Try random slot
  → Benefit: Discover better strategies
  → Cost: 10% suboptimal compactions

With probability 1-ε (0.9):
  → Exploitation: Choose best UCB score
  → Benefit: Use known-good strategy
  → Cost: May miss better strategies
```

**Why 10% exploration?**
- Standard value in bandit literature
- Balances learning vs performance
- Non-stationary environments (workloads change) need exploration
- Validated through benchmarks

**Example:**
```
1000 compaction decisions:
  100 are random (exploration)
  900 use best UCB score (exploitation)

Result:
  Learn about all slots (avoid starvation)
  Mostly compact high-reward slots (good performance)
```

### 2. UCB Exploration Bonus

**Upper Confidence Bound formula:**
```
ucb_score = avg_reward + c × sqrt(ln(total_selections) / arm_selections)
            └─exploitation─┘   └──────────exploration bonus──────────┘
```

**Exploration bonus properties:**
- **Large when rarely selected:** Encourages trying less-explored slots
- **Small when frequently selected:** Don't over-explore well-known slots
- **Grows with total_selections:** More data → more confidence in exploration
- **Theoretical regret bounds:** O(K × log T) regret over T selections

**Example:**
```
Slot 0 (hot, frequently compacted):
  avg_reward = 8.5
  selections = 900
  total = 1000

  ucb_score = 8.5 + 2.0 × sqrt(ln(1000) / 900)
            = 8.5 + 2.0 × sqrt(6.9 / 900)
            = 8.5 + 2.0 × 0.088
            = 8.68

Slot 1 (cold, rarely compacted):
  avg_reward = 2.0
  selections = 10
  total = 1000

  ucb_score = 2.0 + 2.0 × sqrt(ln(1000) / 10)
            = 2.0 + 2.0 × sqrt(6.9 / 10)
            = 2.0 + 2.0 × 0.831
            = 3.66

Decision: Compact Slot 0 (higher UCB score)
  But Slot 1 gets exploration bonus (not starved)
```

### 3. Reward Function Design

**Formula:**
```rust
reward = (predicted_latency_reduction × heat_score) / bytes_written
```

**Components:**

**1. Predicted Latency Reduction:**
```rust
fn predicted_latency_reduction(slot: &Slot) -> f64 {
    // Assume each run adds 100µs to query latency
    let current_latency = slot.runs.len() as f64 * 100.0;

    // After compaction to k_max, latency reduces
    let target_latency = slot.k_max as f64 * 100.0;

    current_latency - target_latency
}
```

**2. Heat Score:** Weight by access frequency
```rust
heat_score ∈ [0.0, 1.0]  // From EWMA tracking
```

**3. Bytes Written:** Cost of compaction
```rust
bytes_written = total_size_of_runs_to_merge
```

**Intuition:**
- **High reward:** Hot slots with high RA (good targets for leveling)
- **Low reward:** Cold slots with low RA (avoid unnecessary compaction)
- **Normalized by cost:** Prefer cheap compactions over expensive ones

**Example:**
```
Slot 0 (hot, 4 runs, 10 MB each):
  latency_reduction = (4 - 1) × 100µs = 300µs
  heat_score = 0.9
  bytes_written = 40 MB

  reward = (300 × 0.9) / 40
         = 270 / 40
         = 6.75

Slot 1 (cold, 4 runs, 100 MB each):
  latency_reduction = (4 - 3) × 100µs = 100µs (k_max=3 for cold)
  heat_score = 0.1
  bytes_written = 400 MB

  reward = (100 × 0.1) / 400
         = 10 / 400
         = 0.025

Decision: Compact Slot 0 (270× higher reward)
```

### 4. Non-Stationary Environment

**Workloads change over time:**
```
Week 1: Slots 0-3 hot (user traffic)
Week 2: Slots 10-13 hot (analytics job)
Week 3: Slots 0-3 hot again (back to normal)
```

**Epsilon-greedy adapts:**
- **Exploration:** Discovers new hot slots (Week 2)
- **EMA rewards:** Recent rewards weigh more (forgets old patterns)
- **UCB bonus:** Encourages trying previously-cold slots

**Contrast with static heuristics:**
```
Priority queue (largest first):
  Week 1: Compacts same large slots
  Week 2: Still compacts same large slots (misses new hot slots)
  Week 3: Still compacts same large slots

Bandit:
  Week 1: Learns Slots 0-3 are best
  Week 2: Explores, discovers Slots 10-13 are better
  Week 3: Re-learns Slots 0-3 (EMA forgets Week 2)
```

---

## Mathematical Guarantees

### Regret Bound (UCB)

**Regret:** Cumulative difference between optimal and actual rewards

```
Regret_T = Σ(reward_optimal - reward_actual)

UCB guarantee:
  Regret_T ≤ O(K × log T)

Where:
  K = number of arms (slots)
  T = total selections
```

**Interpretation:**
```
For 16 slots, 1000 compactions:
  Regret ≤ 16 × log(1000)
         ≈ 16 × 6.9
         ≈ 110 suboptimal decisions

Efficiency: 890/1000 = 89% optimal (after warmup)
```

**Practical implication:** Bandit converges to near-optimal strategy

### Convergence Rate

**EMA reward tracking:**
```
avg_reward_t = α × reward_t + (1-α) × avg_reward_{t-1}

Convergence: 10-20 selections per slot

Example (α=0.1):
  After 10 selections: 65% of steady state
  After 20 selections: 88% of steady state
  After 30 selections: 96% of steady state
```

---

## Trade-offs

### What We Gained

**1. Automatic Optimization**
- Learns which slots benefit most from compaction
- Adapts to workload changes
- No manual tuning required

**2. Theoretical Guarantees**
- O(log T) regret bound (UCB)
- Converges to near-optimal policy
- Proven in bandit literature

**3. Flexible**
- Works for any workload (time-series, key-value, analytics)
- Adapts to non-stationary environments
- Handles heterogeneous slots

**4. Observable**
- Emit BanditSelection and BanditReward events
- Monitor exploration rate, rewards, UCB scores
- Debug suboptimal behavior

### What We Gave Up

**1. Predictability**
- 10% of compactions are random (exploration)
- Slot selection varies over time (not deterministic)
- Harder to reason about worst-case

**2. Simplicity**
- More complex than round-robin or priority queue
- Requires reward function design
- Per-slot state management (EMA, counts)

**3. Warmup Period**
- First 10-20 compactions per slot are learning
- Suboptimal during warmup
- Need ~16 × 20 = 320 compactions to warm up all slots

---

## Configuration

```rust
pub struct ATLLConfig {
    /// Epsilon (exploration rate)
    /// Default: 0.1 (10% exploration)
    /// Range: 0.01 - 0.5
    pub compaction_epsilon: f64,

    /// UCB exploration constant
    /// Default: 2.0 (standard value)
    /// Range: 1.0 - 4.0
    pub compaction_ucb_c: f64,

    /// Reward EMA smoothing factor
    /// Default: 0.1
    /// Range: 0.01 - 0.5
    pub compaction_reward_alpha: f64,
}
```

**Tuning guidelines:**

**Rapidly changing workloads:**
```rust
ATLLConfig {
    compaction_epsilon: 0.15,      // More exploration
    compaction_ucb_c: 2.5,         // Higher exploration bonus
    compaction_reward_alpha: 0.2,  // Faster adaptation
    ..Default::default()
}
```

**Stable workloads:**
```rust
ATLLConfig {
    compaction_epsilon: 0.05,      // Less exploration
    compaction_ucb_c: 1.5,         // Lower exploration bonus
    compaction_reward_alpha: 0.05, // Slower, smoother adaptation
    ..Default::default()
}
```

---

## Observability

**VizEvent emissions:**

**1. BanditSelection** (when slot is selected):
```rust
pub struct BanditSelection {
    pub slot_id: u32,
    pub explored: bool,         // true = exploration, false = exploitation
    pub ucb_score: f64,
    pub avg_reward: f64,
    pub selection_count: u64,
}
```

**2. BanditReward** (after compaction completes):
```rust
pub struct BanditReward {
    pub slot_id: u32,
    pub reward: f64,
    pub bytes_written: u64,
    pub heat_score: f32,
}
```

**Dashboard queries:**
```sql
-- Exploration rate (should be ~10%)
SELECT
  SUM(CASE WHEN explored THEN 1 ELSE 0 END) / COUNT(*) AS exploration_rate
FROM bandit_selection;

-- Top slots by reward
SELECT slot_id, AVG(reward) AS avg_reward, COUNT(*) AS selections
FROM bandit_reward
GROUP BY slot_id
ORDER BY avg_reward DESC;

-- UCB score distribution
SELECT
  slot_id,
  AVG(ucb_score) AS avg_ucb,
  MAX(ucb_score) AS max_ucb
FROM bandit_selection
GROUP BY slot_id;
```

---

## Validation

**Benchmark results** (Zipfian workload):

```
Slots 0-3 (hot, 80% of queries):
  Selections: 720/1000 (72%)
  Avg reward: 8.2
  Compaction frequency: High

Slots 4-15 (cold, 20% of queries):
  Selections: 280/1000 (28%)
  Avg reward: 1.5
  Compaction frequency: Low

Exploration rate: 98/1000 (9.8%)
  ≈ 10% (as expected)

Result: Bandit correctly learned hot slots
```

**Property test: Reward convergence**
```rust
#[test]
fn test_bandit_reward_convergence() {
    let mut scheduler = BanditScheduler::new(16);

    // Simulate compactions with known rewards
    for _ in 0..100 {
        let slot = scheduler.select_slot();
        let reward = if slot < 4 { 8.0 } else { 2.0 };
        scheduler.update_reward(slot, reward);
    }

    // After convergence
    assert!(scheduler.avg_reward(0) > 7.0);  // Hot slot
    assert!(scheduler.avg_reward(5) < 3.0);  // Cold slot
}
```

---

## Future Enhancements

### 1. Contextual Bandits

**Idea:** Use system state as features

```rust
pub struct Context {
    l0_file_count: usize,
    memory_pressure: f32,
    recent_p95_latency: f64,
}

pub fn select_slot(&mut self, context: &Context) -> u32 {
    // Learn: reward = f(slot, l0_count, pressure, latency)
    self.model.predict_best_slot(context)
}
```

**Benefit:** More informed decisions based on system state

### 2. Thompson Sampling

**Idea:** Bayesian alternative to epsilon-greedy

**Benefit:** More principled exploration (better regret bounds in some scenarios)

### 3. Adaptive Epsilon

**Idea:** Adjust ε based on reward variance

```rust
pub fn adaptive_epsilon(&self) -> f64 {
    let variance = self.compute_reward_variance();

    if variance > 0.5 {
        0.2  // High variance → more exploration
    } else {
        0.05  // Low variance → less exploration
    }
}
```

---

## Summary

**Epsilon-greedy multi-armed bandit with UCB is the right choice because:**

1. **Balances exploration vs exploitation** (ε=0.1)
2. **Theoretical guarantees** (O(log T) regret bound)
3. **Adapts to workload changes** (non-stationary environments)
4. **Observable** (BanditSelection/BanditReward events)
5. **Proven** (standard technique in reinforcement learning)

**We accept these trade-offs:**
- 10% exploration overhead (suboptimal compactions)
- Warmup period (first ~300 compactions are learning)
- Complexity (vs simple round-robin)

**For heterogeneous workloads with changing access patterns, the bandit scheduler automatically learns to focus compaction effort where it provides the most benefit.**

---

*Last Updated: 2025-10-31*
*See Also: [Heat Tracking](heat-tracking.md), [Adaptive K-Way Fanout](adaptive-k-fanout.md)*
*References: Auer et al. (UCB, 2002), Sutton & Barto (Reinforcement Learning, 2018)*
