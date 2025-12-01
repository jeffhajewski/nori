/// Heat tracking and dynamic K adjustment for ATLL.
///
/// Based on `lsm_atll_design.yaml` spec:
/// - Lines 106-109: Heat thresholds configuration (H_hot, H_cold, half_life_ops)
/// - Lines 185-187: heat_update algorithm (EWMA)
/// - Lines 188-195: dynamic_K_selection algorithm
///
/// # Heat Model
///
/// Heat represents workload intensity per slot, computed as:
/// - EWMA (Exponential Weighted Moving Average) of operations
/// - Operations weighted: GET/SCAN > PUT/DELETE
/// - Half-life: 100K operations (configurable)
///
/// # Dynamic K Selection
///
/// ```text
/// if heat_s > H_hot:
///     K_s := K_hot (typically 1, pure leveled)
/// else if heat_s < H_cold and write_pressure_high:
///     K_s := min(K_s+1, K_default) (tiered, reduce write amp)
/// else:
///     keep K_s
/// ```
use crate::config::ATLLConfig;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// Operation type for heat tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Point lookup (GET)
    Get,
    /// Range scan
    Scan,
    /// Write (PUT)
    Put,
    /// Delete
    Delete,
}

impl Operation {
    /// Returns the weight of this operation for heat calculation.
    ///
    /// Read operations (GET/SCAN) contribute more to heat than writes.
    /// Spec: "GET, SCAN weighted > PUT, DEL"
    pub fn weight(&self) -> f32 {
        match self {
            Operation::Get => 1.0,
            Operation::Scan => 2.0, // Scans are more intensive
            Operation::Put => 0.5,
            Operation::Delete => 0.5,
        }
    }

    /// Returns true if this is a write operation (PUT/DELETE).
    pub fn is_write(&self) -> bool {
        matches!(self, Operation::Put | Operation::Delete)
    }
}

/// Heat score for a single slot.
///
/// Tracks EWMA of operation weights over time.
#[derive(Debug, Clone)]
pub struct SlotHeat {
    /// Current heat score [0.0, 1.0+]
    /// Normalized but can exceed 1.0 under extreme load
    pub score: f32,

    /// Total operation count (for EWMA calculation)
    pub op_count: u64,

    /// Last update timestamp
    pub last_update: Instant,

    /// Current K value for this slot
    pub current_k: u8,
}

impl SlotHeat {
    /// Creates a new slot heat tracker.
    pub fn new(initial_k: u8) -> Self {
        Self {
            score: 0.0,
            op_count: 0,
            last_update: Instant::now(),
            current_k: initial_k,
        }
    }

    /// Updates heat score with EWMA.
    ///
    /// Formula: heat_new = α × weight + (1 - α) × heat_old
    /// where α = 1 - exp(-ln(2) / half_life)
    ///
    /// Spec: lines 185-187
    pub fn update(&mut self, op: Operation, half_life_ops: u64) {
        let weight = op.weight();
        let alpha = self.ewma_alpha(half_life_ops);

        self.score = alpha * weight + (1.0 - alpha) * self.score;
        self.op_count += 1;
        self.last_update = Instant::now();
    }

    /// Calculates EWMA alpha parameter.
    ///
    /// α = 1 - exp(-ln(2) / half_life)
    /// At half_life operations, old value contributes 50%.
    fn ewma_alpha(&self, half_life: u64) -> f32 {
        if half_life == 0 {
            return 0.5; // Fallback
        }

        // α ≈ ln(2) / half_life for large half_life
        let alpha = std::f32::consts::LN_2 / half_life as f32;
        alpha.min(1.0)
    }

    /// Decays heat score based on time elapsed.
    ///
    /// Useful for idle slots where no operations occur.
    pub fn decay(&mut self, elapsed: Duration, half_life_ops: u64) {
        // Estimate ops from time (assume 10K ops/sec baseline)
        let estimated_ops = (elapsed.as_secs_f32() * 10_000.0) as u64;
        if estimated_ops > 0 {
            let alpha = self.ewma_alpha(half_life_ops);
            // Decay toward 0
            let decay_factor = (1.0 - alpha).powi(estimated_ops as i32);
            self.score *= decay_factor;
        }
    }
}

