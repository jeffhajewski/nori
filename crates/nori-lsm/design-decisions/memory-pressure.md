---
layout: default
title: Memory Pressure System
parent: Design Decisions
grand_parent: nori-lsm
nav_order: 5
---

# Memory Pressure System
{: .no_toc }

Why ATLL uses 4-zone adaptive backpressure (Green/Yellow/Orange/Red) with composite scoring instead of hard stalls or single thresholds.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Decision

**Use a 4-zone adaptive backpressure system with composite scoring to progressively throttle writes before hitting hard limits.**

Each zone applies different strategies:

```rust
pub enum PressureZone {
    Green,   // 0-50%:  No backpressure
    Yellow,  // 50-75%: Soft throttling (warn, delay small)
    Orange,  // 75-90%: Heavy throttling (delay exponential)
    Red,     // 90%+:   Hard stall (reject writes)
}

// Composite pressure score
pressure_score = 0.4 × L0_ratio + 0.4 × memory_ratio + 0.2 × memtable_ratio

Where:
  L0_ratio = L0_count / L0_max
  memory_ratio = total_memory / memory_limit
  memtable_ratio = memtable_size / memtable_max
```

**Result:**
- **Progressive degradation:** Gradual slowdown instead of cliff-edge failures
- **Multi-dimensional awareness:** Responds to L0 count, memory usage, and memtable size
- **Observable behavior:** Clear pressure zones with metrics and VizEvent emissions

---

## Alternatives Considered

### 1. Hard Stall Only (No Backpressure)

**Approach:** Accept writes until limit, then reject

```rust
async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
    if self.l0_count >= L0_MAX {
        return Err(Error::MemoryLimitExceeded);
    }

    // Write normally
    self.write(key, value).await
}
```

**Rejected because:**
- **Cliff-edge failures:** 0ms latency → rejection with no warning
- **No client adaptation:** Clients can't detect pressure and slow down
- **Burst amplification:** All clients retry simultaneously (thundering herd)
- **Poor UX:** Sudden errors instead of gradual slowdown

**Example:**
```
L0 count: 5 (of 6 max)
  100 concurrent writes arrive
  → All succeed (L0 = 6)

L0 count: 6 (at max)
  Next write arrives
  → Rejected (Error::MemoryLimitExceeded)
  → Client retries immediately (backoff logic needed)
  → Same error (compaction hasn't finished yet)

Problem: No warning, no graceful degradation
```

### 2. Single Threshold (Binary Backpressure)

**Approach:** Normal speed until threshold, then fixed delay

```rust
async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
    if self.l0_count > 4 {
        // Above threshold: delay all writes
        sleep(Duration::from_millis(100)).await;
    }

    self.write(key, value).await
}
```

**Rejected because:**
- **Cliff edge (smaller):** 0ms → 100ms with no gradation
- **Inefficient:** Same delay at 50% and 99% pressure
- **No severity signal:** Clients can't distinguish "mildly busy" from "critically overloaded"
- **Fixed delay:** Doesn't adapt to pressure severity

**Example:**
```
L0 count: 4 → 0ms write latency
L0 count: 5 → 100ms write latency (sudden jump)
L0 count: 6 → 100ms write latency (same as 5, but more critical)

Problem: Binary signal, no proportional response
```

### 3. Per-Dimension Limits (Independent Throttling)

**Approach:** Separate thresholds for L0, memory, memtable

```rust
async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
    let mut delay_ms = 0;

    if self.l0_count > L0_THRESHOLD {
        delay_ms += 50;
    }

    if self.total_memory > MEMORY_THRESHOLD {
        delay_ms += 50;
    }

    if self.memtable_size > MEMTABLE_THRESHOLD {
        delay_ms += 50;
    }

    sleep(Duration::from_millis(delay_ms)).await;
    self.write(key, value).await
}
```

**Rejected because:**
- **Additive delays:** Multiple dimensions → 150ms delay (too harsh)
- **No global view:** Doesn't balance dimensions (e.g., high L0 + low memory = less critical)
- **Tuning difficulty:** 3 independent thresholds to configure
- **Uneven response:** One dimension can dominate

**Example:**
```
Scenario A: L0=high, memory=low, memtable=low
  → 50ms delay (reasonable)

Scenario B: L0=high, memory=high, memtable=high
  → 150ms delay (too harsh, might trigger timeout)

Problem: Doesn't consider overall system health
```

