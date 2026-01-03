/// Bandit-based compaction scheduler and physical merge execution.
///
/// Based on `lsm_atll_design.yaml` spec:
/// - Lines 225-231: Scheduler bandit algorithm
/// - Lines 197-224: Compaction action types
/// - Lines 132-134: IO budget and cooperative scheduling
///
/// # Architecture
///
/// ```text
/// ┌─────────────────────────────────────────────────────────────┐
/// │  BanditScheduler (per-slot arm selection)                   │
/// │  - Epsilon-greedy action selection                          │
/// │  - Reward = (latency_reduction × heat) / bytes_rewritten    │
/// │  - Non-stationary reward tracking                           │
/// └──────────────┬──────────────────────────────────────────────┘
///                │ select_action()
///                ↓
/// ┌─────────────────────────────────────────────────────────────┐
/// │  CompactionExecutor                                         │
/// │  - Tier: K-way merge within slot                            │
/// │  - Promote: Move run to L+1, merge if needed                │
/// │  - EagerLevel: Converge hot slot to K=1                     │
/// │  - Cleanup: Drop tombstones and TTL-expired keys            │
/// │  - GuardMove: Rebalance slot boundaries (guard keys)        │
/// └──────────────┬──────────────────────────────────────────────┘
///                │ execute()
///                ↓
/// ┌─────────────────────────────────────────────────────────────┐
/// │  MultiWayMerger                                             │
/// │  - Min-heap K-way merge                                     │
/// │  - Tombstone dropping (when safe)                           │
/// │  - Cooperative yielding (every 64MB)                        │
/// └─────────────────────────────────────────────────────────────┘
/// ```
///
/// # Guard Rebalancing (Trigger 8)
///
/// Guards define slot boundaries in leveled tiers (L≥1). Skewed write patterns can create
/// size imbalances where one slot grows much larger than others, degrading performance.
///
/// **Detection Heuristics** (`check_guard_rebalancing`):
/// - **Size Imbalance**: max_slot_bytes > 3× avg_slot_bytes
/// - **Minimum Threshold**: level total bytes > 1GB (avoid rebalancing tiny levels)
/// - **Hysteresis**: No compaction in last hour (guards are stable)
/// - **Minimum Slots**: level has >1 slot (can't rebalance single slot)
///
/// **Guard Proposal** (`propose_rebalanced_guards`):
/// - Samples keys from run boundaries (min_key, max_key) in all slots
/// - Creates temporary GuardManager and learns key distribution
/// - Uses quantile-based splitting via GuardManager.propose_guards()
/// - Validates guards are strictly sorted and non-empty
///
/// **Execution** (`execute_guard_move`):
/// - Collects all runs from all slots in the level
/// - Splits each run on new guard boundaries (similar to L0→L1 admission)
/// - Creates manifest edits: DeleteFile (old), AddFile (fragments), UpdateGuards
///
/// **Observability**:
/// - Emits VizEvent::Compaction(CompKind::GuardRebalance) with imbalance_ratio metrics
/// - Logs guard count change and total files affected
///
/// **Example Scenario**:
/// - Writes heavily skewed to "z*" prefix (90%) vs "a*" prefix (10%)
/// - Guard at "m" creates large slot [m, ∞) and small slot [∅, m)
/// - Rebalancing proposes new guard at "y" to split the hotspot
/// - Result: More balanced slot sizes → better load distribution
use crate::config::ATLLConfig;
use crate::error::{Error, Result};
use crate::heat::HeatTracker;
use crate::manifest::{ManifestLog, RunMeta};
use bytes::Bytes;
use nori_sstable::{
    Entry, FilterType, SSTableBuilder, SSTableConfig, SSTableIterator, SSTableReader,
};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

/// Compaction action types.
///
/// Each action represents a strategic choice for reducing read/write amplification
/// based on current workload heat and LSM state.
#[derive(Debug, Clone, PartialEq)]
pub enum CompactionAction {
    /// Horizontal tiering: merge K oldest runs within a slot.
    ///
    /// **When**: `runs > K_s` OR `bytes_s > slot_budget`
    /// **Effect**: Reduces fan-in (read amp) at cost of write amp
    Tier {
        level: u8,
        slot_id: u32,
        run_count: usize,
    },

    /// Vertical promotion: move run from level L to L+1.
    ///
    /// **When**: Run size exceeds level target OR slot bytes >> budget
    /// **Effect**: Pushes data down, maintains level invariants
    Promote {
        level: u8,
        slot_id: u32,
        file_number: u64,
    },

    /// Eager leveling for hot slots: converge runs → 1.
    ///
    /// **When**: `heat_s >= H_hot`
    /// **Effect**: Minimizes read amp for hot data
    EagerLevel { level: u8, slot_id: u32 },

    /// Tombstone cleanup: drop obsolete deletes and TTL-expired keys.
    ///
    /// **When**: `tombstone_density > threshold`
    /// **Effect**: Reclaims space, improves scan performance
    Cleanup { level: u8, slot_id: u32 },

    /// Guard rebalancing: adjust slot boundaries to fix imbalanced key distribution.
    ///
    /// **When**: Slot size imbalance detected (max_slot > 3× avg_slot, level > 1GB, no recent rebalance)
    /// **Effect**: Rebalances slot sizes by recalculating guard keys using quantile-based splitting
    /// **How**: Uses GuardManager.propose_guards() with key samples from run boundaries
    /// **Integration**: Trigger 8 in BanditScheduler, executed by CompactionExecutor
    GuardMove { level: u8, new_guards: Vec<Bytes> },

    /// No action: slot is healthy.
    DoNothing,
}

/// Per-slot bandit state for action selection.
#[derive(Debug, Clone)]
pub struct BanditArm {
    /// Slot identifier
    pub slot_id: u32,

    /// Level
    pub level: u8,

    /// Total reward accumulated
    pub total_reward: f64,

    /// Number of times this arm was selected
    pub selection_count: u64,

    /// Last observed reward (for non-stationary tracking)
    pub last_reward: f64,

    /// Exponentially weighted average reward
    pub avg_reward: f64,
}

impl BanditArm {
    /// Creates a new bandit arm.
    pub fn new(level: u8, slot_id: u32) -> Self {
        Self {
            level,
            slot_id,
            total_reward: 0.0,
            selection_count: 0,
            last_reward: 0.0,
            avg_reward: 0.0,
        }
    }

    /// Updates the arm with a new reward observation.
    ///
    /// Uses exponential moving average for non-stationary environments:
    /// `avg_reward_new = α × reward + (1 - α) × avg_reward_old`
    pub fn update(&mut self, reward: f64) {
        const ALPHA: f64 = 0.1; // Decay factor for non-stationary tracking

        self.total_reward += reward;
        self.selection_count += 1;
        self.last_reward = reward;

        if self.selection_count == 1 {
            self.avg_reward = reward;
        } else {
            self.avg_reward = ALPHA * reward + (1.0 - ALPHA) * self.avg_reward;
        }
    }

    /// Computes the upper confidence bound (UCB) score.
    ///
    /// UCB = avg_reward + c × sqrt(ln(total_selections) / arm_selections)
    pub fn ucb_score(&self, total_selections: u64, c: f64) -> f64 {
        if self.selection_count == 0 {
            return f64::INFINITY; // Unvisited arms have infinite priority
        }

        let exploration_bonus =
            c * ((total_selections as f64).ln() / self.selection_count as f64).sqrt();
        self.avg_reward + exploration_bonus
    }
}

/// Bandit scheduler for compaction action selection.
///
/// Uses epsilon-greedy or UCB policy to balance exploration/exploitation.
pub struct BanditScheduler {
    /// Per-(level, slot) arm state
    arms: HashMap<(u8, u32), BanditArm>,

    /// Epsilon for epsilon-greedy (0.1 = 10% exploration) - reserved for future use
    #[allow(dead_code)]
    epsilon: f64,

    /// Total selections across all arms
    total_selections: u64,

    /// Configuration
    config: ATLLConfig,

    /// Observability meter for emitting bandit events
    meter: Arc<dyn nori_observe::Meter>,
}

impl BanditScheduler {
    /// Creates a new bandit scheduler.
    pub fn new(config: ATLLConfig, meter: Arc<dyn nori_observe::Meter>) -> Self {
        Self {
            arms: HashMap::new(),
            epsilon: 0.1,
            total_selections: 0,
            config,
            meter,
        }
    }

    /// Selects the best compaction action based on current state.
    ///
    /// # Algorithm (Epsilon-Greedy)
    /// - With probability ε: explore (random action)
    /// - With probability 1-ε: exploit (best UCB score)
    ///
    /// # Reward Model
    /// `reward = (predicted_latency_reduction × heat_score) / bytes_rewritten`
    pub fn select_action(
        &self,
        snapshot: &crate::manifest::ManifestSnapshot,
        heat_tracker: &HeatTracker,
    ) -> CompactionAction {
        // Get all candidate actions from manifest state
        let candidates = self.generate_candidates(snapshot, heat_tracker);

        if candidates.is_empty() {
            return CompactionAction::DoNothing;
        }

        // Adaptive epsilon-greedy selection based on L0 pressure
        let l0_count = snapshot.l0_file_count();
        let epsilon = self.adaptive_epsilon(l0_count);
        let explore = rand::random::<f64>() < epsilon;

        tracing::trace!(
            l0_count = l0_count,
            epsilon = epsilon,
            explore = explore,
            "Adaptive bandit exploration"
        );

        let chosen_idx = if explore {
            // Explore: random action
            rand::random::<usize>() % candidates.len()
        } else {
            // Exploit: best UCB score
            self.best_ucb_action(&candidates)
        };

        let chosen_action = &candidates[chosen_idx];

        // Emit bandit selection event for observability
        if let Some((level, slot_id)) = Self::extract_level_slot(chosen_action) {
            let arm = self.arms.get(&(level, slot_id)).cloned().unwrap_or_else(|| BanditArm::new(level, slot_id));
            let ucb_score = arm.ucb_score(self.total_selections, 2.0);

            self.meter.emit(nori_observe::VizEvent::Compaction(
                nori_observe::CompEvt {
                    node: self.config.node_id,
                    level,
                    kind: nori_observe::CompKind::BanditSelection {
                        slot_id,
                        explored: explore,
                        ucb_score,
                        avg_reward: arm.avg_reward,
                        selection_count: arm.selection_count,
                    },
                },
            ));
        }

        chosen_action.clone()
    }

    /// Extracts (level, slot_id) from a CompactionAction.
    fn extract_level_slot(action: &CompactionAction) -> Option<(u8, u32)> {
        match action {
            CompactionAction::Tier { level, slot_id, .. }
            | CompactionAction::Promote { level, slot_id, .. }
            | CompactionAction::EagerLevel { level, slot_id }
            | CompactionAction::Cleanup { level, slot_id } => Some((*level, *slot_id)),
            _ => None,
        }
    }

    /// Updates the bandit state after executing an action.
    pub fn update_reward(
        &mut self,
        action: &CompactionAction,
        bytes_written: u64,
        latency_reduction_ms: f64,
        heat_score: f32,
        l0_count: usize,
    ) {
        let (level, slot_id) = match action {
            CompactionAction::Tier { level, slot_id, .. }
            | CompactionAction::Promote { level, slot_id, .. }
            | CompactionAction::EagerLevel { level, slot_id }
            | CompactionAction::Cleanup { level, slot_id } => (*level, *slot_id),
            _ => return, // No reward for DoNothing or GuardMove
        };

        // Calculate pressure weight: boost rewards for actions that reduce L0 pressure
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        // Pressure weight increases with pressure (1.0x -> 3.0x)
        let pressure_weight = if pressure_ratio >= 0.90 {
            3.0 // Red zone: 3x reward for pressure reduction
        } else if pressure_ratio >= 0.75 {
            2.0 // Orange zone: 2x reward
        } else if pressure_ratio >= 0.50 {
            1.5 // Yellow zone: 1.5x reward
        } else {
            1.0 // Green zone: baseline reward
        };

        // Boost L0 actions more than deeper levels when under pressure
        let level_multiplier = if level == 0 && pressure_ratio >= 0.75 {
            1.5 // Extra boost for L0 actions under pressure
        } else {
            1.0
        };

        // Compute pressure-weighted reward: (latency × heat × pressure × level) / bytes
        let base_reward = if bytes_written > 0 {
            (latency_reduction_ms * heat_score as f64) / bytes_written as f64
        } else {
            0.0
        };

        let reward = base_reward * pressure_weight * level_multiplier;

        // Update arm statistics
        let arm = self
            .arms
            .entry((level, slot_id))
            .or_insert_with(|| BanditArm::new(level, slot_id));

        arm.update(reward);
        self.total_selections += 1;

        // Emit bandit reward event for observability
        self.meter.emit(nori_observe::VizEvent::Compaction(
            nori_observe::CompEvt {
                node: self.config.node_id,
                level,
                kind: nori_observe::CompKind::BanditReward {
                    slot_id,
                    reward,
                    bytes_written,
                    heat_score,
                },
            },
        ));
    }