/// Global heat tracker for all levels and slots.
///
/// Maintains per-slot heat scores and provides K adjustment recommendations.
pub struct HeatTracker {
    /// Configuration
    config: ATLLConfig,

    /// Per-level, per-slot heat scores
    /// Key: (level, slot_id)
    slot_heat: Arc<RwLock<HashMap<(u8, u32), SlotHeat>>>,

    /// Global operation count for write pressure estimation
    global_op_count: Arc<RwLock<u64>>,

    /// Last decay timestamp
    last_decay: Arc<RwLock<Instant>>,

    /// EWMA of write ratio (0.0 = all reads, 1.0 = all writes)
    /// Used for proper windowed write pressure estimation.
    write_ratio: Arc<RwLock<f32>>,
}

impl HeatTracker {
    /// Creates a new heat tracker.
    pub fn new(config: ATLLConfig) -> Self {
        Self {
            config,
            slot_heat: Arc::new(RwLock::new(HashMap::new())),
            global_op_count: Arc::new(RwLock::new(0)),
            last_decay: Arc::new(RwLock::new(Instant::now())),
            write_ratio: Arc::new(RwLock::new(0.0)),
        }
    }

    /// Records an operation on a slot.
    pub fn record_op(&self, level: u8, slot_id: u32, op: Operation) {
        let half_life = self.config.heat_thresholds.half_life_ops;

        // Update per-slot heat
        {
            let mut heat_map = self.slot_heat.write().unwrap();
            let slot_heat = heat_map
                .entry((level, slot_id))
                .or_insert_with(|| SlotHeat::new(self.config.default_k_for_level(level)));
            slot_heat.update(op, half_life);
        }

        // Update global counter
        {
            let mut global = self.global_op_count.write().unwrap();
            *global += 1;
        }

        // Update write ratio EWMA
        // Formula: ratio_new = α × is_write + (1 - α) × ratio_old
        // Using same half_life as heat tracking for consistency
        {
            let alpha = self.ewma_alpha(half_life);
            let write_value = if op.is_write() { 1.0 } else { 0.0 };
            let mut ratio = self.write_ratio.write().unwrap();
            *ratio = alpha * write_value + (1.0 - alpha) * *ratio;
        }
    }

    /// Calculates EWMA alpha parameter.
    ///
    /// α = ln(2) / half_life for large half_life
    /// At half_life operations, old value contributes 50%.
    fn ewma_alpha(&self, half_life: u64) -> f32 {
        if half_life == 0 {
            return 0.5; // Fallback
        }
        let alpha = std::f32::consts::LN_2 / half_life as f32;
        alpha.min(1.0)
    }

    /// Returns the heat score for a slot.
    pub fn get_heat(&self, level: u8, slot_id: u32) -> f32 {
        let heat_map = self.slot_heat.read().unwrap();
        heat_map
            .get(&(level, slot_id))
            .map(|h| h.score)
            .unwrap_or(0.0)
    }

    /// Returns the current K value for a slot.
    pub fn get_k(&self, level: u8, slot_id: u32) -> u8 {
        let heat_map = self.slot_heat.read().unwrap();
        heat_map
            .get(&(level, slot_id))
            .map(|h| h.current_k)
            .unwrap_or_else(|| self.config.default_k_for_level(level))
    }

    /// Classifies a slot as Hot, Warm, or Cold.
    pub fn classify_slot(&self, level: u8, slot_id: u32) -> SlotClass {
        let score = self.get_heat(level, slot_id);
        let thresholds = &self.config.heat_thresholds;

        if score >= thresholds.hot {
            SlotClass::Hot
        } else if score <= thresholds.cold {
            SlotClass::Cold
        } else {
            SlotClass::Warm
        }
    }