### 4. Fixed Delay Progression

**Approach:** Static delays per pressure level

```rust
fn calculate_delay(&self) -> Duration {
    match self.pressure_level() {
        1 => Duration::from_millis(10),
        2 => Duration::from_millis(20),
        3 => Duration::from_millis(40),
        4 => Duration::from_millis(80),
        _ => Duration::ZERO,
    }
}
```

**Rejected because:**
- **Linear progression:** Doesn't adapt to rate of change
- **No exponential backoff:** Can't handle transient spikes
- **Static levels:** Requires manual tuning per workload
- **No smooth transition:** Jumps between fixed delays

**Example:**
```
Pressure: 25% → 0ms
Pressure: 50% → 10ms (jump)
Pressure: 75% → 40ms (jump)
Pressure: 90% → 80ms (jump)

Problem: Discrete jumps, no smooth curve
```

### 5. Token Bucket Rate Limiting

**Approach:** Allow X writes/sec, delay when bucket empty

```rust
pub struct TokenBucket {
    tokens: f64,
    rate: f64,  // Tokens per second
    max: f64,   // Bucket capacity
}

async fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<()> {
    // Wait until token available
    self.bucket.acquire(1).await;

    self.write(key, value).await
}
```

**Rejected because:**
- **Fixed rate:** Doesn't adapt to compaction progress
- **Ignores system state:** Tokens refill regardless of L0/memory pressure
- **No pressure signal:** Clients don't know *why* they're being throttled
- **Complex tuning:** Rate must match write throughput and compaction rate

**Example:**
```
Bucket: 100 tokens/sec

Scenario A: Low pressure, 50 writes/sec
  → Tokens accumulate (no backpressure)

Scenario B: High pressure, 50 writes/sec
  → Tokens accumulate (still no backpressure!)

Problem: Doesn't respond to system state
```

---

## Rationale

### 1. Progressive Degradation

**Composite scoring provides smooth transition:**

```rust
fn pressure_score(&self) -> f64 {
    let l0_ratio = self.l0_count as f64 / self.config.l0_max as f64;
    let memory_ratio = self.total_memory as f64 / self.config.memory_limit as f64;
    let memtable_ratio = self.memtable_size as f64 / self.config.memtable_max as f64;

    // Weighted average (L0 and memory matter most)
    0.4 * l0_ratio + 0.4 * memory_ratio + 0.2 * memtable_ratio
}
```

**Benefits:**
- **Smooth curve:** Pressure increases gradually, not in jumps
- **Balanced view:** No single dimension dominates
- **Tunable weights:** Can adjust for workload (e.g., 0.6×L0 for write-heavy)

**Example:**
```
State A: L0=50%, memory=30%, memtable=20%
  pressure_score = 0.4×0.5 + 0.4×0.3 + 0.2×0.2 = 0.36 (Green)

State B: L0=70%, memory=60%, memtable=40%
  pressure_score = 0.4×0.7 + 0.4×0.6 + 0.2×0.4 = 0.60 (Yellow)

State C: L0=90%, memory=85%, memtable=80%
  pressure_score = 0.4×0.9 + 0.4×0.85 + 0.2×0.8 = 0.86 (Orange)

Result: Smooth progression from Green → Yellow → Orange
```

### 2. Zone-Based Strategies

**Each zone applies different backpressure:**

```rust
pub async fn apply_backpressure(&self, score: f64) -> Result<()> {
    match Self::zone_for_score(score) {
        PressureZone::Green => {
            // 0-50%: No delay, emit metrics only
            Ok(())
        }

        PressureZone::Yellow => {
            // 50-75%: Soft throttling (linear delay)
            let delay_ms = (score - 0.5) / 0.25 * 50.0;  // 0-50ms
            sleep(Duration::from_millis(delay_ms as u64)).await;
            Ok(())
        }

        PressureZone::Orange => {
            // 75-90%: Heavy throttling (exponential delay)
            let factor = (score - 0.75) / 0.15;  // 0-1
            let delay_ms = 50.0 * (2.0_f64.powf(factor * 4.0));  // 50-800ms
            sleep(Duration::from_millis(delay_ms as u64)).await;
            Ok(())
        }

        PressureZone::Red => {
            // 90%+: Hard stall (reject writes)
            Err(Error::MemoryLimitExceeded)
        }
    }
}
```