    /// Calculates adaptive L0 compaction trigger threshold based on pressure.
    ///
    /// More aggressive thresholds under pressure to prevent L0 accumulation:
    /// - Green zone (0-49% of max): 4 files (relaxed)
    /// - Yellow zone (50-74% of max): 3 files (normal)
    /// - Orange/Red zone (75%+ of max): 2 files (aggressive)
    fn adaptive_l0_threshold(&self, l0_count: usize) -> usize {
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        if pressure_ratio >= 0.75 {
            2 // Orange/Red zone: aggressive compaction
        } else if pressure_ratio >= 0.50 {
            3 // Yellow zone: moderate compaction
        } else {
            4 // Green zone: relaxed (default)
        }
    }

    /// Calculates adaptive epsilon (exploration rate) based on L0 pressure.
    ///
    /// Reduces exploration under pressure to exploit known-good strategies:
    /// - Green zone (0-49% of max): 0.10 (10% exploration - default)
    /// - Yellow zone (50-74% of max): 0.05 (5% exploration)
    /// - Orange zone (75-89% of max): 0.02 (2% exploration)
    /// - Red zone (90%+ of max): 0.0 (pure exploitation - emergency)
    fn adaptive_epsilon(&self, l0_count: usize) -> f64 {
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        if pressure_ratio >= 0.90 {
            0.0 // Red zone: pure exploitation (emergency)
        } else if pressure_ratio >= 0.75 {
            0.02 // Orange zone: minimal exploration
        } else if pressure_ratio >= 0.50 {
            0.05 // Yellow zone: reduced exploration
        } else {
            0.10 // Green zone: default exploration (10%)
        }
    }

    /// Generates candidate compaction actions from current LSM state.
    fn generate_candidates(
        &self,
        snapshot: &crate::manifest::ManifestSnapshot,
        heat_tracker: &HeatTracker,
    ) -> Vec<CompactionAction> {
        let mut candidates = Vec::new();

        // Trigger 1: L0 tiering (merge ALL L0 files to reduce read amplification)
        // Uses adaptive threshold based on current L0 pressure
        let l0_count = snapshot.l0_file_count();
        let l0_threshold = self.adaptive_l0_threshold(l0_count);

        tracing::trace!(
            l0_count = l0_count,
            threshold = l0_threshold,
            "Adaptive L0 compaction threshold"
        );

        if l0_count >= l0_threshold {
            // Merge ALL L0 files, not just a subset
            // This ensures MVCC correctness: newest version must be preserved
            candidates.push(CompactionAction::Tier {
                level: 0,
                slot_id: 0,          // L0 is treated as a single virtual slot
                run_count: l0_count, // Merge all files
            });
        }

        // Iterate over L1+ slots and check for compaction triggers
        for level_meta in snapshot.levels.iter().skip(1) {
            for slot in &level_meta.slots {
                let heat = heat_tracker.get_heat(level_meta.level, slot.slot_id);

                // Trigger 2: Single-run promotion (free tier transition)
                // If slot has exactly 1 run, promote it to next level (zero-cost metadata move)
                if slot.runs.len() == 1 && level_meta.level < self.config.max_levels {
                    candidates.push(CompactionAction::Promote {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                        file_number: slot.runs[0].file_number,
                    });
                }

                // Trigger 3: Size-based tiering (slot too large)
                // Target size grows exponentially: L1=64MB, L2=640MB, L3=6.4GB, etc.
                // Each level is fanout (default 10x) larger than previous
                let base_size_mb = 64; // L1 base size
                let level_multiplier = self.config.fanout.pow((level_meta.level - 1) as u32) as u64;
                let target_size_bytes = (base_size_mb * 1024 * 1024) * level_multiplier;

                // Trigger if slot exceeds 1.5x target size
                if slot.bytes > (target_size_bytes * 3 / 2) && slot.runs.len() >= 2 {
                    candidates.push(CompactionAction::Tier {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                        run_count: slot.runs.len().min(4),
                    });
                }

                // Trigger 4: Run count threshold (too many runs)
                // Use dynamic K from heat tracker (which adjusts based on workload)
                let dynamic_k = heat_tracker.get_k(level_meta.level, slot.slot_id);
                if slot.runs.len() > dynamic_k as usize {
                    candidates.push(CompactionAction::Tier {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                        run_count: dynamic_k.min(4) as usize,
                    });
                }

                // Trigger 5: EagerLevel for hot slots with multiple runs
                if heat >= self.config.heat_thresholds.hot && slot.runs.len() > 1 {
                    candidates.push(CompactionAction::EagerLevel {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                    });
                }

                // Trigger 6: Tombstone cleanup at bottom level
                // Only clean tombstones at max_levels (no lower levels exist)
                if level_meta.level == self.config.max_levels
                    && slot.tombstone_density >= self.config.tombstone.density_threshold
                    && !slot.runs.is_empty()
                {
                    candidates.push(CompactionAction::Cleanup {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                    });
                }

                // Trigger 7: Age-based compaction (stale slots need refresh)
                // If a slot hasn't been compacted in 24 hours and has multiple runs, compact it
                let max_age_sec = 24 * 3600; // 24 hours
                let now_sec = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let age_sec = now_sec.saturating_sub(slot.last_compaction_ts);

                if age_sec > max_age_sec && slot.runs.len() >= 2 {
                    candidates.push(CompactionAction::Tier {
                        level: level_meta.level,
                        slot_id: slot.slot_id,
                        run_count: slot.runs.len().min(4),
                    });
                }
            }

            // Trigger 8: Guard rebalancing (key distribution skew)
            // Check if slots in this level have significantly imbalanced sizes
            if let Some(guard_move) = self.check_guard_rebalancing(level_meta, snapshot) {
                candidates.push(guard_move);
            }
        }

        candidates
    }

    /// Selects the action with the best UCB score.
    fn best_ucb_action(&self, candidates: &[CompactionAction]) -> usize {
        let mut best_idx = 0;
        let mut best_score = f64::NEG_INFINITY;

        for (idx, action) in candidates.iter().enumerate() {
            let (level, slot_id) = match action {
                CompactionAction::Tier { level, slot_id, .. }
                | CompactionAction::Promote { level, slot_id, .. }
                | CompactionAction::EagerLevel { level, slot_id }
                | CompactionAction::Cleanup { level, slot_id } => (*level, *slot_id),
                _ => continue,
            };

            let arm = self
                .arms
                .get(&(level, slot_id))
                .cloned()
                .unwrap_or_else(|| BanditArm::new(level, slot_id));

            let score = arm.ucb_score(self.total_selections, 2.0);

            if score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }

        best_idx
    }

    /// Checks if guard rebalancing is needed for a level.
    ///
    /// # Detection Heuristics
    /// Guards should be rebalanced when:
    /// 1. **Size Imbalance**: One slot is significantly larger than others (>3x average)
    /// 2. **Minimum Data Threshold**: Level has enough data to make rebalancing worthwhile (>1GB)
    /// 3. **Minimum Slots**: Level has multiple slots (can't rebalance single slot)
    /// 4. **Hysteresis**: Haven't rebalanced this level recently (>1 hour since last)
    ///
    /// # Returns
    /// `Some(CompactionAction::GuardMove)` if rebalancing is needed, `None` otherwise
    fn check_guard_rebalancing(
        &self,
        level_meta: &crate::manifest::LevelMeta,
        _snapshot: &crate::manifest::ManifestSnapshot,
    ) -> Option<CompactionAction> {
        // Only rebalance levels with multiple slots
        if level_meta.slots.len() <= 1 {
            return None;
        }

        // Calculate total bytes and average per slot
        let total_bytes: u64 = level_meta.slots.iter().map(|s| s.bytes).sum();
        let avg_bytes = total_bytes / level_meta.slots.len() as u64;

        // Minimum threshold: only rebalance if level has >1GB of data
        const MIN_LEVEL_SIZE_BYTES: u64 = 1024 * 1024 * 1024; // 1GB
        if total_bytes < MIN_LEVEL_SIZE_BYTES {
            return None;
        }

        // Find max slot size
        let max_slot_bytes = level_meta.slots.iter().map(|s| s.bytes).max().unwrap_or(0);

        // Detect imbalance: max slot is >3x average
        const IMBALANCE_THRESHOLD: f64 = 3.0;
        let imbalance_ratio = if avg_bytes > 0 {
            max_slot_bytes as f64 / avg_bytes as f64
        } else {
            0.0
        };

        if imbalance_ratio < IMBALANCE_THRESHOLD {
            return None; // Not imbalanced enough
        }

        // Hysteresis: Don't rebalance too frequently
        // Check if any slot was recently compacted (guards likely stable)
        const MIN_REBALANCE_INTERVAL_SEC: u64 = 3600; // 1 hour
        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for slot in &level_meta.slots {
            let time_since_compaction = now_sec.saturating_sub(slot.last_compaction_ts);
            if time_since_compaction < MIN_REBALANCE_INTERVAL_SEC {
                return None; // Too soon since last compaction
            }
        }

        // Imbalance detected! Propose new guards based on quantile splitting
        let new_guards = self.propose_rebalanced_guards(level_meta);

        if new_guards.is_empty() || new_guards == level_meta.guards {
            return None; // No better guard configuration found
        }

        println!(
            "Guard rebalancing triggered for level {}: imbalance ratio {:.2}x, proposing {} new guards",
            level_meta.level,
            imbalance_ratio,
            new_guards.len()
        );

        Some(CompactionAction::GuardMove {
            level: level_meta.level,
            new_guards,
        })
    }

    /// Proposes new guard placements to rebalance slot sizes.
    ///
    /// # Strategy
    /// 1. Create a temporary GuardManager with target slot count
    /// 2. Learn key distribution from run boundaries
    /// 3. Use GuardManager's propose_guards() for quantile-based splitting
    ///
    /// # Integration with GuardManager Learning
    /// This method integrates GuardManager's learning infrastructure:
    /// - Uses learn_from_keys() to build key distribution sketch
    /// - Uses propose_guards() for better quantile estimation
    /// - Provides better guard placement than simple min/max sampling
    ///
    /// # Future Enhancement
    /// Could be extended to sample keys from actual SSTable files for
    /// even better distribution estimation (requires async context).
    fn propose_rebalanced_guards(&self, level_meta: &crate::manifest::LevelMeta) -> Vec<Bytes> {
        // Collect all run min/max keys as sample points
        let mut key_samples: Vec<Bytes> = Vec::new();

        for slot in &level_meta.slots {
            for run in &slot.runs {
                key_samples.push(run.min_key.clone());
                key_samples.push(run.max_key.clone());
            }
        }

        if key_samples.len() < 2 {
            return Vec::new(); // Not enough samples
        }

        // Target: same number of slots as before (or slightly more if very imbalanced)
        let target_slot_count = level_meta.slots.len().max(4);

        // Create temporary GuardManager and use its learning infrastructure
        let mut temp_guard_mgr = match crate::guards::GuardManager::new(
            level_meta.level,
            target_slot_count as u32,
        ) {
            Ok(mgr) => mgr,
            Err(_) => {
                // Fallback to current guards if GuardManager creation fails
                return level_meta.guards.clone();
            }
        };

        // Learn from collected key samples
        temp_guard_mgr.learn_from_keys(&key_samples);

        // Propose new guards using GuardManager's quantile logic
        let proposed = temp_guard_mgr.propose_guards();

        // Validate: guards must be strictly sorted and non-empty
        if proposed.is_empty() {
            return Vec::new();
        }

        for window in proposed.windows(2) {
            if window[0] >= window[1] {
                return Vec::new(); // Invalid guards
            }
        }

        proposed
    }
}

/// Multi-way merger for K SSTable inputs.
///
/// Uses a min-heap to efficiently merge sorted runs while dropping tombstones
/// when safe (no older levels exist).
pub struct MultiWayMerger {
    /// Input SSTable readers
    readers: Vec<Arc<SSTableReader>>,

    /// Current state of each iterator
    heap: BinaryHeap<MergeCandidate>,

    /// Output SSTable builder
    builder: SSTableBuilder,

    /// Can we drop tombstones? (true if no lower levels)
    can_drop_tombstones: bool,

    /// Bytes written so far (for cooperative yielding)
    bytes_written: u64,

    /// Slice size for cooperative yielding (adaptive based on pressure)
    slice_size: u64,

    /// Output filter type (v1=Bloom, v2=QuotientFilter)
    output_filter_type: FilterType,
}