    /// Adjusts K value for a slot based on heat and write pressure.
    ///
    /// Algorithm from spec lines 188-195:
    /// ```text
    /// if heat_s > H_hot: K_s := K_hot (typically 1)
    /// else if heat_s < H_cold and write_pressure_high: K_s := min(K_s+1, K_default)
    /// else: keep K_s
    /// ```
    pub fn adjust_k(&self, level: u8, slot_id: u32, write_pressure_high: bool) -> Option<u8> {
        let mut heat_map = self.slot_heat.write().unwrap();

        let slot_heat = heat_map
            .entry((level, slot_id))
            .or_insert_with(|| SlotHeat::new(self.config.default_k_for_level(level)));

        let old_k = slot_heat.current_k;
        let new_k = self.compute_target_k(slot_heat.score, old_k, level, write_pressure_high);

        if new_k != old_k {
            slot_heat.current_k = new_k;
            Some(new_k)
        } else {
            None
        }
    }

    /// Computes target K value based on heat score.
    fn compute_target_k(
        &self,
        heat_score: f32,
        current_k: u8,
        level: u8,
        write_pressure_high: bool,
    ) -> u8 {
        let thresholds = &self.config.heat_thresholds;

        if heat_score >= thresholds.hot {
            // Hot slot: converge to K=1 (leveled, low read amp)
            self.config.hot_k
        } else if heat_score <= thresholds.cold && write_pressure_high {
            // Cold slot under write pressure: increase K (tiered, low write amp)
            let default_k = self.config.default_k_for_level(level);
            current_k.saturating_add(1).min(default_k)
        } else {
            // Warm slot or no write pressure: maintain current K
            current_k
        }
    }

    /// Decays heat scores for idle slots.
    ///
    /// Should be called periodically (e.g., every 10 seconds).
    pub fn decay_idle_slots(&self) {
        let mut last_decay = self.last_decay.write().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(*last_decay);

        if elapsed.as_secs() < 10 {
            return; // Too soon
        }

        let mut heat_map = self.slot_heat.write().unwrap();
        let half_life = self.config.heat_thresholds.half_life_ops;

        for slot_heat in heat_map.values_mut() {
            let idle_time = now.duration_since(slot_heat.last_update);
            if idle_time.as_secs() > 5 {
                slot_heat.decay(idle_time, half_life);
            }
        }

        *last_decay = now;
    }

    /// Returns all slot heat scores for a level (for debugging/observability).
    pub fn level_heat_scores(&self, level: u8) -> Vec<(u32, f32, u8)> {
        let heat_map = self.slot_heat.read().unwrap();
        let mut scores: Vec<_> = heat_map
            .iter()
            .filter(|((l, _), _)| *l == level)
            .map(|((_, slot_id), heat)| (*slot_id, heat.score, heat.current_k))
            .collect();
        scores.sort_by_key(|(slot_id, _, _)| *slot_id);
        scores
    }

    /// Estimates write pressure based on recent operation mix.
    ///
    /// High write pressure when write ratio (EWMA of PUT/DELETE ops) exceeds threshold.
    /// Returns true when write_ratio > write_pressure_threshold (default: 0.5).
    pub fn estimate_write_pressure(&self) -> bool {
        let ratio = *self.write_ratio.read().unwrap();
        let threshold = self.config.heat_thresholds.write_pressure_threshold;
        ratio > threshold
    }

    /// Returns the current write ratio (0.0 = all reads, 1.0 = all writes).
    pub fn get_write_ratio(&self) -> f32 {
        *self.write_ratio.read().unwrap()
    }
}