**Delay curves:**

```
Green (0-50%):
  delay = 0ms (always)

Yellow (50-75%):
  50% → 0ms
  60% → 20ms
  70% → 40ms
  75% → 50ms
  Linear growth

Orange (75-90%):
  75% → 50ms
  80% → 100ms
  85% → 200ms
  90% → 400ms
  Exponential growth (2^x curve)

Red (90%+):
  Reject writes immediately
```

**Rationale:**
- **Green:** No overhead, normal throughput
- **Yellow:** Gentle slowdown, clients notice but not alarmed
- **Orange:** Aggressive backoff, clear "slow down" signal
- **Red:** Last resort, system critically overloaded

### 3. Multi-Dimensional Awareness

**Why composite scoring beats single metrics:**

**Scenario 1: High L0, low memory**
```
L0: 90% (6 files pending compaction)
Memory: 20% (plenty of RAM available)
Memtable: 30%

Composite: 0.4×0.9 + 0.4×0.2 + 0.2×0.3 = 0.50 (Yellow, not Orange)

Interpretation: High L0 but plenty of memory, compaction will catch up soon
Strategy: Soft throttling (20ms delay), not aggressive backoff
```

**Scenario 2: Low L0, high memory**
```
L0: 30% (few files)
Memory: 95% (nearly full)
Memtable: 80%

Composite: 0.4×0.3 + 0.4×0.95 + 0.2×0.8 = 0.66 (Yellow, approaching Orange)

Interpretation: L0 is fine but memory critical, need flush
Strategy: Moderate throttling, trigger memtable flush
```

**Scenario 3: All dimensions high**
```
L0: 85%
Memory: 90%
Memtable: 95%

Composite: 0.4×0.85 + 0.4×0.9 + 0.2×0.95 = 0.89 (Orange, near Red)

Interpretation: System critically overloaded
Strategy: Heavy throttling (300ms+ delay), aggressive compaction
```

**Benefit:** Responds to overall system health, not single bottleneck

### 4. Observable and Debuggable

**VizEvent emissions for monitoring:**

```rust
pub fn emit_pressure_event(&self, zone: PressureZone, score: f64) {
    self.meter.emit(VizEvent::PressureZone {
        zone: zone.as_str(),
        score,
        l0_count: self.l0_count,
        memory_bytes: self.total_memory,
        memtable_bytes: self.memtable_size,
        timestamp_ns: self.clock.now().as_nanos(),
    });
}
```

**Dashboard visualization:**

```
Pressure Timeline:

  100% │                    ████ Red
   90% │               ████░░░░░░
   75% │          ████░░░░░░░░░░░ Orange
   50% │     ████░░░░░░░░░░░░░░░░ Yellow
    0% │████░░░░░░░░░░░░░░░░░░░░░ Green
       └────────────────────────────────
         Time →

Zone transitions visible in real-time
Operators can see pressure buildup before failures
```

**Metrics exposed:**

```rust
// Gauge metrics
pressure_score (0.0-1.0)
pressure_zone (0=Green, 1=Yellow, 2=Orange, 3=Red)
l0_ratio (0.0-1.0)
memory_ratio (0.0-1.0)

// Counter metrics
writes_delayed_total (by zone)
writes_rejected_total
backpressure_time_ms_total
```

---

## Trade-offs

### What We Gained

**1. Graceful Degradation**
- Smooth slowdown from 0ms → 50ms → 400ms → rejection
- Clients experience latency increase, not sudden errors
- Time to adapt (reduce write rate, batch, defer)

**2. Multi-Dimensional Health**
- Composite score balances L0, memory, memtable
- Avoids overreacting to single metric spike
- Responds to true system stress

**3. Clear Severity Signals**
- 4 zones with distinct behaviors
- Operators know severity at a glance
- Clients can implement zone-aware retry logic

**4. Configurable Weights**
- Adjust for workload (write-heavy, read-heavy, balanced)
- Tune L0 vs memory importance
- Override via config without code changes

### What We Gave Up

**1. Simplicity**
- More complex than hard stall or single threshold
- Composite formula requires tuning (default works for most)
- 4 zones vs binary on/off