impl MultiWayMerger {
    /// Calculates adaptive slice size based on L0 pressure.
    ///
    /// Smaller slices provide more cooperative yielding and better responsiveness
    /// under pressure, at the cost of slightly higher overhead:
    /// - Green zone (0-49% of max): 128MB (larger slices, less overhead)
    /// - Yellow zone (50-74% of max): 64MB (normal, default)
    /// - Orange zone (75-89% of max): 32MB (smaller slices, more responsive)
    /// - Red zone (90%+ of max): 16MB (emergency, maximum responsiveness)
    fn adaptive_slice_size(l0_count: usize, max_files: usize, default_mb: u64) -> u64 {
        let pressure_ratio = l0_count as f64 / max_files as f64;

        let slice_mb = if pressure_ratio >= 0.90 {
            16 // Red zone: emergency responsiveness
        } else if pressure_ratio >= 0.75 {
            32 // Orange zone: more responsive
        } else if pressure_ratio >= 0.50 {
            default_mb // Yellow zone: normal (use config default)
        } else {
            128 // Green zone: larger slices for efficiency
        };

        slice_mb * 1024 * 1024
    }
}

/// Heap entry for K-way merge.
struct MergeCandidate {
    /// Entry from SSTable
    entry: Entry,

    /// Iterator index (which SSTable this came from)
    iterator_idx: usize,
}

impl PartialEq for MergeCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.entry.key == other.entry.key
    }
}

impl Eq for MergeCandidate {}

impl PartialOrd for MergeCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MergeCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: reverse comparison on key
        // For ties, prefer newer data (higher iterator_idx = newer)
        other
            .entry
            .key
            .cmp(&self.entry.key)
            .then_with(|| self.iterator_idx.cmp(&other.iterator_idx))
    }
}

impl MultiWayMerger {
    /// Determines output filter type based on input SSTables.
    ///
    /// Strategy:
    /// - If any input uses v2 (QuotientFilter), output uses v2
    /// - Otherwise, uses v1 (Bloom) for backward compatibility
    ///
    /// This allows gradual migration: as v2 SSTables enter the system,
    /// compaction outputs will also be v2, eventually converting the entire store.
    fn choose_output_filter_type(readers: &[Arc<SSTableReader>]) -> FilterType {
        // Use v2 if any input uses v2 (gradual migration)
        let has_v2_input = readers.iter().any(|r| r.uses_quotient_filter());

        if has_v2_input {
            tracing::debug!(
                v2_inputs = readers.iter().filter(|r| r.uses_quotient_filter()).count(),
                total_inputs = readers.len(),
                "Using QuotientFilter for compaction output (v2 input detected)"
            );
            FilterType::QuotientFilter
        } else {
            FilterType::Bloom
        }
    }

    /// Creates a new K-way merger with optional adaptive slice sizing.
    ///
    /// If `l0_count` is provided, uses adaptive slice size based on pressure.
    /// Otherwise, uses the default from config.
    pub async fn new(
        input_paths: Vec<PathBuf>,
        output_path: PathBuf,
        can_drop_tombstones: bool,
        config: &ATLLConfig,
        l0_count: Option<usize>,
    ) -> Result<Self> {
        Self::new_with_filter(
            input_paths,
            output_path,
            can_drop_tombstones,
            config,
            l0_count,
            None, // Auto-detect filter type
        )
        .await
    }

    /// Creates a new K-way merger with explicit filter type control.
    ///
    /// If `filter_type` is None, auto-detects based on input SSTable formats.
    /// If `filter_type` is Some, uses the specified filter type.
    pub async fn new_with_filter(
        input_paths: Vec<PathBuf>,
        output_path: PathBuf,
        can_drop_tombstones: bool,
        config: &ATLLConfig,
        l0_count: Option<usize>,
        filter_type: Option<FilterType>,
    ) -> Result<Self> {
        // Open all input SSTables
        let mut readers = Vec::new();
        for path in input_paths {
            let reader = SSTableReader::open(path)
                .await
                .map_err(|e| Error::Internal(format!("Failed to open SSTable: {}", e)))?;
            readers.push(Arc::new(reader));
        }

        // Determine output filter type
        let output_filter_type = filter_type.unwrap_or_else(|| Self::choose_output_filter_type(&readers));

        // Create output builder
        let estimated_entries: u64 = readers.iter().map(|r| r.entry_count()).sum();
        let estimated_entries = estimated_entries as usize;
        let sst_config = SSTableConfig {
            path: output_path,
            estimated_entries,
            block_size: 4096,
            restart_interval: 16,
            compression: nori_sstable::Compression::None,
            bloom_bits_per_key: 10,
            block_cache_mb: 64,
            filter_type: output_filter_type,
            ..Default::default()
        };

        let builder = SSTableBuilder::new(sst_config)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create SSTable builder: {}", e)))?;

        // Calculate slice size: adaptive if l0_count provided, otherwise use config default
        let slice_size = if let Some(l0_count) = l0_count {
            let adaptive_size = Self::adaptive_slice_size(
                l0_count,
                config.l0.max_files,
                config.io.compaction_slice_mb as u64,
            );
            tracing::trace!(
                l0_count = l0_count,
                max_files = config.l0.max_files,
                slice_mb = adaptive_size / (1024 * 1024),
                "Adaptive compaction slice size"
            );
            adaptive_size
        } else {
            (config.io.compaction_slice_mb as u64) * 1024 * 1024
        };

        Ok(Self {
            readers,
            heap: BinaryHeap::new(),
            builder,
            can_drop_tombstones,
            bytes_written: 0,
            slice_size,
            output_filter_type,
        })
    }

    /// Performs K-way merge and writes to output SSTable.
    ///
    /// Returns metadata for the merged run.
    pub async fn merge(mut self) -> Result<RunMeta> {
        // Initialize heap with first entry from each iterator
        let mut iterators: Vec<SSTableIterator> =
            self.readers.iter().map(|r| r.clone().iter()).collect();

        for (idx, iter) in iterators.iter_mut().enumerate() {
            if let Some(entry) = iter
                .try_next()
                .await
                .map_err(|e| Error::Internal(format!("SSTable read error: {}", e)))?
            {
                self.heap.push(MergeCandidate {
                    entry,
                    iterator_idx: idx,
                });
            }
        }

        let mut min_key: Option<Bytes> = None;
        let mut max_key: Option<Bytes> = None;
        let min_version = crate::Version::new(u64::MAX, u64::MAX);
        let max_version = crate::Version::new(0, 0);
        let mut tombstone_count = 0u32;
        let mut last_key: Option<Bytes> = None;

        // Merge loop
        while let Some(candidate) = self.heap.pop() {
            let entry = candidate.entry;
            let iter_idx = candidate.iterator_idx;

            // Deduplicate: skip if same key as last (keeping newest)
            if let Some(ref last) = last_key {
                if last == &entry.key {
                    // Advance iterator and continue
                    if let Some(next_entry) = iterators[iter_idx]
                        .try_next()
                        .await
                        .map_err(|e| Error::Internal(format!("SSTable read error: {}", e)))?
                    {
                        self.heap.push(MergeCandidate {
                            entry: next_entry,
                            iterator_idx: iter_idx,
                        });
                    }
                    continue;
                }
            }

            // Tombstone dropping
            if entry.tombstone {
                tombstone_count += 1;

                if self.can_drop_tombstones {
                    // Drop tombstone and continue
                    last_key = Some(entry.key.clone());
                    if let Some(next_entry) = iterators[iter_idx]
                        .try_next()
                        .await
                        .map_err(|e| Error::Internal(format!("SSTable read error: {}", e)))?
                    {
                        self.heap.push(MergeCandidate {
                            entry: next_entry,
                            iterator_idx: iter_idx,
                        });
                    }
                    continue;
                }
            }

            // Write entry to output
            // For v2 format, compute fingerprint once and pass through to avoid re-hashing
            if self.output_filter_type == FilterType::QuotientFilter {
                if let Some(fp) = self.builder.compute_fingerprint(&entry.key) {
                    self.builder
                        .add_with_fingerprint(&entry, fp)
                        .await
                        .map_err(|e| Error::Internal(format!("SSTable write error: {}", e)))?;
                } else {
                    // Fallback if compute_fingerprint fails (shouldn't happen for v2)
                    self.builder
                        .add(&entry)
                        .await
                        .map_err(|e| Error::Internal(format!("SSTable write error: {}", e)))?;
                }
            } else {
                self.builder
                    .add(&entry)
                    .await
                    .map_err(|e| Error::Internal(format!("SSTable write error: {}", e)))?;
            }

            // Track metadata
            if min_key.is_none() {
                min_key = Some(entry.key.clone());
            }
            max_key = Some(entry.key.clone());

            // TODO: Extract seqno from entry metadata when available
            // For now, use placeholder values

            last_key = Some(entry.key.clone());
            // Estimate entry size: key + value + overhead
            let entry_size = entry.key.len() + entry.value.len() + 16;
            self.bytes_written += entry_size as u64;

            // Cooperative yield point
            if self.bytes_written >= self.slice_size {
                // In a real implementation, we'd yield to tokio scheduler here
                // For now, just reset counter
                self.bytes_written = 0;
            }

            // Advance iterator
            if let Some(next_entry) = iterators[iter_idx]
                .try_next()
                .await
                .map_err(|e| Error::Internal(format!("SSTable read error: {}", e)))?
            {
                self.heap.push(MergeCandidate {
                    entry: next_entry,
                    iterator_idx: iter_idx,
                });
            }
        }

        // Finalize output SSTable
        let metadata = self
            .builder
            .finish()
            .await
            .map_err(|e| Error::Internal(format!("Failed to finish SSTable: {}", e)))?;

        Ok(RunMeta {
            file_number: 0, // Filled by caller
            size: metadata.file_size,
            min_key: min_key.ok_or_else(|| Error::Internal("No min key".to_string()))?,
            max_key: max_key.ok_or_else(|| Error::Internal("No max key".to_string()))?,
            min_version,
            max_version,
            tombstone_count,
            filter_fp: 0.001,
            heat_hint: 0.0,
            value_log_segment_id: None,
        })
    }

    /// Returns the filter type used for the output SSTable.
    pub fn output_filter_type(&self) -> FilterType {
        self.output_filter_type
    }

    /// Returns true if the output uses per-block Quotient Filters (v2 format).
    pub fn uses_quotient_filter(&self) -> bool {
        self.output_filter_type == FilterType::QuotientFilter
    }
}

/// Placeholder for manifest level state.
///
/// In a real implementation, this would come from ManifestLog.
pub struct LevelSlotState {
    pub run_count: usize,
    pub total_bytes: u64,
    pub tombstone_density: f64,
    pub runs: Vec<RunMeta>,
}

/// Placeholder extension trait for ManifestLog.
trait ManifestExt {
    fn level_state(&self, level: u8) -> Vec<LevelSlotState>;
}

impl ManifestExt for ManifestLog {
    fn level_state(&self, level: u8) -> Vec<LevelSlotState> {
        let snapshot = self.snapshot();
        let snap_guard = snapshot.read();

        if level >= snap_guard.levels.len() as u8 {
            return vec![]; // Level doesn't exist yet
        }

        let level_meta = &snap_guard.levels[level as usize];

        // L0 is special: no slots, just a collection of overlapping files
        if level == 0 {
            let total_bytes: u64 = level_meta.l0_files.iter().map(|r| r.size).sum();
            let run_count = level_meta.l0_files.len();

            // Calculate tombstone density
            let total_tombstones: u32 = level_meta.l0_files.iter().map(|r| r.tombstone_count).sum();
            let total_entries: u64 = level_meta
                .l0_files
                .iter()
                .map(|r| {
                    // Estimate entries from file size (rough approximation)
                    r.size / 100 // Assume ~100 bytes per entry on average
                })
                .sum();
            let tombstone_density = if total_entries > 0 {
                (total_tombstones as f64) / (total_entries as f64)
            } else {
                0.0
            };

            return vec![LevelSlotState {
                run_count,
                total_bytes,
                tombstone_density,
                runs: level_meta.l0_files.clone(),
            }];
        }

        // L1+: return state for each slot
        level_meta
            .slots
            .iter()
            .map(|slot| {
                let run_count = slot.runs.len();
                let total_bytes = slot.bytes;
                let tombstone_density = slot.tombstone_density as f64;

                LevelSlotState {
                    run_count,
                    total_bytes,
                    tombstone_density,
                    runs: slot.runs.clone(),
                }
            })
            .collect()
    }
}

/// Compaction executor that executes selected actions.
///
/// Coordinates with the scheduler, manifest, and SSTable I/O to perform
/// physical compaction operations.
pub struct CompactionExecutor {
    /// SSTable directory
    sst_dir: PathBuf,

