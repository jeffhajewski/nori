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
use crate::config::ATLLConfig;
use crate::error::{Error, Result};
use crate::heat::HeatTracker;
use crate::manifest::{ManifestLog, RunMeta};
use bytes::Bytes;
use nori_sstable::{Entry, SSTableBuilder, SSTableConfig, SSTableIterator, SSTableReader};
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

    /// Guard placement adjustment (deferred to Phase 7).
    ///
    /// **When**: Key distribution skew detected
    /// **Effect**: Rebalances slot sizes
    GuardMove {
        level: u8,
        new_guards: Vec<Bytes>,
    },

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

        let exploration_bonus = c * ((total_selections as f64).ln() / self.selection_count as f64).sqrt();
        self.avg_reward + exploration_bonus
    }
}

/// Bandit scheduler for compaction action selection.
///
/// Uses epsilon-greedy or UCB policy to balance exploration/exploitation.
pub struct BanditScheduler {
    /// Per-(level, slot) arm state
    arms: HashMap<(u8, u32), BanditArm>,

    /// Epsilon for epsilon-greedy (0.1 = 10% exploration)
    epsilon: f64,

    /// Total selections across all arms
    total_selections: u64,

    /// Configuration
    config: ATLLConfig,
}

impl BanditScheduler {
    /// Creates a new bandit scheduler.
    pub fn new(config: ATLLConfig) -> Self {
        Self {
            arms: HashMap::new(),
            epsilon: 0.1,
            total_selections: 0,
            config,
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
        &mut self,
        heat_tracker: &HeatTracker,
        manifest: &ManifestLog,
    ) -> CompactionAction {
        // Get all candidate actions from manifest state
        let candidates = self.generate_candidates(heat_tracker, manifest);

        if candidates.is_empty() {
            return CompactionAction::DoNothing;
        }

        // Epsilon-greedy selection
        let explore = rand::random::<f64>() < self.epsilon;

        let chosen_idx = if explore {
            // Explore: random action
            rand::random::<usize>() % candidates.len()
        } else {
            // Exploit: best UCB score
            self.best_ucb_action(&candidates)
        };

        candidates[chosen_idx].clone()
    }

    /// Updates the bandit state after executing an action.
    pub fn update_reward(
        &mut self,
        action: &CompactionAction,
        bytes_written: u64,
        latency_reduction_ms: f64,
        heat_score: f32,
    ) {
        let (level, slot_id) = match action {
            CompactionAction::Tier { level, slot_id, .. }
            | CompactionAction::Promote { level, slot_id, .. }
            | CompactionAction::EagerLevel { level, slot_id }
            | CompactionAction::Cleanup { level, slot_id } => (*level, *slot_id),
            _ => return, // No reward for DoNothing or GuardMove
        };

        // Compute reward: (latency improvement × heat) / bytes rewritten
        let reward = if bytes_written > 0 {
            (latency_reduction_ms * heat_score as f64) / bytes_written as f64
        } else {
            0.0
        };

        // Update arm statistics
        let arm = self
            .arms
            .entry((level, slot_id))
            .or_insert_with(|| BanditArm::new(level, slot_id));

        arm.update(reward);
        self.total_selections += 1;
    }

    /// Generates candidate compaction actions from current LSM state.
    fn generate_candidates(
        &self,
        heat_tracker: &HeatTracker,
        manifest: &ManifestLog,
    ) -> Vec<CompactionAction> {
        let mut candidates = Vec::new();

        // Iterate over all levels and slots in manifest
        for level in 1..=self.config.max_levels {
            let level_state = manifest.level_state(level);

            for (slot_id, slot_state) in level_state.iter().enumerate() {
                let slot_id = slot_id as u32;
                let heat = heat_tracker.get_heat(level, slot_id);
                let k = heat_tracker.get_k(level, slot_id);

                // Candidate 1: Tier (if runs > K or bytes > budget)
                if slot_state.run_count > k as usize
                    || slot_state.total_bytes > self.config.slot_budget_bytes(level)
                {
                    candidates.push(CompactionAction::Tier {
                        level,
                        slot_id,
                        run_count: k.min(4) as usize, // Merge up to 4 runs
                    });
                }

                // Candidate 2: EagerLevel (if hot)
                if heat >= self.config.heat_thresholds.hot && slot_state.run_count > 1 {
                    candidates.push(CompactionAction::EagerLevel { level, slot_id });
                }

                // Candidate 3: Cleanup (if high tombstone density)
                if slot_state.tombstone_density > 0.3 {
                    candidates.push(CompactionAction::Cleanup { level, slot_id });
                }

                // Candidate 4: Promote (if slot bytes >> budget)
                if slot_state.total_bytes > 2 * self.config.slot_budget_bytes(level)
                    && !slot_state.runs.is_empty()
                {
                    let largest_run = slot_state.runs[0].file_number;
                    candidates.push(CompactionAction::Promote {
                        level,
                        slot_id,
                        file_number: largest_run,
                    });
                }
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

    /// Slice size for cooperative yielding (from config)
    slice_size: u64,
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
    /// Creates a new K-way merger.
    pub async fn new(
        input_paths: Vec<PathBuf>,
        output_path: PathBuf,
        can_drop_tombstones: bool,
        config: &ATLLConfig,
    ) -> Result<Self> {
        // Open all input SSTables
        let mut readers = Vec::new();
        for path in input_paths {
            let reader = SSTableReader::open(path)
                .await
                .map_err(|e| Error::Internal(format!("Failed to open SSTable: {}", e)))?;
            readers.push(Arc::new(reader));
        }

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
        };

        let builder = SSTableBuilder::new(sst_config)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create SSTable builder: {}", e)))?;

        Ok(Self {
            readers,
            heap: BinaryHeap::new(),
            builder,
            can_drop_tombstones,
            bytes_written: 0,
            slice_size: (config.io.compaction_slice_mb as u64) * 1024 * 1024,
        })
    }

    /// Performs K-way merge and writes to output SSTable.
    ///
    /// Returns metadata for the merged run.
    pub async fn merge(mut self) -> Result<RunMeta> {
        // Initialize heap with first entry from each iterator
        let mut iterators: Vec<SSTableIterator> = self
            .readers
            .iter()
            .map(|r| r.clone().iter())
            .collect();

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
        let min_seqno = u64::MAX;
        let max_seqno = 0u64;
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
            self.builder
                .add(&entry)
                .await
                .map_err(|e| Error::Internal(format!("SSTable write error: {}", e)))?;

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
            min_seqno,
            max_seqno,
            tombstone_count,
            filter_fp: 0.001,
            heat_hint: 0.0,
            value_log_segment_id: None,
        })
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
    fn level_state(&self, _level: u8) -> Vec<LevelSlotState> {
        // TODO: Implement in Phase 7 when integrating with full manifest
        vec![LevelSlotState {
            run_count: 0,
            total_bytes: 0,
            tombstone_density: 0.0,
            runs: vec![],
        }]
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

    /// Next file number allocator (shared with manifest)
    next_file_number: u64,
}

impl CompactionExecutor {
    /// Creates a new compaction executor.
    pub fn new(sst_dir: impl AsRef<std::path::Path>, config: ATLLConfig) -> Result<Self> {
        let sst_dir = sst_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&sst_dir)?;

        Ok(Self {
            sst_dir,
            config,
            next_file_number: 1000, // Start after flush file numbers
        })
    }

    /// Executes a compaction action and returns manifest edits.
    ///
    /// # Returns
    /// - Vec<ManifestEdit>: Edits to apply to manifest
    /// - u64: Bytes written during compaction
    pub async fn execute(
        &mut self,
        action: &CompactionAction,
        manifest: &ManifestLog,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        match action {
            CompactionAction::Tier {
                level,
                slot_id,
                run_count,
            } => self.execute_tier(*level, *slot_id, *run_count, manifest).await,

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

            CompactionAction::GuardMove { .. } => {
                // GuardMove deferred to Phase 7
                Ok((vec![], 0))
            }

            CompactionAction::DoNothing => Ok((vec![], 0)),
        }
    }

    /// Executes horizontal tiering: merge K oldest runs in a slot.
    async fn execute_tier(
        &mut self,
        level: u8,
        slot_id: u32,
        run_count: usize,
        manifest: &ManifestLog,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        // Get slot state from manifest
        let level_state = manifest.level_state(level);
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

        // Allocate new file number
        let output_file_number = self.allocate_file_number();
        let output_path = self.sst_path(output_file_number);

        // Can drop tombstones only if this is the last level
        let can_drop_tombstones = level == self.config.max_levels;

        // Perform K-way merge
        let merger = MultiWayMerger::new(input_paths, output_path, can_drop_tombstones, &self.config)
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
        &mut self,
        level: u8,
        slot_id: u32,
        file_number: u64,
        manifest: &ManifestLog,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        if level >= self.config.max_levels {
            return Err(Error::Internal("Cannot promote from last level".to_string()));
        }

        // Get the run to promote
        let level_state = manifest.level_state(level);
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
        let mut edits = Vec::new();

        // Delete from current level
        edits.push(ManifestEdit::DeleteFile {
            level,
            slot_id: Some(slot_id),
            file_number,
        });

        // Add to next level (same slot)
        edits.push(ManifestEdit::AddFile {
            level: level + 1,
            slot_id: Some(slot_id),
            run: run_to_promote.clone(),
        });

        Ok((edits, 0)) // No actual bytes written (just metadata move)
    }

    /// Executes eager leveling: converge hot slot to K=1.
    async fn execute_eager_level(
        &mut self,
        level: u8,
        slot_id: u32,
        manifest: &ManifestLog,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        // Eager leveling is essentially tiering with run_count = all runs
        let level_state = manifest.level_state(level);
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
        &mut self,
        level: u8,
        slot_id: u32,
        manifest: &ManifestLog,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        use crate::manifest::ManifestEdit;

        // Get slot runs
        let level_state = manifest.level_state(level);
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

        // Allocate new file number
        let output_file_number = self.allocate_file_number();
        let output_path = self.sst_path(output_file_number);

        // Force tombstone dropping for cleanup
        let can_drop_tombstones = true;

        // Perform merge with tombstone dropping
        let merger = MultiWayMerger::new(input_paths, output_path, can_drop_tombstones, &self.config)
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

    /// Returns the SSTable file path for a given file number.
    fn sst_path(&self, file_number: u64) -> PathBuf {
        self.sst_dir.join(format!("sst-{:06}.sst", file_number))
    }

    /// Allocates a new file number.
    fn allocate_file_number(&mut self) -> u64 {
        let num = self.next_file_number;
        self.next_file_number += 1;
        num
    }
}

/// Compaction coordinator that runs background compaction tasks.
///
/// Manages a pool of compaction workers, selects actions via the bandit scheduler,
/// and respects IO budget constraints.
pub struct CompactionCoordinator {
    /// Bandit scheduler for action selection
    scheduler: BanditScheduler,

    /// Compaction executor
    executor: CompactionExecutor,

    /// Heat tracker reference
    heat_tracker: Arc<HeatTracker>,

    /// Manifest reference
    manifest: Arc<parking_lot::RwLock<ManifestLog>>,

    /// Maximum concurrent background compactions
    max_concurrent: usize,

    /// Current active compactions
    active_count: Arc<parking_lot::RwLock<usize>>,

    /// Shutdown signal
    shutdown: Arc<parking_lot::RwLock<bool>>,
}

impl CompactionCoordinator {
    /// Creates a new compaction coordinator.
    pub fn new(
        sst_dir: impl AsRef<std::path::Path>,
        config: ATLLConfig,
        heat_tracker: Arc<HeatTracker>,
        manifest: Arc<parking_lot::RwLock<ManifestLog>>,
    ) -> Result<Self> {
        let max_concurrent = config.io.max_background_compactions;
        let scheduler = BanditScheduler::new(config.clone());
        let executor = CompactionExecutor::new(sst_dir, config)?;

        Ok(Self {
            scheduler,
            executor,
            heat_tracker,
            manifest,
            max_concurrent,
            active_count: Arc::new(parking_lot::RwLock::new(0)),
            shutdown: Arc::new(parking_lot::RwLock::new(false)),
        })
    }

    /// Starts the background compaction loop.
    ///
    /// Returns a join handle that can be awaited for graceful shutdown.
    pub fn start(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(200));

            loop {
                interval.tick().await;

                // Check shutdown signal
                if *self.shutdown.read() {
                    break;
                }

                // Check if we can start a new compaction
                let active = *self.active_count.read();
                if active >= self.max_concurrent {
                    continue; // At capacity, wait
                }

                // Select next action
                let manifest = self.manifest.read();
                let action = self.scheduler.select_action(&self.heat_tracker, &manifest);

                if matches!(action, CompactionAction::DoNothing) {
                    continue; // No work to do
                }

                // Increment active count
                *self.active_count.write() += 1;

                // Spawn compaction task
                let executor = self.executor.clone_for_task();
                let manifest_clone = self.manifest.clone();
                let heat_tracker = self.heat_tracker.clone();
                let active_count = self.active_count.clone();
                let action_clone = action.clone();

                tokio::spawn(async move {
                    // Execute compaction
                    let result = executor.execute_action(&action_clone, &manifest_clone).await;

                    // Update rewards based on result
                    if let Ok((edits, _bytes_written)) = result {
                        // Apply manifest edits
                        let mut manifest = manifest_clone.write();
                        for edit in edits {
                            let _ = manifest.append(edit);
                        }

                        // Estimate latency reduction (placeholder heuristic)
                        let _latency_reduction = Self::estimate_latency_reduction(&action_clone);

                        // Get heat score
                        let (level, slot_id) = Self::extract_slot(&action_clone);
                        let _heat_score = heat_tracker.get_heat(level, slot_id);

                        // Update scheduler rewards (would need access to scheduler)
                        // This is a design challenge - scheduler is owned by coordinator
                        // In Phase 8, we'd use message passing or shared state
                        drop(manifest);

                        // Log completion (commented out - would need log crate)
                        // log::info!(
                        //     "Completed compaction {:?}: {} bytes written",
                        //     action_clone,
                        //     bytes_written
                        // );
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
            next_file_number: self.next_file_number, // Shared counter needs Arc in real impl
        }
    }

    async fn execute_action(
        &self,
        _action: &CompactionAction,
        manifest: &Arc<parking_lot::RwLock<ManifestLog>>,
    ) -> Result<(Vec<crate::manifest::ManifestEdit>, u64)> {
        let manifest_read = manifest.read();
        // Note: This is simplified - real impl needs mutable executor
        // For now, this demonstrates the pattern
        drop(manifest_read);
        Ok((vec![], 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memtable::Memtable;
    use crate::flush::Flusher;

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
        let scheduler = BanditScheduler::new(config);
        assert_eq!(scheduler.epsilon, 0.1);
        assert_eq!(scheduler.total_selections, 0);
    }

    #[test]
    fn test_bandit_scheduler_reward_update() {
        let config = ATLLConfig::default();
        let mut scheduler = BanditScheduler::new(config);

        let action = CompactionAction::Tier {
            level: 1,
            slot_id: 0,
            run_count: 3,
        };

        scheduler.update_reward(&action, 1024, 5.0, 0.8);

        let arm = scheduler.arms.get(&(1, 0)).unwrap();
        assert_eq!(arm.selection_count, 1);
        assert!(arm.avg_reward > 0.0);
        assert_eq!(scheduler.total_selections, 1);
    }

    #[tokio::test]
    async fn test_multiway_merger_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let output_path = temp_dir.path().join("merged.sst");

        let config = ATLLConfig::default();

        let merger = MultiWayMerger::new(vec![], output_path, false, &config)
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
        let mt = Memtable::new(1);
        mt.put(Bytes::from("key1"), Bytes::from("value1"), 1)
            .unwrap();
        mt.put(Bytes::from("key2"), Bytes::from("value2"), 2)
            .unwrap();
        mt.put(Bytes::from("key3"), Bytes::from("value3"), 3)
            .unwrap();

        // Flush to SSTable
        let _run = flusher.flush_to_l0(&mt, 1).await.unwrap();

        // Merge single SSTable
        let input_path = sst_dir.join("sst-000001.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input_path], output_path, false, &config)
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
        let mt1 = Memtable::new(1);
        mt1.put(Bytes::from("key1"), Bytes::from("value1"), 1)
            .unwrap();
        mt1.put(Bytes::from("key3"), Bytes::from("value3"), 3)
            .unwrap();
        mt1.put(Bytes::from("key5"), Bytes::from("value5"), 5)
            .unwrap();

        flusher.flush_to_l0(&mt1, 1).await.unwrap();

        // Create second memtable (keys 2, 4, 6)
        let mt2 = Memtable::new(10);
        mt2.put(Bytes::from("key2"), Bytes::from("value2"), 10)
            .unwrap();
        mt2.put(Bytes::from("key4"), Bytes::from("value4"), 11)
            .unwrap();
        mt2.put(Bytes::from("key6"), Bytes::from("value6"), 12)
            .unwrap();

        flusher.flush_to_l0(&mt2, 2).await.unwrap();

        // Merge both SSTables
        let input1 = sst_dir.join("sst-000001.sst");
        let input2 = sst_dir.join("sst-000002.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input1, input2], output_path.clone(), false, &config)
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
        let mt = Memtable::new(1);
        mt.put(Bytes::from("key1"), Bytes::from("value1"), 1)
            .unwrap();
        mt.delete(Bytes::from("key2"), 2).unwrap();
        mt.put(Bytes::from("key3"), Bytes::from("value3"), 3)
            .unwrap();

        flusher.flush_to_l0(&mt, 1).await.unwrap();

        // Merge with can_drop_tombstones = false
        let input_path = sst_dir.join("sst-000001.sst");
        let output_path1 = temp_dir.path().join("merged_keep.sst");

        let merger = MultiWayMerger::new(vec![input_path.clone()], output_path1.clone(), false, &config)
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

        let merger2 = MultiWayMerger::new(vec![input_path], output_path2.clone(), true, &config)
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
        let mt1 = Memtable::new(1);
        mt1.put(Bytes::from("key1"), Bytes::from("old_value"), 1)
            .unwrap();

        flusher.flush_to_l0(&mt1, 1).await.unwrap();

        let mt2 = Memtable::new(10);
        mt2.put(Bytes::from("key1"), Bytes::from("new_value"), 10)
            .unwrap();

        flusher.flush_to_l0(&mt2, 2).await.unwrap();

        // Merge (mt2 has higher index, should be newer)
        let input1 = sst_dir.join("sst-000001.sst");
        let input2 = sst_dir.join("sst-000002.sst");
        let output_path = temp_dir.path().join("merged.sst");

        let merger = MultiWayMerger::new(vec![input1, input2], output_path.clone(), false, &config)
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
        let executor = CompactionExecutor::new(&sst_dir, config).unwrap();

        assert!(sst_dir.exists());
        assert_eq!(executor.next_file_number, 1000);
    }

    #[test]
    fn test_executor_file_number_allocation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sst_dir = temp_dir.path().join("sst");

        let config = ATLLConfig::default();
        let mut executor = CompactionExecutor::new(&sst_dir, config).unwrap();

        let num1 = executor.allocate_file_number();
        let num2 = executor.allocate_file_number();
        let num3 = executor.allocate_file_number();

        assert_eq!(num1, 1000);
        assert_eq!(num2, 1001);
        assert_eq!(num3, 1002);
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

        let coordinator = CompactionCoordinator::new(&sst_dir, config, heat_tracker, manifest).unwrap();

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

        let eager_action = CompactionAction::EagerLevel { level: 1, slot_id: 0 };
        let reduction = CompactionCoordinator::estimate_latency_reduction(&eager_action);
        assert_eq!(reduction, 2.0);

        let cleanup_action = CompactionAction::Cleanup { level: 1, slot_id: 0 };
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
}