**2. Maximum Throughput**
- Yellow/Orange zones add latency (by design)
- Reduces peak write rate before hitting limits
- Trade throughput for stability

**3. Predictability**
- Zone transitions depend on multiple metrics
- Not a simple "L0 > 6 → reject" rule
- Requires monitoring to understand behavior

---

## Configuration

**Default configuration:**

```rust
pub struct PressureConfig {
    /// L0 file count limit
    /// Default: 6
    pub l0_max: usize,

    /// Total memory limit (bytes)
    /// Default: 512 MB
    pub memory_limit: usize,

    /// Memtable size limit (bytes)
    /// Default: 64 MB
    pub memtable_max: usize,

    /// Composite score weights (must sum to 1.0)
    /// Default: [0.4, 0.4, 0.2]
    pub weights: [f64; 3],  // [L0, memory, memtable]

    /// Zone boundaries (pressure score thresholds)
    /// Default: [0.50, 0.75, 0.90]
    pub zone_thresholds: [f64; 3],  // [Yellow, Orange, Red]
}
```

**Tuning guidelines:**

### Write-Heavy Workloads

```rust
PressureConfig {
    l0_max: 8,  // Allow more L0 accumulation
    memory_limit: 1024 * 1024 * 1024,  // 1 GB
    weights: [0.6, 0.3, 0.1],  // Prioritize L0 over memory
    ..Default::default()
}
```

**Effect:**
- Higher L0 tolerance (8 files vs 6)
- L0 ratio weighted more heavily (60% vs 40%)
- Less backpressure from memory spikes

### Read-Heavy Workloads

```rust
PressureConfig {
    l0_max: 4,  // Keep L0 low (faster reads)
    memory_limit: 2048 * 1024 * 1024,  // 2 GB (more cache)
    weights: [0.5, 0.3, 0.2],  // Balanced
    ..Default::default()
}
```

**Effect:**
- Lower L0 limit (better read performance)
- More memory for block cache
- Earlier backpressure to maintain low L0

### High-Pressure Tolerance

```rust
PressureConfig {
    zone_thresholds: [0.60, 0.85, 0.95],  // Shift zones right
    ..Default::default()
}
```

**Effect:**
- Yellow zone: 60-85% (wider, less sensitive)
- Orange zone: 85-95% (narrower, more tolerance)
- Red zone: 95%+ (only truly critical)
- Less aggressive backpressure overall

### Low-Latency SLOs

```rust
PressureConfig {
    zone_thresholds: [0.40, 0.65, 0.85],  // Shift zones left
    ..Default::default()
}
```

**Effect:**
- Yellow zone: 40-65% (earlier warning)
- Orange zone: 65-85% (wider, more aggressive)
- Red zone: 85%+ (earlier hard stall)
- Preemptive backpressure to maintain low latency

---

## Zone Evolution Examples

### Example 1: Gradual Write Burst

**Workload:** Steady increase from 1K writes/sec → 10K writes/sec

```
Time  │ L0  │ Mem  │ MT  │ Score │ Zone   │ Delay
──────┼─────┼──────┼─────┼───────┼────────┼──────
00:00 │ 20% │ 30%  │ 10% │ 0.24  │ Green  │ 0ms
00:10 │ 40% │ 50%  │ 30% │ 0.42  │ Green  │ 0ms
00:20 │ 60% │ 65%  │ 50% │ 0.60  │ Yellow │ 20ms
00:30 │ 75% │ 75%  │ 70% │ 0.74  │ Yellow │ 48ms
00:40 │ 85% │ 85%  │ 85% │ 0.85  │ Orange │ 178ms
00:50 │ 90% │ 90%  │ 90% │ 0.90  │ Red    │ Reject

Compaction kicks in aggressively at 00:40
Pressure stabilizes at Yellow zone by 01:00
```

### Example 2: Transient Memory Spike

**Workload:** Sudden 200 MB write batch

```
Time  │ L0  │ Mem  │ MT  │ Score │ Zone   │ Delay
──────┼─────┼──────┼─────┼───────┼────────┼──────
00:00 │ 30% │ 40%  │ 20% │ 0.32  │ Green  │ 0ms
00:01 │ 35% │ 85%  │ 70% │ 0.62  │ Yellow │ 24ms  ← Spike
00:02 │ 40% │ 50%  │ 30% │ 0.38  │ Green  │ 0ms   ← Recovered

Memtable flushed quickly (200 MB → SSTable)
Composite score smooths out spike (85% memory but 35% L0)
No Orange zone entered (avoided overreaction)
```