    /// Configuration
    config: ATLLConfig,
}

impl CompactionExecutor {
    /// Creates a new compaction executor.
    pub fn new(sst_dir: impl AsRef<std::path::Path>, config: ATLLConfig) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&sst_dir)?;

        Ok(Self { sst_dir, config })
    }

    /// Executes a compaction action and returns manifest edits.
    ///
    /// # Returns
    /// - Vec<ManifestEdit>: Edits to apply to manifest
    /// - u64: Bytes written during compaction
    pub async fn execute(
        &self,
        action: &CompactionAction,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        match action {
            CompactionAction::Tier {
                level,
                slot_id,
                run_count,
            } => {
                self.execute_tier(*level, *slot_id, *run_count, manifest)
                    .await
            }

            CompactionAction::Promote {
                level,
                slot_id,
                file_number,
            } => {
                self.execute_promote(*level, *slot_id, *file_number, manifest)
                    .await
            }

            CompactionAction::EagerLevel { level, slot_id } => {
                self.execute_eager_level(*level, *slot_id, manifest).await
            }

            CompactionAction::Cleanup { level, slot_id } => {
                self.execute_cleanup(*level, *slot_id, manifest).await
            }

            CompactionAction::GuardMove { level, new_guards } => {
                self.execute_guard_move(*level, new_guards.clone(), manifest)
                    .await
            }

            CompactionAction::DoNothing => Ok((vec![], 0)),
        }
    }

    /// Executes horizontal tiering: merge K oldest runs in a slot.
    async fn execute_tier(
        &self,
        level: u8,
        slot_id: u32,
        run_count: usize,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        // Get slot state from manifest
        let level_state = {
            let manifest_guard = manifest.read();
            manifest_guard.level_state(level)
        };
        if slot_id as usize >= level_state.len() {
            return Err(Error::Internal(format!(
                "Slot {} not found in level {}",
                slot_id, level
            )));
        }

        let slot_state = &level_state[slot_id as usize];
        if slot_state.runs.is_empty() {
            return Ok((vec![], 0)); // Nothing to compact
        }

        // Select oldest N runs (up to run_count)
        let runs_to_merge: Vec<_> = slot_state
            .runs
            .iter()
            .take(run_count.min(slot_state.runs.len()))
            .cloned()
            .collect();

        if runs_to_merge.len() <= 1 {
            return Ok((vec![], 0)); // Need at least 2 runs to merge
        }

        // Build input paths
        let input_paths: Vec<PathBuf> = runs_to_merge
            .iter()
            .map(|run| self.sst_path(run.file_number))
            .collect();

        // Allocate new file number from manifest
        let output_file_number = Self::allocate_file_number(manifest);
        let output_path = self.sst_path(output_file_number);

        // Can drop tombstones only if this is the last level
        let can_drop_tombstones = level == self.config.max_levels;

        // Get L0 count for adaptive slice sizing
        let l0_count = {
            let manifest_guard = manifest.read();
            let snapshot = manifest_guard.snapshot();
            let snapshot_guard = snapshot.read();
            snapshot_guard.l0_file_count()
        };

        // Perform K-way merge with adaptive slice size
        let merger = MultiWayMerger::new(
            input_paths,
            output_path,
            can_drop_tombstones,
            &self.config,
            Some(l0_count),
        )
        .await?;

        let mut merged_run = merger.merge().await?;
        merged_run.file_number = output_file_number;

        let bytes_written = merged_run.size;

        // Generate manifest edits
        let mut edits = Vec::new();

        // Delete input runs
        for run in &runs_to_merge {
            edits.push(ManifestEdit::DeleteFile {
                level,
                slot_id: Some(slot_id),
                file_number: run.file_number,
            });
        }

        // Add merged run
        edits.push(ManifestEdit::AddFile {
            level,
            slot_id: Some(slot_id),
            run: merged_run,
        });

        Ok((edits, bytes_written))
    }

    /// Executes vertical promotion: move run from level L to L+1.
    async fn execute_promote(
        &self,
        level: u8,
        slot_id: u32,
        file_number: u64,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        if level >= self.config.max_levels {
            return Err(Error::Internal(
                "Cannot promote from last level".to_string(),
            ));
        }

        // Get the run to promote
        let level_state = {
            let manifest_guard = manifest.read();
            manifest_guard.level_state(level)
        };
        if slot_id as usize >= level_state.len() {
            return Err(Error::Internal(format!(
                "Slot {} not found in level {}",
                slot_id, level
            )));
        }

        let slot_state = &level_state[slot_id as usize];
        let run_to_promote = slot_state
            .runs
            .iter()
            .find(|r| r.file_number == file_number)
            .ok_or_else(|| Error::Internal(format!("Run {} not found", file_number)))?;

        // For simplification in Phase 6: just move the run
        // In Phase 8, we'd merge with overlapping runs in L+1
        let edits = vec![
            // Delete from current level
            ManifestEdit::DeleteFile {
                level,
                slot_id: Some(slot_id),
                file_number,
            },
            // Add to next level (same slot)
            ManifestEdit::AddFile {
                level: level + 1,
                slot_id: Some(slot_id),
                run: run_to_promote.clone(),
            },
        ];

        Ok((edits, 0)) // No actual bytes written (just metadata move)
    }

    /// Executes eager leveling: converge hot slot to K=1.
    async fn execute_eager_level(
        &self,
        level: u8,
        slot_id: u32,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        // Eager leveling is essentially tiering with run_count = all runs
        let level_state = {
            let manifest_guard = manifest.read();
            manifest_guard.level_state(level)
        };
        if slot_id as usize >= level_state.len() {
            return Err(Error::Internal(format!(
                "Slot {} not found in level {}",
                slot_id, level
            )));
        }

        let slot_state = &level_state[slot_id as usize];
        let run_count = slot_state.runs.len();

        // Merge all runs into one
        self.execute_tier(level, slot_id, run_count, manifest).await
    }

    /// Executes cleanup: drop tombstones and expired keys.
    async fn execute_cleanup(
        &self,
        level: u8,
        slot_id: u32,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        // Get slot runs
        let level_state = {
            let manifest_guard = manifest.read();
            manifest_guard.level_state(level)
        };
        if slot_id as usize >= level_state.len() {
            return Err(Error::Internal(format!(
                "Slot {} not found in level {}",
                slot_id, level
            )));
        }

        let slot_state = &level_state[slot_id as usize];
        if slot_state.runs.is_empty() {
            return Ok((vec![], 0));
        }

        // Build input paths for all runs in slot
        let input_paths: Vec<PathBuf> = slot_state
            .runs
            .iter()
            .map(|run| self.sst_path(run.file_number))
            .collect();

        // Allocate new file number from manifest
        let output_file_number = Self::allocate_file_number(manifest);
        let output_path = self.sst_path(output_file_number);

        // Force tombstone dropping for cleanup
        let can_drop_tombstones = true;

        // Get L0 count for adaptive slice sizing
        let l0_count = {
            let manifest_guard = manifest.read();
            let snapshot = manifest_guard.snapshot();
            let snapshot_guard = snapshot.read();
            snapshot_guard.l0_file_count()
        };

        // Perform merge with tombstone dropping and adaptive slice size
        let merger = MultiWayMerger::new(
            input_paths,
            output_path,
            can_drop_tombstones,
            &self.config,
            Some(l0_count),
        )
        .await?;

        let mut merged_run = merger.merge().await?;
        merged_run.file_number = output_file_number;

        let bytes_written = merged_run.size;

        // Generate manifest edits
        let mut edits = Vec::new();

        // Delete all input runs
        for run in &slot_state.runs {
            edits.push(ManifestEdit::DeleteFile {
                level,
                slot_id: Some(slot_id),
                file_number: run.file_number,
            });
        }

        // Add cleaned run
        edits.push(ManifestEdit::AddFile {
            level,
            slot_id: Some(slot_id),
            run: merged_run,
        });

        Ok((edits, bytes_written))
    }

    /// Executes guard move: rebalances slot boundaries by moving guard keys.
    ///
    /// # Phase 7 Implementation
    /// Physically reorganizes data in a level according to new guard boundaries.
    /// This is triggered when key distribution skew is detected.
    ///
    /// # Algorithm
    /// 1. Create temporary GuardManager with new guards
    /// 2. For each file in the level:
    ///    - Determine which new slots it overlaps
    ///    - Split file on new guard boundaries (similar to L0→L1 admission)
    ///    - Create fragments for each overlapping slot
    /// 3. Generate manifest edits:
    ///    - DeleteFile for all old files
    ///    - AddFile for all new fragments
    ///    - UpdateGuards to apply new guard configuration
    ///
    /// # Returns
    /// Vector of manifest edits and total bytes written
    async fn execute_guard_move(
        &self,
        level: u8,
        new_guards: Vec<Bytes>,
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        println!(
            "=== GUARD MOVE: Rebalancing level {} with {} new guards ===",
            level,
            new_guards.len()
        );

        // Get all files in this level
        let level_state = {
            let manifest_guard = manifest.read();
            manifest_guard.level_state(level)
        };

        // Collect all runs from all slots in this level
        let mut all_runs = Vec::new();
        for slot_state in &level_state {
            for run in &slot_state.runs {
                all_runs.push(run.clone());
            }
        }

        if all_runs.is_empty() {
            println!("  Level {} is empty, only updating guards", level);
            // Still need to update guards even if level is empty
            return Ok((
                vec![ManifestEdit::UpdateGuards {
                    level,
                    guards: new_guards,
                }],
                0,
            ));
        }

        // Create temporary GuardManager with new guards
        let temp_guard_manager = crate::guards::GuardManager::with_guards(level, new_guards.clone())?;

        let mut edits = Vec::new();
        let mut total_bytes_written = 0u64;

        // Process each file: split according to new guards
        for run in &all_runs {
            println!(
                "  Processing file {} ({}..{})",
                run.file_number,
                String::from_utf8_lossy(&run.min_key),
                String::from_utf8_lossy(&run.max_key)
            );

            // Find which new slots this file overlaps
            let overlapping_slots =
                temp_guard_manager.overlapping_slots(&run.min_key, &run.max_key);

            println!("    Overlaps {} new slots: {:?}", overlapping_slots.len(), overlapping_slots);

            // If file only overlaps one slot, no splitting needed - just move it
            if overlapping_slots.len() == 1 {
                let new_slot_id = overlapping_slots[0];

                // Delete from old location (we'll re-add to new slot)
                edits.push(ManifestEdit::DeleteFile {
                    level,
                    slot_id: None, // Don't specify old slot
                    file_number: run.file_number,
                });

                // Add to new slot
                edits.push(ManifestEdit::AddFile {
                    level,
                    slot_id: Some(new_slot_id),
                    run: run.clone(),
                });

                continue;
            }

            // File spans multiple slots - need physical splitting

            // Create one fragment per overlapping slot
            for slot_id in &overlapping_slots {
                let new_file_number = Self::allocate_file_number(manifest);
                let output_path = self.sst_path(new_file_number);

                // Get slot boundaries
                let (slot_min, slot_max) = temp_guard_manager.slot_boundaries(*slot_id);

                let config = nori_sstable::SSTableConfig {
                    path: output_path,
                    estimated_entries: 1000,
                    block_size: 4096,
                    restart_interval: 16,
                    compression: nori_sstable::Compression::None,
                    bloom_bits_per_key: 10,
                    block_cache_mb: 64,
                    ..Default::default()
                };

                let mut builder = nori_sstable::SSTableBuilder::new(config)
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to create builder: {}", e)))?;

                // Iterate and filter entries within slot bounds
                let reader_clone = nori_sstable::SSTableReader::open(self.sst_path(run.file_number))
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to reopen file: {}", e)))?;
                let reader_clone_arc = std::sync::Arc::new(reader_clone);
                let mut iter = reader_clone_arc.iter();

                let mut min_key: Option<Bytes> = None;
                let mut max_key: Option<Bytes> = None;
                let mut tombstone_count = 0u32;
                let mut entry_count = 0usize;
                let mut min_version = crate::Version::new(u64::MAX, u64::MAX);
                let mut max_version = crate::Version::new(0, 0);

                while let Some(entry) = iter
                    .try_next()
                    .await
                    .map_err(|e| Error::Internal(format!("Iterator error: {}", e)))?
                {
                    // Check if entry is within slot bounds
                    let in_range = match (&slot_min, &slot_max) {
                        (Some(min), Some(max)) => {
                            entry.key.as_ref() >= min.as_ref()
                                && entry.key.as_ref() < max.as_ref()
                        }
                        (Some(min), None) => entry.key.as_ref() >= min.as_ref(),
                        (None, Some(max)) => entry.key.as_ref() < max.as_ref(),
                        (None, None) => true,
                    };

                    if !in_range {
                        continue;
                    }

                    builder
                        .add(&entry)
                        .await
                        .map_err(|e| Error::Internal(format!("Failed to add entry: {}", e)))?;

                    if min_key.is_none() {
                        min_key = Some(entry.key.clone());
                    }
                    max_key = Some(entry.key.clone());

                    if entry.tombstone {
                        tombstone_count += 1;
                    }

                    let entry_version = crate::Version::new(entry.term, entry.index);
                    min_version = min_version.min(entry_version);
                    max_version = max_version.max(entry_version);
                    entry_count += 1;
                }

                // Skip empty fragments
                if entry_count == 0 {
                    println!("      Slot {} fragment is empty, skipping", slot_id);
                    continue;
                }

                // Finalize fragment
                let metadata = builder
                    .finish()
                    .await
                    .map_err(|e| Error::Internal(format!("Failed to finish fragment: {}", e)))?;

                println!(
                    "      Slot {} fragment: {} entries, {} bytes (file {})",
                    slot_id, entry_count, metadata.file_size, new_file_number
                );

                total_bytes_written += metadata.file_size;

                // Create run metadata for fragment
                let fragment_run = crate::manifest::RunMeta {
                    file_number: new_file_number,
                    size: metadata.file_size,
                    min_key: min_key.ok_or_else(|| Error::Internal("No min key".to_string()))?,
                    max_key: max_key.ok_or_else(|| Error::Internal("No max key".to_string()))?,
                    min_version,
                    max_version,
                    tombstone_count,
                    filter_fp: 0.001,
                    heat_hint: 0.0,
                    value_log_segment_id: None,
                };

                edits.push(ManifestEdit::AddFile {
                    level,
                    slot_id: Some(*slot_id),
                    run: fragment_run,
                });
            }

            // Delete original file
            edits.push(ManifestEdit::DeleteFile {
                level,
                slot_id: None, // Don't specify old slot
                file_number: run.file_number,
            });
        }

        // Finally, update the guards themselves
        edits.push(ManifestEdit::UpdateGuards {
            level,
            guards: new_guards.clone(),
        });

        println!(
            "=== GUARD MOVE COMPLETE: {} files reorganized, {} bytes written ===",
            all_runs.len(),
            total_bytes_written
        );

        Ok((edits, total_bytes_written))
    }

    /// Returns the SSTable file path for a given file number.
    fn sst_path(&self, file_number: u64) -> PathBuf {
        self.sst_dir.join(format!("sst-{:06}.sst", file_number))
    }

    /// Allocates a new file number from the manifest.
    fn allocate_file_number(
        manifest: &Arc<parking_lot::RwLock<crate::manifest::ManifestLog>>,
    ) -> u64 {
        let manifest_guard = manifest.read();
        let snapshot = manifest_guard.snapshot();
        let mut snap_guard = snapshot.write();
        snap_guard.alloc_file_number()
    }
}