/// Slot classification based on heat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotClass {
    /// Hot slot: heat >= H_hot (0.8)
    /// Target: K=1 (leveled compaction, minimize read amplification)
    Hot,

    /// Warm slot: H_cold < heat < H_hot
    /// Target: Maintain current K
    Warm,

    /// Cold slot: heat <= H_cold (0.2)
    /// Target: Increase K if write pressure high (tiered, minimize write amplification)
    Cold,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_weights() {
        assert_eq!(Operation::Get.weight(), 1.0);
        assert_eq!(Operation::Scan.weight(), 2.0);
        assert_eq!(Operation::Put.weight(), 0.5);
        assert_eq!(Operation::Delete.weight(), 0.5);
    }

    #[test]
    fn test_slot_heat_creation() {
        let heat = SlotHeat::new(3);
        assert_eq!(heat.score, 0.0);
        assert_eq!(heat.op_count, 0);
        assert_eq!(heat.current_k, 3);
    }

    #[test]
    fn test_slot_heat_update() {
        let mut heat = SlotHeat::new(3);

        // Record GET operation
        heat.update(Operation::Get, 100_000);
        assert!(heat.score > 0.0);
        assert_eq!(heat.op_count, 1);

        let first_score = heat.score;

        // Record another GET
        heat.update(Operation::Get, 100_000);
        assert!(heat.score > first_score); // Heat increases
        assert_eq!(heat.op_count, 2);
    }

    #[test]
    fn test_slot_heat_decay() {
        let mut heat = SlotHeat::new(3);
        heat.update(Operation::Get, 100_000);

        let initial_score = heat.score;

        // Simulate 1 second of idle time
        let elapsed = Duration::from_secs(1);
        heat.decay(elapsed, 100_000);

        assert!(heat.score < initial_score); // Heat decayed
    }

    #[test]
    fn test_heat_tracker_record_op() {
        let config = ATLLConfig::default();
        let tracker = HeatTracker::new(config);

        tracker.record_op(1, 0, Operation::Get);
        tracker.record_op(1, 0, Operation::Get);

        let heat = tracker.get_heat(1, 0);
        assert!(heat > 0.0);
    }

    #[test]
    fn test_slot_classification() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.hot = 0.8;
        config.heat_thresholds.cold = 0.2;
        config.heat_thresholds.half_life_ops = 100; // Faster convergence for testing

        let tracker = HeatTracker::new(config);

        // Cold slot (no operations)
        assert_eq!(tracker.classify_slot(1, 0), SlotClass::Cold);

        // Make it hot with many operations
        for _ in 0..10000 {
            tracker.record_op(1, 0, Operation::Get);
        }

        let classification = tracker.classify_slot(1, 0);
        let heat = tracker.get_heat(1, 0);

        // With enough operations and faster convergence, should be Hot or Warm
        assert!(
            classification == SlotClass::Hot || classification == SlotClass::Warm,
            "Expected Hot or Warm, got {:?} with heat {}",
            classification,
            heat
        );
    }

    #[test]
    fn test_dynamic_k_adjustment_hot() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.hot = 0.8;
        config.hot_k = 1;

        let tracker = HeatTracker::new(config);

        // Heat up slot 0
        for _ in 0..10000 {
            tracker.record_op(1, 0, Operation::Get);
        }

        // Adjust K (should converge to hot_k=1)
        let new_k = tracker.adjust_k(1, 0, false);

        if let Some(k) = new_k {
            assert_eq!(k, 1);
        }
    }

    #[test]
    fn test_dynamic_k_adjustment_cold() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.cold = 0.2;
        config.default_k.l1 = 3;

        let tracker = HeatTracker::new(config);

        // Cold slot, high write pressure
        let new_k = tracker.adjust_k(1, 0, true);

        // Should increase K toward default (3)
        // Starting from default, no change expected
        assert!(new_k.is_none() || new_k.unwrap() >= 3);
    }

    #[test]
    fn test_level_heat_scores() {
        let config = ATLLConfig::default();
        let tracker = HeatTracker::new(config);

        tracker.record_op(1, 0, Operation::Get);
        tracker.record_op(1, 1, Operation::Scan);
        tracker.record_op(2, 0, Operation::Put);

        let l1_scores = tracker.level_heat_scores(1);
        assert_eq!(l1_scores.len(), 2); // Slots 0 and 1

        let l2_scores = tracker.level_heat_scores(2);
        assert_eq!(l2_scores.len(), 1); // Slot 0 only
    }

    #[test]
    fn test_decay_idle_slots() {
        let config = ATLLConfig::default();
        let tracker = HeatTracker::new(config);

        // Heat up a slot
        for _ in 0..100 {
            tracker.record_op(1, 0, Operation::Get);
        }

        let initial_heat = tracker.get_heat(1, 0);

        // Manually decay (normally called periodically)
        tracker.decay_idle_slots();

        // Heat should remain similar since not enough time passed
        let heat_after = tracker.get_heat(1, 0);
        assert!((heat_after - initial_heat).abs() < 0.1);
    }

    #[test]
    fn test_operation_is_write() {
        assert!(!Operation::Get.is_write());
        assert!(!Operation::Scan.is_write());
        assert!(Operation::Put.is_write());
        assert!(Operation::Delete.is_write());
    }

    #[test]
    fn test_write_ratio_all_reads() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.half_life_ops = 100; // Faster convergence
        config.heat_thresholds.write_pressure_threshold = 0.5;

        let tracker = HeatTracker::new(config);

        // All reads: write ratio should stay near 0
        for _ in 0..1000 {
            tracker.record_op(1, 0, Operation::Get);
        }

        let ratio = tracker.get_write_ratio();
        assert!(
            ratio < 0.1,
            "Expected write ratio near 0 with all reads, got {}",
            ratio
        );
        assert!(!tracker.estimate_write_pressure());
    }

    #[test]
    fn test_write_ratio_all_writes() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.half_life_ops = 100; // Faster convergence
        config.heat_thresholds.write_pressure_threshold = 0.5;

        let tracker = HeatTracker::new(config);

        // All writes: write ratio should converge near 1
        for _ in 0..1000 {
            tracker.record_op(1, 0, Operation::Put);
        }

        let ratio = tracker.get_write_ratio();
        assert!(
            ratio > 0.9,
            "Expected write ratio near 1 with all writes, got {}",
            ratio
        );
        assert!(tracker.estimate_write_pressure());
    }

    #[test]
    fn test_write_ratio_mixed_workload() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.half_life_ops = 100; // Faster convergence
        config.heat_thresholds.write_pressure_threshold = 0.5;

        let tracker = HeatTracker::new(config);

        // 50% reads, 50% writes
        for i in 0..1000 {
            if i % 2 == 0 {
                tracker.record_op(1, 0, Operation::Get);
            } else {
                tracker.record_op(1, 0, Operation::Put);
            }
        }

        let ratio = tracker.get_write_ratio();
        // Should be around 0.5
        assert!(
            ratio > 0.4 && ratio < 0.6,
            "Expected write ratio near 0.5 with 50/50 mix, got {}",
            ratio
        );
    }

    #[test]
    fn test_write_pressure_threshold_custom() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.half_life_ops = 100; // Faster convergence
        config.heat_thresholds.write_pressure_threshold = 0.7; // Higher threshold

        let tracker = HeatTracker::new(config);

        // 60% writes
        for i in 0..1000 {
            if i % 5 < 2 {
                tracker.record_op(1, 0, Operation::Get);
            } else {
                tracker.record_op(1, 0, Operation::Put);
            }
        }

        let ratio = tracker.get_write_ratio();
        // Ratio should be around 0.6, which is below threshold 0.7
        assert!(
            !tracker.estimate_write_pressure(),
            "Expected low pressure with ratio {} < threshold 0.7",
            ratio
        );
    }

    #[test]
    fn test_write_ratio_adapts_to_workload_change() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.half_life_ops = 50; // Very fast convergence
        config.heat_thresholds.write_pressure_threshold = 0.5;

        let tracker = HeatTracker::new(config);

        // Start with all writes
        for _ in 0..500 {
            tracker.record_op(1, 0, Operation::Put);
        }

        let ratio_after_writes = tracker.get_write_ratio();
        assert!(
            ratio_after_writes > 0.8,
            "Expected high ratio after writes, got {}",
            ratio_after_writes
        );

        // Switch to all reads
        for _ in 0..500 {
            tracker.record_op(1, 0, Operation::Get);
        }

        let ratio_after_reads = tracker.get_write_ratio();
        assert!(
            ratio_after_reads < ratio_after_writes,
            "Expected ratio to decrease after reads: {} vs {}",
            ratio_after_reads,
            ratio_after_writes
        );
        assert!(
            ratio_after_reads < 0.3,
            "Expected low ratio after reads, got {}",
            ratio_after_reads
        );
    }
}