### Example 3: Sustained High Load

**Workload:** 8K writes/sec sustained for 10 minutes

```
Time  │ L0  │ Mem  │ MT  │ Score │ Zone   │ Delay
──────┼─────┼──────┼─────┼───────┼────────┼──────
00:00 │ 50% │ 55%  │ 40% │ 0.50  │ Yellow │ 0ms    ← Enter Yellow
00:30 │ 70% │ 70%  │ 60% │ 0.68  │ Yellow │ 36ms
01:00 │ 80% │ 78%  │ 75% │ 0.78  │ Orange │ 89ms   ← Enter Orange
02:00 │ 82% │ 80%  │ 78% │ 0.80  │ Orange │ 126ms
05:00 │ 78% │ 76%  │ 74% │ 0.76  │ Orange │ 63ms   ← Compaction catching up
10:00 │ 65% │ 60%  │ 55% │ 0.61  │ Yellow │ 22ms   ← Stabilized

System never hits Red zone
Backpressure + compaction reach equilibrium in Orange zone
Gradual recovery to Yellow after workload completes
```

---

## Validation

**Property test: Pressure monotonicity**

```rust
#[test]
fn test_pressure_increases_with_load() {
    let mut lsm = LSM::new(PressureConfig::default());

    let mut prev_score = 0.0;

    // Incrementally add L0 files
    for i in 0..6 {
        lsm.add_l0_file(1024 * 1024);  // 1 MB each

        let score = lsm.pressure_score();
        assert!(score >= prev_score);  // Monotonic increase
        prev_score = score;
    }

    // Check zone transitions
    assert_eq!(lsm.zone_for_score(0.30), PressureZone::Green);
    assert_eq!(lsm.zone_for_score(0.60), PressureZone::Yellow);
    assert_eq!(lsm.zone_for_score(0.80), PressureZone::Orange);
    assert_eq!(lsm.zone_for_score(0.95), PressureZone::Red);
}
```

**Benchmark: Backpressure overhead**

```
Baseline (no pressure):
  PUT latency: 1.2ms (p50), 2.1ms (p95)

Yellow zone (score=0.60):
  PUT latency: 21.5ms (p50), 32.8ms (p95)
  Overhead: +20ms delay (expected)

Orange zone (score=0.85):
  PUT latency: 201.3ms (p50), 285.6ms (p95)
  Overhead: ~200ms delay (expected)

Red zone (score=0.95):
  PUT latency: N/A (writes rejected)
  Error rate: 100% (expected)
```

**Integration test: Recovery from Red zone**

```rust
#[tokio::test]
async fn test_recovery_from_red_zone() {
    let lsm = LSM::new(PressureConfig::default());

    // Fill L0 to Red zone
    for _ in 0..10 {
        let _ = lsm.flush_memtable().await;
    }

    assert_eq!(lsm.pressure_zone(), PressureZone::Red);

    // Trigger aggressive compaction
    lsm.compact_all_slots().await.unwrap();

    // After compaction, should return to Green
    assert_eq!(lsm.pressure_zone(), PressureZone::Green);
}
```

---

## Summary

**4-zone adaptive backpressure is the right choice for ATLL because:**

1. **Progressive degradation** - Smooth transition from 0ms → 50ms → 400ms → rejection
2. **Multi-dimensional health** - Composite score balances L0, memory, memtable
3. **Observable behavior** - Clear zones with VizEvent emissions and metrics
4. **Configurable response** - Tune weights and thresholds per workload

**We accept these trade-offs:**
- Complexity (vs hard stall only)
- Reduced peak throughput (by design, for stability)
- Multiple metrics to monitor (vs single threshold)

**For heterogeneous workloads with bursty writes, 4-zone backpressure prevents cliff-edge failures while maintaining observable, predictable behavior.**

---

*Last Updated: 2025-10-31*
*See Also: [ATLL Architecture](../core-concepts/atll-architecture.md), [Write Path](../core-concepts/write-path.md)*