/// Compaction coordinator that runs background compaction tasks.
///
/// Manages a pool of compaction workers, selects actions via the bandit scheduler,
/// and respects IO budget constraints.
///
/// # Concurrency Model
/// - Spawns multiple concurrent compaction tasks (up to max_background_compactions)
/// - Uses shared scheduler (Arc<Mutex<BanditScheduler>>) for reward updates
/// - Automatically cleans up old SSTable files after successful compaction
#[allow(dead_code)]
pub struct CompactionCoordinator {
    /// Bandit scheduler for action selection (shared for reward updates from tasks)
    scheduler: Arc<parking_lot::Mutex<BanditScheduler>>,

    /// Compaction executor
    executor: CompactionExecutor,

    /// Heat tracker reference
    heat_tracker: Arc<HeatTracker>,

    /// Manifest reference
    manifest: Arc<parking_lot::RwLock<ManifestLog>>,

    /// SSTable directory for file cleanup
    sst_dir: std::path::PathBuf,

    /// Configuration for adaptive behavior
    config: ATLLConfig,

    /// Maximum concurrent background compactions
    max_concurrent: usize,

    /// Current active compactions
    active_count: Arc<parking_lot::RwLock<usize>>,

    /// Shutdown signal
    shutdown: Arc<parking_lot::RwLock<bool>>,
}

#[allow(dead_code)]
impl CompactionCoordinator {
    /// Creates a new compaction coordinator.
    pub fn new(
        sst_dir: impl AsRef<std::path::Path>,
        config: ATLLConfig,
        heat_tracker: Arc<HeatTracker>,
        manifest: Arc<parking_lot::RwLock<ManifestLog>>,
    ) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();
        let max_concurrent = config.io.max_background_compactions;
        let meter = Arc::new(nori_observe::NoopMeter);
        let scheduler = Arc::new(parking_lot::Mutex::new(BanditScheduler::new(
            config.clone(),
            meter,
        )));
        let executor = CompactionExecutor::new(&sst_dir, config.clone())?;

        Ok(Self {
            scheduler,
            executor,
            heat_tracker,
            manifest,
            sst_dir,
            config,
            max_concurrent,
            active_count: Arc::new(parking_lot::RwLock::new(0)),
            shutdown: Arc::new(parking_lot::RwLock::new(false)),
        })
    }

    /// Calculates adaptive polling interval based on L0 pressure.
    ///
    /// Uses L0 file count as a proxy for system pressure:
    /// - Green zone (0-49% of max): 500ms (relaxed)
    /// - Yellow zone (50-74% of max): 200ms (normal)
    /// - Orange zone (75-89% of max): 100ms (aggressive)
    /// - Red zone (90%+ of max): 50ms (emergency)
    fn adaptive_interval(&self, l0_count: usize) -> std::time::Duration {
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        let interval_ms = if pressure_ratio >= 0.90 {
            50 // Red zone: emergency
        } else if pressure_ratio >= 0.75 {
            100 // Orange zone: aggressive
        } else if pressure_ratio >= 0.50 {
            200 // Yellow zone: normal (default)
        } else {
            500 // Green zone: relaxed
        };

        std::time::Duration::from_millis(interval_ms)
    }

    /// Calculates adaptive L0 compaction trigger threshold based on pressure.
    ///
    /// More aggressive thresholds under pressure to prevent L0 accumulation:
    /// - Green zone (0-49% of max): 4 files (relaxed)
    /// - Yellow zone (50-74% of max): 3 files (normal)
    /// - Orange/Red zone (75%+ of max): 2 files (aggressive)
    fn adaptive_l0_threshold(&self, l0_count: usize) -> usize {
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        if pressure_ratio >= 0.75 {
            2 // Orange/Red zone: aggressive compaction
        } else if pressure_ratio >= 0.50 {
            3 // Yellow zone: moderate compaction
        } else {
            4 // Green zone: relaxed (default)
        }
    }

    /// Calculates adaptive maximum concurrent compactions based on L0 pressure.
    ///
    /// Increases parallelism under pressure to process compactions faster:
    /// - Green/Yellow zone (0-74% of max): 2 concurrent (default)
    /// - Orange zone (75-89% of max): 3 concurrent (increased)
    /// - Red zone (90%+ of max): 4 concurrent (emergency)
    fn adaptive_max_concurrent(&self, l0_count: usize) -> usize {
        let max_files = self.config.l0.max_files;
        let pressure_ratio = l0_count as f64 / max_files as f64;

        if pressure_ratio >= 0.90 {
            4 // Red zone: emergency parallelism
        } else if pressure_ratio >= 0.75 {
            3 // Orange zone: increased parallelism
        } else {
            2 // Green/Yellow zone: default parallelism
        }
    }

    /// Starts the background compaction loop.
    ///
    /// Returns a join handle that can be awaited for graceful shutdown.
    pub fn start(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                // Calculate adaptive interval based on current L0 pressure
                let (interval, l0_count) = {
                    let manifest_guard = self.manifest.read();
                    let snapshot = manifest_guard.snapshot();
                    let snapshot_guard = snapshot.read();
                    let l0_count = snapshot_guard.l0_files().len();
                    let interval = self.adaptive_interval(l0_count);
                    (interval, l0_count)
                };

                // Sleep for adaptive duration
                tokio::time::sleep(interval).await;

                // Log adaptive interval for observability
                tracing::trace!(
                    l0_count = l0_count,
                    interval_ms = interval.as_millis(),
                    "Adaptive compaction interval"
                );

                // Check shutdown signal
                if *self.shutdown.read() {
                    break;
                }

                // Check if we can start a new compaction (adaptive concurrency limit)
                let active = *self.active_count.read();
                let max_concurrent = self.adaptive_max_concurrent(l0_count);

                tracing::trace!(
                    active = active,
                    max_concurrent = max_concurrent,
                    l0_count = l0_count,
                    "Adaptive concurrency limit"
                );

                if active >= max_concurrent {
                    continue; // At capacity, wait
                }

                // Select compaction action using proper snapshot access
                let action = {
                    let manifest_guard = self.manifest.read();
                    let snapshot = manifest_guard.snapshot();
                    let snapshot_guard = snapshot.read();
                    let scheduler_guard = self.scheduler.lock();
                    scheduler_guard.select_action(&snapshot_guard, &self.heat_tracker)
                };

                if matches!(action, CompactionAction::DoNothing) {
                    continue; // No work to do
                }

                // Increment active count
                *self.active_count.write() += 1;

                // Spawn compaction task
                let executor = self.executor.clone_for_task();
                let manifest_clone = self.manifest.clone();
                let scheduler_clone = self.scheduler.clone();
                let heat_tracker = self.heat_tracker.clone();
                let active_count = self.active_count.clone();
                let sst_dir = self.sst_dir.clone();
                let action_clone = action.clone();

                tokio::spawn(async move {
                    // Execute compaction
                    let result = executor
                        .execute_action(&action_clone, &manifest_clone)
                        .await;

                    match result {
                        Ok((edits, bytes_written)) => {
                            // Track old files for deletion
                            let mut old_files = Vec::new();
                            for edit in &edits {
                                if let crate::manifest::ManifestEdit::DeleteFile {
                                    file_number,
                                    ..
                                } = edit
                                {
                                    old_files.push(*file_number);
                                }
                            }

                            // Apply manifest edits
                            let edit_result = {
                                let mut manifest = manifest_clone.write();
                                edits.into_iter().try_for_each(|edit| manifest.append(edit))
                            };

                            if edit_result.is_ok() {
                                // Delete old SSTable files
                                for file_number in old_files {
                                    let file_path =
                                        sst_dir.join(format!("sst-{:06}.sst", file_number));
                                    let _ = std::fs::remove_file(file_path);
                                }

                                // Update scheduler rewards
                                let latency_reduction =
                                    Self::estimate_latency_reduction(&action_clone);
                                let (level, slot_id) = Self::extract_slot(&action_clone);
                                let heat_score = heat_tracker.get_heat(level, slot_id);

                                // Get current L0 count for pressure-weighted rewards
                                let l0_count = {
                                    let manifest_guard = manifest_clone.read();
                                    let snapshot = manifest_guard.snapshot();
                                    let snapshot_guard = snapshot.read();
                                    snapshot_guard.l0_file_count()
                                };

                                let mut scheduler = scheduler_clone.lock();
                                scheduler.update_reward(
                                    &action_clone,
                                    bytes_written,
                                    latency_reduction,
                                    heat_score,
                                    l0_count,
                                );

                                tracing::debug!(
                                    "Completed compaction {:?}: {} bytes written",
                                    action_clone,
                                    bytes_written
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Compaction failed for {:?}: {}", action_clone, e);
                        }
                    }

                    // Decrement active count
                    *active_count.write() -= 1;
                });
            }
        })
    }

    /// Signals the coordinator to shutdown gracefully.
    pub fn shutdown(&self) {
        *self.shutdown.write() = true;
    }

    /// Waits for all active compactions to complete.
    pub async fn wait_for_idle(&self) {
        loop {
            let active = *self.active_count.read();
            if active == 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Estimates latency reduction from a compaction action (heuristic).
    fn estimate_latency_reduction(action: &CompactionAction) -> f64 {
        match action {
            CompactionAction::Tier { run_count, .. } => (*run_count as f64) * 0.5, // ~0.5ms per run merged
            CompactionAction::EagerLevel { .. } => 2.0, // Significant benefit for hot slots
            CompactionAction::Cleanup { .. } => 1.0,    // Moderate benefit
            CompactionAction::Promote { .. } => 0.1,    // Small benefit
            _ => 0.0,
        }
    }

    /// Extracts (level, slot_id) from an action.
    fn extract_slot(action: &CompactionAction) -> (u8, u32) {
        match action {
            CompactionAction::Tier { level, slot_id, .. }
            | CompactionAction::Promote { level, slot_id, .. }
            | CompactionAction::EagerLevel { level, slot_id }
            | CompactionAction::Cleanup { level, slot_id } => (*level, *slot_id),
            _ => (0, 0),
        }
    }
}

/// Helper trait for executor cloning in async tasks.
trait ExecutorClone {
    fn clone_for_task(&self) -> Self;
    async fn execute_action(
        &self,
        action: &CompactionAction,
        manifest: &Arc<parking_lot::RwLock<ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)>;
}

impl ExecutorClone for CompactionExecutor {
    fn clone_for_task(&self) -> Self {
        Self {
            sst_dir: self.sst_dir.clone(),
            config: self.config.clone(),
        }
    }

    async fn execute_action(
        &self,
        action: &CompactionAction,
        manifest: &Arc<parking_lot::RwLock<ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        // Execute the compaction action using the main execute method
        self.execute(action, manifest).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flush::Flusher;
    use crate::memtable::Memtable;
    use crate::Version;

    #[test]
    fn test_bandit_arm_creation() {
        let arm = BanditArm::new(1, 5);
        assert_eq!(arm.level, 1);
        assert_eq!(arm.slot_id, 5);
        assert_eq!(arm.selection_count, 0);
        assert_eq!(arm.avg_reward, 0.0);
    }

    #[test]
    fn test_bandit_arm_update() {
        let mut arm = BanditArm::new(1, 0);

        arm.update(1.0);
        assert_eq!(arm.selection_count, 1);
        assert_eq!(arm.avg_reward, 1.0);

        arm.update(0.5);
        assert_eq!(arm.selection_count, 2);
        // EMA: 0.1 * 0.5 + 0.9 * 1.0 = 0.95
        assert!((arm.avg_reward - 0.95).abs() < 0.01);
    }

    #[test]
    fn test_bandit_arm_ucb() {
        let mut arm = BanditArm::new(1, 0);
        arm.update(1.0);

        let score = arm.ucb_score(10, 2.0);
        assert!(score > 1.0); // Should have exploration bonus
    }

    #[test]
    fn test_bandit_arm_ucb_unvisited() {
        let arm = BanditArm::new(1, 0);
        let score = arm.ucb_score(10, 2.0);
        assert_eq!(score, f64::INFINITY); // Unvisited arms have infinite priority
    }

    #[test]
    fn test_compaction_action_equality() {
        let action1 = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 3,
        };
        let action2 = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 3,
        };
        assert_eq!(action1, action2);
    }

    #[test]
    fn test_bandit_scheduler_creation() {
        let config = ATLLConfig::default();
        let meter = Arc::new(nori_observe::NoopMeter);
        let scheduler = BanditScheduler::new(config, meter);
        assert_eq!(scheduler.epsilon, 0.1);
        assert_eq!(scheduler.total_selections, 0);
    }

    #[test]
    fn test_bandit_scheduler_reward_update() {
        let config = ATLLConfig::default();
        let meter = Arc::new(nori_observe::NoopMeter);
        let mut scheduler = BanditScheduler::new(config, meter);

        let action = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 3,
        };

        scheduler.update_reward(&action, 1024, 5.0, 0.8, 5); // l0_count = 5 (mid-range)

        let arm = scheduler.arms.get(&(1, 0)).unwrap();
        assert_eq!(arm.selection_count, 1);
        assert!(arm.avg_reward > 0.0);
        assert_eq!(scheduler.total_selections, 1);
    }

    // Test meter that captures VizEvents for analysis
    struct CapturingMeter {
        events: Arc<parking_lot::Mutex<Vec<nori_observe::VizEvent>>>,
    }

    impl CapturingMeter {
        fn new() -> Self {
            Self {
                events: Arc::new(parking_lot::Mutex::new(Vec::new())),
            }
        }

        fn get_events(&self) -> Vec<nori_observe::VizEvent> {
            self.events.lock().clone()
        }
    }

    struct TestCounter;
    impl nori_observe::Counter for TestCounter {
        fn inc(&self, _v: u64) {}
    }

    struct TestGauge;
    impl nori_observe::Gauge for TestGauge {
        fn set(&self, _v: i64) {}
    }

    struct TestHistogram;
    impl nori_observe::Histogram for TestHistogram {
        fn observe(&self, _v: f64) {}
    }

    impl nori_observe::Meter for CapturingMeter {
        fn counter(
            &self,
            _name: &'static str,
            _labels: &'static [(&'static str, &'static str)],
        ) -> Box<dyn nori_observe::Counter> {
            Box::new(TestCounter)
        }

        fn gauge(
            &self,
            _name: &'static str,
            _labels: &'static [(&'static str, &'static str)],
        ) -> Box<dyn nori_observe::Gauge> {
            Box::new(TestGauge)
        }

        fn histo(
            &self,
            _name: &'static str,
            _buckets: &'static [f64],
            _labels: &'static [(&'static str, &'static str)],
        ) -> Box<dyn nori_observe::Histogram> {
            Box::new(TestHistogram)
        }

        fn emit(&self, evt: nori_observe::VizEvent) {
            self.events.lock().push(evt);
        }
    }

    #[test]
    fn test_bandit_viz_event_emission() {
        use nori_observe::CompKind;

        let config = ATLLConfig::default();
        let meter = Arc::new(CapturingMeter::new());
        let mut scheduler = BanditScheduler::new(config, meter.clone());

        // Simulate multiple compaction actions and rewards
        let actions = vec![
            CompactionAction::Tier {
                level: 1,
                slot_id: 0,
                run_count: 3,
            },
            CompactionAction::Tier {
                level: 1,
                slot_id: 1,
                run_count: 3,
            },
        ];

        // Apply rewards for different actions
        for action in &actions {
            for _ in 0..10 {
                scheduler.update_reward(action, 1024, 5.0, 0.8, 5); // l0_count = 5
            }
        }

        // Analyze captured events
        let events = meter.get_events();

        // Count reward events
        let reward_events: Vec<_> = events
            .iter()
            .filter(|evt| {
                matches!(
                    evt,
                    nori_observe::VizEvent::Compaction(nori_observe::CompEvt {
                        kind: CompKind::BanditReward { .. },
                        ..
                    })
                )
            })
            .collect();

        assert_eq!(
            reward_events.len(),
            20,
            "Should have 20 reward events (10 per action)"
        );

        // Verify reward event structure
        for event in &reward_events {
            if let nori_observe::VizEvent::Compaction(comp_evt) = event {
                if let CompKind::BanditReward {
                    slot_id,
                    reward,
                    bytes_written,
                    heat_score,
                } = comp_evt.kind
                {
                    assert!(slot_id <= 1, "Slot ID should be 0 or 1");
                    assert!(reward > 0.0, "Reward should be positive");
                    assert_eq!(bytes_written, 1024, "Bytes written should match");
                    assert_eq!(heat_score, 0.8, "Heat score should match");
                }
            }
        }
    }

    #[test]
    fn test_bandit_reward_convergence() {
        use nori_observe::CompKind;

        let config = ATLLConfig::default();
        let meter = Arc::new(CapturingMeter::new());
        let mut scheduler = BanditScheduler::new(config, meter.clone());

        let action = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 3,
        };

        // Apply consistent rewards
        for _ in 0..50 {
            scheduler.update_reward(&action, 1024, 10.0, 0.5, 5); // l0_count = 5
        }

        // Check reward events
        let events = meter.get_events();
        let reward_events: Vec<_> = events
            .iter()
            .filter_map(|evt| {
                if let nori_observe::VizEvent::Compaction(comp_evt) = evt {
                    if let CompKind::BanditReward { reward, .. } = comp_evt.kind {
                        Some(reward)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        assert!(reward_events.len() > 0, "Should have reward events");

        // Check that rewards are converging (later rewards should be similar)
        if reward_events.len() >= 10 {
            let last_10: Vec<f64> = reward_events.iter().rev().take(10).copied().collect();
            let avg_recent = last_10.iter().sum::<f64>() / last_10.len() as f64;

            // All recent rewards should be within 20% of average (convergence)
            for &reward in &last_10 {
                let diff = (reward - avg_recent).abs() / avg_recent;
                assert!(
                    diff < 0.2,
                    "Recent rewards should converge: {} vs avg {}",
                    reward,
                    avg_recent
                );
            }
        }
    }

    #[tokio::test]
    async fn test_multiway_merger_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("merged.sst");

        let config = ATLLConfig::default();

        let merger = MultiWayMerger::new(vec![], output_path, false, &config, None)
            .await
            .unwrap();

        let result = merger.merge().await;
        assert!(result.is_err()); // Should fail with no min/max key
    }

    #[tokio::test]
    async fn test_multiway_merger_single_sstable() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create a memtable with entries
        let mt = Memtable::new(Version::new(0, 1));
        mt.put(Bytes::from("key1"), Bytes::from("value1"), Version::new(0, 1), None)
            .unwrap();
        mt.put(Bytes::from("key2"), Bytes::from("value2"), Version::new(0, 2), None)
            .unwrap();
        mt.put(Bytes::from("key3"), Bytes::from("value3"), Version::new(0, 3), None)
            .unwrap();

        // Flush to SSTable
        let _run = flusher.flush_to_l0(&mt, 1).await.unwrap();

        // Merge single SSTable
        let input_path = sst_dir.join("sst-000001.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input_path], output_path, false, &config, None)
            .await
            .unwrap();

        let result = merger.merge().await.unwrap();

        assert_eq!(result.min_key, Bytes::from("key1"));
        assert_eq!(result.max_key, Bytes::from("key3"));
        assert_eq!(result.tombstone_count, 0);
    }

    #[tokio::test]
    async fn test_multiway_merger_multiple_sstables() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create first memtable (keys 1, 3, 5)
        let mt1 = Memtable::new(Version::new(0, 1));
        mt1.put(Bytes::from("key1"), Bytes::from("value1"), Version::new(0, 1), None)
            .unwrap();
        mt1.put(Bytes::from("key3"), Bytes::from("value3"), Version::new(0, 3), None)
            .unwrap();
        mt1.put(Bytes::from("key5"), Bytes::from("value5"), Version::new(0, 5), None)
            .unwrap();

        flusher.flush_to_l0(&mt1, 1).await.unwrap();

        // Create second memtable (keys 2, 4, 6)
        let mt2 = Memtable::new(Version::new(0, 10));
        mt2.put(Bytes::from("key2"), Bytes::from("value2"), Version::new(0, 10), None)
            .unwrap();
        mt2.put(Bytes::from("key4"), Bytes::from("value4"), Version::new(0, 11), None)
            .unwrap();
        mt2.put(Bytes::from("key6"), Bytes::from("value6"), Version::new(0, 12), None)
            .unwrap();

        flusher.flush_to_l0(&mt2, 2).await.unwrap();

        // Merge both SSTables
        let input1 = sst_dir.join("sst-000001.sst");
        let input2 = sst_dir.join("sst-000002.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input1, input2], output_path.clone(), false, &config, None)
            .await
            .unwrap();

        let result = merger.merge().await.unwrap();

        assert_eq!(result.min_key, Bytes::from("key1"));
        assert_eq!(result.max_key, Bytes::from("key6"));

        // Verify merged SSTable contains all keys in sorted order
        let reader = Arc::new(SSTableReader::open(output_path).await.unwrap());
        let mut iter = reader.iter();

        let mut keys = Vec::new();
        while let Some(entry) = iter.try_next().await.unwrap() {
            keys.push(entry.key);
        }

        assert_eq!(keys.len(), 6);
        assert_eq!(keys[0], Bytes::from("key1"));
        assert_eq!(keys[1], Bytes::from("key2"));
        assert_eq!(keys[2], Bytes::from("key3"));
        assert_eq!(keys[3], Bytes::from("key4"));
        assert_eq!(keys[4], Bytes::from("key5"));
        assert_eq!(keys[5], Bytes::from("key6"));
    }

    #[tokio::test]
    async fn test_multiway_merger_with_tombstones() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create memtable with tombstones
        let mt = Memtable::new(Version::new(0, 1));
        mt.put(Bytes::from("key1"), Bytes::from("value1"), Version::new(0, 1), None)
            .unwrap();
        mt.delete(Bytes::from("key2"), Version::new(0, 2)).unwrap();
        mt.put(Bytes::from("key3"), Bytes::from("value3"), Version::new(0, 3), None)
            .unwrap();

        flusher.flush_to_l0(&mt, 1).await.unwrap();

        // Merge with can_drop_tombstones = false
        let input_path = sst_dir.join("sst-000001.sst");
        let output_path1 = temp_dir.path().join("merged_keep.sst");

        let merger = MultiWayMerger::new(
            vec![input_path.clone()],
            output_path1.clone(),
            false,
            &config,
            None,
        )
        .await
        .unwrap();

        let result1 = merger.merge().await.unwrap();
        assert_eq!(result1.tombstone_count, 1);

        // Verify tombstone was kept
        let reader1 = Arc::new(SSTableReader::open(output_path1).await.unwrap());
        let mut iter1 = reader1.iter();
        let mut entry_count = 0;
        while let Some(_entry) = iter1.try_next().await.unwrap() {
            entry_count += 1;
        }
        assert_eq!(entry_count, 3); // key1, key2 (tombstone), key3

        // Merge with can_drop_tombstones = true
        let output_path2 = temp_dir.path().join("merged_drop.sst");

        let merger2 = MultiWayMerger::new(vec![input_path], output_path2.clone(), true, &config, None)
            .await
            .unwrap();

        let result2 = merger2.merge().await.unwrap();
        assert_eq!(result2.tombstone_count, 1); // Still tracked

        // Verify tombstone was dropped
        let reader2 = Arc::new(SSTableReader::open(output_path2).await.unwrap());
        let mut iter2 = reader2.iter();
        let mut entry_count2 = 0;
        while let Some(_entry) = iter2.try_next().await.unwrap() {
            entry_count2 += 1;
        }
        assert_eq!(entry_count2, 2); // Only key1, key3 (key2 tombstone dropped)
    }

    #[tokio::test]
    async fn test_multiway_merger_deduplication() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create two memtables with overlapping keys (newer one should win)
        let mt1 = Memtable::new(Version::new(0, 1));
        mt1.put(Bytes::from("key1"), Bytes::from("old_value"), Version::new(0, 1), None)
            .unwrap();

        flusher.flush_to_l0(&mt1, 1).await.unwrap();

        let mt2 = Memtable::new(Version::new(0, 10));
        mt2.put(Bytes::from("key1"), Bytes::from("new_value"), Version::new(0, 10), None)
            .unwrap();

        flusher.flush_to_l0(&mt2, 2).await.unwrap();

        // Merge (mt2 has higher index, should be newer)
        let input1 = sst_dir.join("sst-000001.sst");
        let input2 = sst_dir.join("sst-000002.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input1, input2], output_path.clone(), false, &config, None)
            .await
            .unwrap();

        merger.merge().await.unwrap();

        // Verify only one entry with newest value
        let reader = Arc::new(SSTableReader::open(output_path).await.unwrap());
        let mut iter = reader.iter();

        let mut entries = Vec::new();
        while let Some(entry) = iter.try_next().await.unwrap() {
            entries.push(entry);
        }

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, Bytes::from("key1"));
        assert_eq!(entries[0].value, Bytes::from("new_value")); // Newest wins
    }

    #[test]
    fn test_executor_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let _executor = CompactionExecutor::new(&sst_dir, config).unwrap();

        assert!(sst_dir.exists());
    }

    #[test]
    fn test_executor_file_number_allocation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        let manifest_dir = temp_dir.path().join("manifest");

        let config = ATLLConfig::default();
        let _executor = CompactionExecutor::new(&sst_dir, config.clone()).unwrap();

        let manifest = Arc::new(parking_lot::RwLock::new(
            ManifestLog::open(&manifest_dir, config.max_levels).unwrap(),
        ));

        let num1 = CompactionExecutor::allocate_file_number(&manifest);
        let num2 = CompactionExecutor::allocate_file_number(&manifest);
        let num3 = CompactionExecutor::allocate_file_number(&manifest);

        // File numbers should be sequential
        assert!(num2 > num1);
        assert!(num3 > num2);
    }

    #[test]
    fn test_compaction_coordinator_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        let manifest_dir = temp_dir.path().join("manifest");

        let config = ATLLConfig::default();
        let heat_tracker = Arc::new(HeatTracker::new(config.clone()));
        let manifest = Arc::new(parking_lot::RwLock::new(
            ManifestLog::open(&manifest_dir, config.max_levels).unwrap(),
        ));

        let coordinator = CompactionCoordinator::new(&sst_dir, config, heat_tracker, manifest);
        assert!(coordinator.is_ok());

        let coord = coordinator.unwrap();
        assert_eq!(coord.max_concurrent, 4); // Default from config
    }

    #[tokio::test]
    async fn test_compaction_coordinator_shutdown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        let manifest_dir = temp_dir.path().join("manifest");

        let config = ATLLConfig::default();
        let heat_tracker = Arc::new(HeatTracker::new(config.clone()));
        let manifest = Arc::new(parking_lot::RwLock::new(
            ManifestLog::open(&manifest_dir, config.max_levels).unwrap(),
        ));

        let coordinator =
            CompactionCoordinator::new(&sst_dir, config, heat_tracker, manifest).unwrap();

        // Get shutdown handle before starting
        let shutdown_signal = coordinator.shutdown.clone();

        // Start coordinator (consumes self)
        let handle = coordinator.start();

        // Let it run briefly
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Shutdown via cloned signal
        *shutdown_signal.write() = true;

        // Wait for completion
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), handle).await;
    }

    #[test]
    fn test_estimate_latency_reduction() {
        let tier_action = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 4,
        };
        let reduction = CompactionCoordinator::estimate_latency_reduction(&tier_action);
        assert_eq!(reduction, 2.0); // 4 * 0.5

        let eager_action = CompactionAction::EagerLevel {
            level: 1,
            slot_id: 0,
        };
        let reduction = CompactionCoordinator::estimate_latency_reduction(&eager_action);
        assert_eq!(reduction, 2.0);

        let cleanup_action = CompactionAction::Cleanup {
            level: 1,
            slot_id: 0,
        };
        let reduction = CompactionCoordinator::estimate_latency_reduction(&cleanup_action);
        assert_eq!(reduction, 1.0);
    }

    #[test]
    fn test_extract_slot() {
        let action = CompactionAction::Tier {
            level: 2,
            slot_id: 5,
            run_count: 3,
        };
        let (level, slot_id) = CompactionCoordinator::extract_slot(&action);
        assert_eq!(level, 2);
        assert_eq!(slot_id, 5);

        let promote_action = CompactionAction::Promote {
            level: 1,
            slot_id: 3,
            file_number: 100,
        };
        let (level, slot_id) = CompactionCoordinator::extract_slot(&promote_action);
        assert_eq!(level, 1);
        assert_eq!(slot_id, 3);
    }

    #[test]
    fn test_adaptive_interval() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config = ATLLConfig::default();
        let max_files = config.l0.max_files; // Default is 12
        let max_levels = config.max_levels;

        let heat_tracker = Arc::new(HeatTracker::new(config.clone()));
        let manifest = Arc::new(parking_lot::RwLock::new(
            ManifestLog::open(temp_dir.path(), max_levels).unwrap(),
        ));

        let coordinator = CompactionCoordinator::new(
            temp_dir.path(),
            config,
            heat_tracker,
            manifest,
        )
        .unwrap();

        // Test green zone (0-49% of max_files): should be 500ms
        let l0_count = (max_files as f64 * 0.3) as usize; // 30%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 500, "Green zone should be 500ms");

        // Test yellow zone (50-74% of max_files): should be 200ms
        let l0_count = (max_files as f64 * 0.6) as usize; // 60%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 200, "Yellow zone should be 200ms");

        // Test orange zone (75-89% of max_files): should be 100ms
        let l0_count = (max_files as f64 * 0.8) as usize; // 80%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 100, "Orange zone should be 100ms");

        // Test red zone (90%+ of max_files): should be 50ms
        let l0_count = (max_files as f64 * 0.95) as usize; // 95%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 50, "Red zone should be 50ms");

        // Test boundary: exactly at 50% threshold (6/12 = 0.50)
        let l0_count = max_files / 2; // Exactly 50%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 200, "50% threshold should be yellow (200ms)");

        // Test boundary: just below 75% threshold (8/12 = 0.666)
        let l0_count = (max_files as f64 * 0.74) as usize;
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 200, "Just below 75% should still be yellow (200ms)");

        // Test boundary: at 75% threshold (9/12 = 0.75)
        let l0_count = (max_files * 3) / 4; // Exactly 75%
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 100, "75% threshold should be orange (100ms)");

        // Test boundary: just below 90% threshold (10/12 = 0.833)
        let l0_count = (max_files as f64 * 0.89) as usize;
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 100, "Just below 90% should still be orange (100ms)");

        // Test boundary: at 90% threshold (11/12 = 0.916)
        let l0_count = max_files - 1; // One below max, definitely >= 0.90
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 50, "90% threshold should be red (50ms)");

        // Test at max capacity
        let l0_count = max_files;
        let interval = coordinator.adaptive_interval(l0_count);
        assert_eq!(interval.as_millis(), 50, "At max capacity should be red (50ms)");
    }

    #[test]
    fn test_adaptive_slice_size() {
        let max_files = 12; // Default max_files
        let default_mb = 64; // Default compaction slice size

        // Test green zone (0-49% of max_files): should be 128MB
        let l0_count = 3; // 25% of 12
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            128 * 1024 * 1024,
            "Green zone should be 128MB"
        );

        // Test yellow zone (50-74% of max_files): should be default (64MB)
        let l0_count = 7; // 58% of 12
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            64 * 1024 * 1024,
            "Yellow zone should use default (64MB)"
        );

        // Test boundary: at 50% threshold
        let l0_count = 6; // Exactly 50%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            64 * 1024 * 1024,
            "50% threshold should be yellow (64MB default)"
        );

        // Test boundary: just below 75% threshold (8/12 = 0.666)
        let l0_count = 8; // 66.6%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            64 * 1024 * 1024,
            "Just below 75% should still be yellow (64MB)"
        );

        // Test boundary: at 75% threshold (9/12 = 0.75)
        let l0_count = 9; // Exactly 75%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            32 * 1024 * 1024,
            "75% threshold should be orange (32MB)"
        );

        // Test boundary: just below 90% threshold (10/12 = 0.833)
        let l0_count = 10; // 83.3%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            32 * 1024 * 1024,
            "Just below 90% should still be orange (32MB)"
        );

        // Test boundary: at 90% threshold (11/12 = 0.916)
        let l0_count = 11; // 91.6%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            16 * 1024 * 1024,
            "90% threshold should be red (16MB)"
        );

        // Test at max capacity
        let l0_count = 12; // 100%
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            16 * 1024 * 1024,
            "At max capacity should be red (16MB)"
        );

        // Test beyond max capacity (should still be red zone)
        let l0_count = 15; // 125% (over capacity)
        let slice_size = MultiWayMerger::adaptive_slice_size(l0_count, max_files, default_mb);
        assert_eq!(
            slice_size,
            16 * 1024 * 1024,
            "Over max capacity should be red (16MB)"
        );
    }

    #[test]
    fn test_adaptive_max_concurrent() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let config = ATLLConfig::default();
        let max_levels = config.max_levels;

        let heat_tracker = Arc::new(HeatTracker::new(config.clone()));
        let manifest = Arc::new(parking_lot::RwLock::new(
            ManifestLog::open(temp_dir.path(), max_levels).unwrap(),
        ));

        let coordinator = CompactionCoordinator::new(
            temp_dir.path(),
            config,
            heat_tracker,
            manifest,
        )
        .unwrap();

        // Test green zone (0-49% of max_files): should be 2 concurrent
        let l0_count = 3; // 25% of 12
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(max_concurrent, 2, "Green zone should have 2 concurrent compactions");

        // Test yellow zone (50-74% of max_files): should be 2 concurrent
        let l0_count = 7; // 58% of 12
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 2,
            "Yellow zone should have 2 concurrent compactions"
        );

        // Test boundary: at 50% threshold
        let l0_count = 6; // Exactly 50%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 2,
            "50% threshold should be 2 concurrent (default)"
        );

        // Test boundary: just below 75% threshold (8/12 = 0.666)
        let l0_count = 8; // 66.6%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 2,
            "Just below 75% should still be 2 concurrent"
        );

        // Test boundary: at 75% threshold (9/12 = 0.75)
        let l0_count = 9; // Exactly 75%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 3,
            "75% threshold should be orange (3 concurrent)"
        );

        // Test boundary: just below 90% threshold (10/12 = 0.833)
        let l0_count = 10; // 83.3%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 3,
            "Just below 90% should still be orange (3 concurrent)"
        );

        // Test boundary: at 90% threshold (11/12 = 0.916)
        let l0_count = 11; // 91.6%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 4,
            "90% threshold should be red (4 concurrent)"
        );

        // Test at max capacity
        let l0_count = 12; // 100%
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 4,
            "At max capacity should be red (4 concurrent)"
        );

        // Test beyond max capacity (should still be red zone)
        let l0_count = 15; // 125% (over capacity)
        let max_concurrent = coordinator.adaptive_max_concurrent(l0_count);
        assert_eq!(
            max_concurrent, 4,
            "Over max capacity should be red (4 concurrent)"
        );
    }

    #[test]
    fn test_adaptive_l0_threshold() {
        let config = ATLLConfig::default();
        let max_files = config.l0.max_files; // Default is 12
        let meter = Arc::new(nori_observe::NoopMeter);
        let scheduler = BanditScheduler::new(config, meter);

        // Test green zone (0-49% of max_files): threshold should be 4
        let l0_count = 3; // 25% of 12
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 4, "Green zone should have threshold of 4 files");

        // Test yellow zone (50-74% of max_files): threshold should be 3
        let l0_count = 7; // 58% of 12
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 3, "Yellow zone should have threshold of 3 files");

        // Test orange/red zone (75%+ of max_files): threshold should be 2
        let l0_count = 10; // 83% of 12
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 2, "Orange/Red zone should have threshold of 2 files");

        // Test boundary: exactly at 50% threshold (6/12 = 0.50)
        let l0_count = max_files / 2;
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 3, "50% threshold should be yellow (3 files)");

        // Test boundary: exactly at 75% threshold (9/12 = 0.75)
        let l0_count = (max_files * 3) / 4;
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 2, "75% threshold should be orange (2 files)");

        // Test boundary: just below 75% (8/12 = 0.666)
        let l0_count = 8;
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 3, "Just below 75% should be yellow (3 files)");

        // Test near max capacity (11/12 = 0.916)
        let l0_count = max_files - 1;
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 2, "Near max should be aggressive (2 files)");

        // Test at max capacity
        let l0_count = max_files;
        let threshold = scheduler.adaptive_l0_threshold(l0_count);
        assert_eq!(threshold, 2, "At max should be aggressive (2 files)");
    }

    #[test]
    fn test_adaptive_epsilon() {
        let config = ATLLConfig::default();
        let max_files = config.l0.max_files; // Default is 12
        let meter = Arc::new(nori_observe::NoopMeter);
        let scheduler = BanditScheduler::new(config, meter);

        // Test green zone (0-49% of max_files): epsilon should be 0.10 (10%)
        let l0_count = 3; // 25% of 12
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.10, "Green zone should have 10% exploration");

        // Test yellow zone (50-74% of max_files): epsilon should be 0.05 (5%)
        let l0_count = 7; // 58% of 12
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.05, "Yellow zone should have 5% exploration");

        // Test orange zone (75-89% of max_files): epsilon should be 0.02 (2%)
        let l0_count = 10; // 83% of 12
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.02, "Orange zone should have 2% exploration");

        // Test red zone (90%+ of max_files): epsilon should be 0.0 (pure exploitation)
        let l0_count = 11; // 91.6% of 12
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.0, "Red zone should have 0% exploration (pure exploitation)");

        // Test boundary: exactly at 50% threshold (6/12 = 0.50)
        let l0_count = max_files / 2;
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.05, "50% threshold should be yellow (5%)");

        // Test boundary: exactly at 75% threshold (9/12 = 0.75)
        let l0_count = (max_files * 3) / 4;
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.02, "75% threshold should be orange (2%)");

        // Test boundary: just below 75% (8/12 = 0.666)
        let l0_count = 8;
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.05, "Just below 75% should be yellow (5%)");

        // Test boundary: exactly at 90% threshold (10.8/12, round to 11)
        let l0_count = max_files - 1; // 91.6%
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.0, "90% threshold should be red (0%)");

        // Test at max capacity (12/12 = 1.0)
        let l0_count = max_files;
        let epsilon = scheduler.adaptive_epsilon(l0_count);
        assert_eq!(epsilon, 0.0, "At max capacity should be red (0%)");
    }

    // ==================== V2 FORMAT COMPACTION TESTS ====================

    #[tokio::test]
    async fn test_multiway_merger_v2_format_explicit() {
        // Test that we can explicitly request v2 format for output
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create a memtable with entries
        let mt = Memtable::new(Version::new(0, 1));
        mt.put(Bytes::from("key1"), Bytes::from("value1"), Version::new(0, 1), None)
            .unwrap();
        mt.put(Bytes::from("key2"), Bytes::from("value2"), Version::new(0, 2), None)
            .unwrap();

        // Flush to SSTable (v1 format by default)
        let _run = flusher.flush_to_l0(&mt, 1).await.unwrap();

        let input_path = sst_dir.join("sst-000001.sst");
        let output_path = temp_dir.path().join("merged_v2.sst");

        // Explicitly request v2 format output
        let merger = MultiWayMerger::new_with_filter(
            vec![input_path],
            output_path.clone(),
            false,
            &config,
            None,
            Some(FilterType::QuotientFilter),
        )
        .await
        .unwrap();

        // Verify merger knows it's using v2 format
        assert!(merger.uses_quotient_filter());
        assert_eq!(merger.output_filter_type(), FilterType::QuotientFilter);

        let result = merger.merge().await.unwrap();

        assert_eq!(result.min_key, Bytes::from("key1"));
        assert_eq!(result.max_key, Bytes::from("key2"));

        // Verify output SSTable uses v2 format
        let reader = SSTableReader::open(output_path).await.unwrap();
        assert!(reader.uses_quotient_filter());
    }

    #[tokio::test]
    async fn test_multiway_merger_v1_to_v1_format() {
        // Test that v1 inputs produce v1 output by default (backward compat)
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();
        let flusher = Flusher::new(&sst_dir, config.clone()).unwrap();

        // Create a memtable with entries
        let mt = Memtable::new(Version::new(0, 1));
        mt.put(Bytes::from("a"), Bytes::from("1"), Version::new(0, 1), None)
            .unwrap();
        mt.put(Bytes::from("b"), Bytes::from("2"), Version::new(0, 2), None)
            .unwrap();

        // Flush to SSTable (v1 format)
        let _run = flusher.flush_to_l0(&mt, 1).await.unwrap();

        let input_path = sst_dir.join("sst-000001.sst");

        // Verify input is v1
        let input_reader = SSTableReader::open(input_path.clone()).await.unwrap();
        assert!(!input_reader.uses_quotient_filter());

        let output_path = temp_dir.path().join("merged_v1.sst");

        // Default merger (auto-detect) should produce v1 from v1 inputs
        let merger = MultiWayMerger::new(
            vec![input_path],
            output_path.clone(),
            false,
            &config,
            None,
        )
        .await
        .unwrap();

        // Verify merger knows it's using v1 format
        assert!(!merger.uses_quotient_filter());
        assert_eq!(merger.output_filter_type(), FilterType::Bloom);

        let _result = merger.merge().await.unwrap();

        // Verify output SSTable uses v1 format
        let reader = SSTableReader::open(output_path).await.unwrap();
        assert!(!reader.uses_quotient_filter());
    }

    #[tokio::test]
    async fn test_multiway_merger_v2_input_propagates() {
        // Test that v2 input causes v2 output (gradual migration)
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();

        // Create a v2 SSTable directly
        let v2_path = sst_dir.join("v2_input.sst");
        let sst_config = SSTableConfig {
            path: v2_path.clone(),
            estimated_entries: 10,
            block_size: 4096,
            restart_interval: 16,
            compression: nori_sstable::Compression::None,
            bloom_bits_per_key: 10,
            block_cache_mb: 64,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(sst_config).await.unwrap();
        builder.add(&Entry::put("key1", "value1")).await.unwrap();
        builder.add(&Entry::put("key2", "value2")).await.unwrap();
        builder.finish().await.unwrap();

        // Verify input is v2
        let input_reader = SSTableReader::open(v2_path.clone()).await.unwrap();
        assert!(input_reader.uses_quotient_filter());

        let output_path = temp_dir.path().join("merged_from_v2.sst");

        // Auto-detect should choose v2 because input is v2
        let merger = MultiWayMerger::new(
            vec![v2_path],
            output_path.clone(),
            false,
            &config,
            None,
        )
        .await
        .unwrap();

        assert!(merger.uses_quotient_filter());

        let _result = merger.merge().await.unwrap();

        // Verify output SSTable uses v2 format
        let reader = SSTableReader::open(output_path).await.unwrap();
        assert!(reader.uses_quotient_filter());
    }

    #[tokio::test]
    async fn test_multiway_merger_mixed_format_inputs() {
        // Test that mixing v1 and v2 inputs produces v2 output
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();

        // Create a v1 SSTable
        let v1_path = sst_dir.join("v1_input.sst");
        let v1_config = SSTableConfig {
            path: v1_path.clone(),
            estimated_entries: 10,
            block_size: 4096,
            filter_type: FilterType::Bloom,
            ..Default::default()
        };

        let mut v1_builder = SSTableBuilder::new(v1_config).await.unwrap();
        v1_builder.add(&Entry::put("a", "1")).await.unwrap();
        v1_builder.add(&Entry::put("c", "3")).await.unwrap();
        v1_builder.finish().await.unwrap();

        // Create a v2 SSTable
        let v2_path = sst_dir.join("v2_input.sst");
        let v2_config = SSTableConfig {
            path: v2_path.clone(),
            estimated_entries: 10,
            block_size: 4096,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut v2_builder = SSTableBuilder::new(v2_config).await.unwrap();
        v2_builder.add(&Entry::put("b", "2")).await.unwrap();
        v2_builder.add(&Entry::put("d", "4")).await.unwrap();
        v2_builder.finish().await.unwrap();

        // Verify input formats
        let v1_reader = SSTableReader::open(v1_path.clone()).await.unwrap();
        let v2_reader = SSTableReader::open(v2_path.clone()).await.unwrap();
        assert!(!v1_reader.uses_quotient_filter());
        assert!(v2_reader.uses_quotient_filter());

        let output_path = temp_dir.path().join("merged_mixed.sst");

        // Auto-detect should choose v2 because one input is v2
        let merger = MultiWayMerger::new(
            vec![v1_path, v2_path],
            output_path.clone(),
            false,
            &config,
            None,
        )
        .await
        .unwrap();

        assert!(merger.uses_quotient_filter());

        let result = merger.merge().await.unwrap();

        // Verify merge result
        assert_eq!(result.min_key, Bytes::from("a"));
        assert_eq!(result.max_key, Bytes::from("d"));

        // Verify output SSTable uses v2 format
        let reader = SSTableReader::open(output_path).await.unwrap();
        assert!(reader.uses_quotient_filter());

        // Verify all entries are present
        assert!(reader.get(b"a").await.unwrap().is_some());
        assert!(reader.get(b"b").await.unwrap().is_some());
        assert!(reader.get(b"c").await.unwrap().is_some());
        assert!(reader.get(b"d").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_multiway_merger_v2_deduplication() {
        // Test that deduplication works correctly with v2 format
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");
        std::fs::create_dir_all(&sst_dir).unwrap();

        let config = ATLLConfig::default();

        // Create two v2 SSTables with overlapping keys
        let v2_path1 = sst_dir.join("v2_old.sst");
        let v2_config1 = SSTableConfig {
            path: v2_path1.clone(),
            estimated_entries: 10,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder1 = SSTableBuilder::new(v2_config1).await.unwrap();
        builder1.add(&Entry::put("key1", "old_value1")).await.unwrap();
        builder1.add(&Entry::put("key2", "old_value2")).await.unwrap();
        builder1.finish().await.unwrap();

        let v2_path2 = sst_dir.join("v2_new.sst");
        let v2_config2 = SSTableConfig {
            path: v2_path2.clone(),
            estimated_entries: 10,
            filter_type: FilterType::QuotientFilter,
            ..Default::default()
        };

        let mut builder2 = SSTableBuilder::new(v2_config2).await.unwrap();
        builder2.add(&Entry::put("key1", "new_value1")).await.unwrap(); // Overwrites key1
        builder2.add(&Entry::put("key3", "value3")).await.unwrap();
        builder2.finish().await.unwrap();

        let output_path = temp_dir.path().join("merged_dedup.sst");

        // Merge with v2_path2 (newer) having higher iterator_idx
        let merger = MultiWayMerger::new(
            vec![v2_path1, v2_path2], // path2 is newer (higher idx)
            output_path.clone(),
            false,
            &config,
            None,
        )
        .await
        .unwrap();

        assert!(merger.uses_quotient_filter());

        let _result = merger.merge().await.unwrap();

        // Verify deduplication: key1 should have new value
        let reader = SSTableReader::open(output_path).await.unwrap();
        assert!(reader.uses_quotient_filter());

        let entry1 = reader.get(b"key1").await.unwrap().unwrap();
        assert_eq!(entry1.value.as_ref(), b"new_value1");

        let entry2 = reader.get(b"key2").await.unwrap().unwrap();
        assert_eq!(entry2.value.as_ref(), b"old_value2");

        let entry3 = reader.get(b"key3").await.unwrap().unwrap();
        assert_eq!(entry3.value.as_ref(), b"value3");
    }

}

