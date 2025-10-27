/// Guard key management for ATLL range partitioning.
///
/// Based on `lsm_atll_design.yaml` spec lines 215-219 (guard_placement_adjustment).
///
/// Guards divide the keyspace into slots (range partitions) at each level L≥1.
/// The guard manager maintains:
/// - Current guard boundaries per level
/// - Key distribution sketches (quantile approximation)
/// - Guard adjustment proposals based on heat and size
///
/// # Example
/// ```text
/// Guards: [g₀="a", g₁="m", g₂="z"]
/// Slots:  [a,m)  [m,z)  [z,∞)
///         slot0  slot1  slot2
/// ```
///
/// # Guard Placement Goals
/// - Balance bytes across slots (minimize max(bytes_per_slot))
/// - Minimize integral(heat × overlap) for hot ranges
/// - Constrain guard moves to avoid iterator invalidation
use crate::error::{Error, Result};
use bytes::Bytes;

/// Guard key manager for a single level.
///
/// Tracks guard keys and provides slot lookup/assignment.
pub struct GuardManager {
    /// Level number this manager tracks
    _level: u8,

    /// Guard keys in sorted order
    /// Slot i covers range [guards[i], guards[i+1])
    /// Last slot covers [guards[n], +∞)
    guards: Vec<Bytes>,

    /// Number of slots (= guards.len())
    slot_count: u32,

    /// Key distribution sketch for rebalancing (simplified for now)
    /// In production, this would be a KLL sketch or T-Digest
    key_samples: Vec<Bytes>,

    /// Maximum samples to keep for sketch
    max_samples: usize,
}

impl GuardManager {
    /// Creates a new guard manager with initial uniform partitioning.
    ///
    /// # Arguments
    /// * `level` - Level number (must be ≥ 1)
    /// * `slot_count` - Number of slots to create
    ///
    /// # Initial Guards
    /// Starts with empty guards (single slot covering entire keyspace).
    /// Guards are populated via `learn_from_keys()` as writes occur.
    pub fn new(level: u8, slot_count: u32) -> Result<Self> {
        if level == 0 {
            return Err(Error::GuardInvariant(
                "Guards not used for L0".to_string(),
            ));
        }

        if slot_count == 0 {
            return Err(Error::Config("slot_count must be > 0".to_string()));
        }

        Ok(Self {
            _level: level,
            guards: Vec::new(), // Start with single slot
            slot_count,
            key_samples: Vec::new(),
            max_samples: 10000, // Keep up to 10K samples for quantile estimation
        })
    }

    /// Initializes guards with explicit boundary keys.
    ///
    /// Used during recovery from MANIFEST.
    pub fn with_guards(level: u8, guards: Vec<Bytes>) -> Result<Self> {
        if level == 0 {
            return Err(Error::GuardInvariant(
                "Guards not used for L0".to_string(),
            ));
        }

        let slot_count = if guards.is_empty() {
            1
        } else {
            guards.len() as u32
        };

        Ok(Self {
            _level: level,
            guards,
            slot_count,
            key_samples: Vec::new(),
            max_samples: 10000,
        })
    }

    /// Finds the slot ID for a given key.
    ///
    /// # Returns
    /// Slot ID in range [0, slot_count)
    ///
    /// # Algorithm
    /// Binary search over guards to find:
    /// - slot i if guards[i] <= key < guards[i+1]
    /// - last slot if key >= guards[last]
    ///
    /// # Example
    /// Guards: ["m", "z"]
    /// - key="a" → slot 0 (before "m")
    /// - key="m" → slot 1 (starts at "m")
    /// - key="p" → slot 1 (between "m" and "z")
    /// - key="z" → slot 2 (starts at "z", but only 2 guards so wraps to last slot)
    pub fn slot_for_key(&self, key: &[u8]) -> u32 {
        if self.guards.is_empty() {
            return 0; // Single slot covering all keys
        }

        // Find first guard > key
        let pos = self
            .guards
            .iter()
            .position(|guard| guard.as_ref() > key);

        match pos {
            Some(idx) => idx as u32, // Key is before guards[idx], so in slot idx
            None => self.guards.len() as u32, // Key is after all guards, last slot
        }
    }

    /// Returns the slot ID and range boundaries for a key.
    pub fn slot_range_for_key(&self, key: &[u8]) -> (u32, Option<Bytes>, Option<Bytes>) {
        let slot_id = self.slot_for_key(key);
        let (start, end) = self.slot_boundaries(slot_id);
        (slot_id, start, end)
    }

    /// Returns the (start, end) boundaries for a slot.
    ///
    /// # Returns
    /// - (Some(start), Some(end)) for interior slots
    /// - (None, Some(end)) for first slot (unbounded start)
    /// - (Some(start), None) for last slot (unbounded end)
    /// - (None, None) if no guards (single slot)
    ///
    /// # Slot Layout with guards ["m", "z"]
    /// - Slot 0: (None, Some("m"))     - keys < "m"
    /// - Slot 1: (Some("m"), Some("z")) - keys >= "m" and < "z"
    /// - Slot 2: (Some("z"), None)      - keys >= "z"
    pub fn slot_boundaries(&self, slot_id: u32) -> (Option<Bytes>, Option<Bytes>) {
        if self.guards.is_empty() {
            return (None, None); // Single slot, entire keyspace
        }

        let slot_idx = slot_id as usize;

        let start = if slot_idx == 0 {
            None // First slot: unbounded start
        } else {
            self.guards.get(slot_idx - 1).cloned()
        };

        let end = self.guards.get(slot_idx).cloned(); // None for last slot

        (start, end)
    }

    /// Returns all guard keys.
    pub fn guards(&self) -> &[Bytes] {
        &self.guards
    }

    /// Returns the number of slots.
    pub fn slot_count(&self) -> u32 {
        self.slot_count
    }

    /// Learns key distribution from a batch of keys.
    ///
    /// Samples keys for future guard rebalancing.
    /// In production, this would update a quantile sketch.
    pub fn learn_from_keys(&mut self, keys: &[Bytes]) {
        for key in keys {
            if self.key_samples.len() < self.max_samples {
                self.key_samples.push(key.clone());
            } else {
                // Reservoir sampling: randomly replace with decreasing probability
                let idx = rand::random::<usize>() % self.max_samples;
                if rand::random::<f64>() < 0.1 {
                    // 10% replacement rate
                    self.key_samples[idx] = key.clone();
                }
            }
        }
    }

    /// Proposes new guard keys based on learned distribution.
    ///
    /// # Algorithm (Simplified)
    /// 1. Sort sampled keys
    /// 2. Pick quantiles at intervals of 1/slot_count
    /// 3. Return proposed guards
    ///
    /// # Production Enhancement
    /// - Use KLL sketch or T-Digest for better quantile estimation
    /// - Incorporate heat distribution (minimize hot key splits)
    /// - Constrain guard moves to avoid iterator invalidation
    /// - Apply hysteresis to avoid thrashing
    pub fn propose_guards(&self) -> Vec<Bytes> {
        if self.key_samples.is_empty() {
            return Vec::new();
        }

        let mut sorted_samples = self.key_samples.clone();
        sorted_samples.sort();
        sorted_samples.dedup(); // Remove duplicates

        if sorted_samples.len() < self.slot_count as usize {
            // Not enough samples for desired slot count
            return sorted_samples;
        }

        // Pick quantiles
        let interval = sorted_samples.len() / self.slot_count as usize;
        let mut proposed = Vec::new();

        for i in 1..self.slot_count {
            let idx = (i as usize * interval).min(sorted_samples.len() - 1);
            proposed.push(sorted_samples[idx].clone());
        }

        proposed
    }

    /// Updates guards to new boundaries.
    ///
    /// # Safety
    /// Caller must ensure:
    /// - New guards are sorted
    /// - Active iterators are handled (paused or invalidated)
    /// - MANIFEST edit is applied atomically
    pub fn update_guards(&mut self, new_guards: Vec<Bytes>) -> Result<()> {
        // Validate sorted order
        for window in new_guards.windows(2) {
            if window[0] >= window[1] {
                return Err(Error::GuardInvariant(
                    "Guards must be strictly sorted".to_string(),
                ));
            }
        }

        self.guards = new_guards;
        self.slot_count = if self.guards.is_empty() {
            1
        } else {
            self.guards.len() as u32
        };

        Ok(())
    }

    /// Clears key samples (e.g., after rebalancing).
    pub fn clear_samples(&mut self) {
        self.key_samples.clear();
    }

    /// Returns the number of key samples collected.
    pub fn sample_count(&self) -> usize {
        self.key_samples.len()
    }

    /// Finds all slots that overlap with a key range [start, end).
    ///
    /// Returns a vector of slot IDs.
    pub fn overlapping_slots(&self, start: &[u8], end: &[u8]) -> Vec<u32> {
        if self.guards.is_empty() {
            return vec![0]; // Single slot
        }

        let start_slot = self.slot_for_key(start);
        let end_slot = self.slot_for_key(end);

        // Handle edge case where end == guard boundary
        let end_slot = if end_slot > 0
            && self
                .guards
                .get(end_slot as usize - 1)
                .map(|g| g.as_ref() == end)
                .unwrap_or(false)
        {
            end_slot - 1
        } else {
            end_slot
        };

        (start_slot..=end_slot).collect()
    }
}

/// Global guard registry for all levels.
///
/// Manages guard managers for L1..Lmax.
pub struct GuardRegistry {
    /// Guard managers indexed by level (L1 = index 0)
    managers: Vec<GuardManager>,

    /// Maximum levels
    max_levels: u8,
}

impl GuardRegistry {
    /// Creates a new guard registry.
    ///
    /// # Arguments
    /// * `max_levels` - Maximum number of levels (L0..Lmax)
    /// * `slot_counts` - Number of slots per level (L1..Lmax)
    pub fn new(max_levels: u8, slot_counts: Vec<u32>) -> Result<Self> {
        if slot_counts.len() != max_levels as usize {
            return Err(Error::Config(format!(
                "slot_counts length ({}) must match max_levels ({})",
                slot_counts.len(),
                max_levels
            )));
        }

        let mut managers = Vec::new();
        for (idx, &count) in slot_counts.iter().enumerate() {
            let level = (idx + 1) as u8; // L1..Lmax
            managers.push(GuardManager::new(level, count)?);
        }

        Ok(Self {
            managers,
            max_levels,
        })
    }

    /// Returns the guard manager for a level.
    ///
    /// # Panics
    /// Panics if level is 0 or > max_levels.
    pub fn manager(&self, level: u8) -> Result<&GuardManager> {
        if level == 0 || level > self.max_levels {
            return Err(Error::GuardInvariant(format!(
                "Invalid level {} (must be 1..{})",
                level, self.max_levels
            )));
        }

        Ok(&self.managers[(level - 1) as usize])
    }

    /// Returns a mutable guard manager for a level.
    pub fn manager_mut(&mut self, level: u8) -> Result<&mut GuardManager> {
        if level == 0 || level > self.max_levels {
            return Err(Error::GuardInvariant(format!(
                "Invalid level {} (must be 1..{})",
                level, self.max_levels
            )));
        }

        Ok(&mut self.managers[(level - 1) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_manager_single_slot() {
        let mgr = GuardManager::new(1, 1).unwrap();
        assert_eq!(mgr.slot_count(), 1);
        assert_eq!(mgr.guards().len(), 0);

        // All keys map to slot 0
        assert_eq!(mgr.slot_for_key(b"a"), 0);
        assert_eq!(mgr.slot_for_key(b"z"), 0);
        assert_eq!(mgr.slot_for_key(b"zzz"), 0);
    }

    #[test]
    fn test_guard_manager_multiple_slots() {
        let guards = vec![Bytes::from("m"), Bytes::from("z")];
        let mgr = GuardManager::with_guards(1, guards).unwrap();

        // With 2 guards, we have 3 slots: [-, m), [m, z), [z, +∞)
        assert_eq!(mgr.slot_count(), 2);

        // [-, m) -> slot 0
        assert_eq!(mgr.slot_for_key(b"a"), 0);
        assert_eq!(mgr.slot_for_key(b"k"), 0);

        // [m, z) -> slot 1
        assert_eq!(mgr.slot_for_key(b"m"), 1);
        assert_eq!(mgr.slot_for_key(b"n"), 1);
        assert_eq!(mgr.slot_for_key(b"y"), 1);

        // [z, +∞) -> slot 2
        assert_eq!(mgr.slot_for_key(b"z"), 2);
        assert_eq!(mgr.slot_for_key(b"zzz"), 2);
    }

    #[test]
    fn test_slot_boundaries() {
        let guards = vec![
            Bytes::from("m"),
            Bytes::from("z"),
        ];
        let mgr = GuardManager::with_guards(1, guards).unwrap();

        // Slot 0: (None, Some("m"))
        let (start, end) = mgr.slot_boundaries(0);
        assert!(start.is_none());
        assert_eq!(end.unwrap(), Bytes::from("m"));

        // Slot 1: (Some("m"), Some("z"))
        let (start, end) = mgr.slot_boundaries(1);
        assert_eq!(start.unwrap(), Bytes::from("m"));
        assert_eq!(end.unwrap(), Bytes::from("z"));
    }

    #[test]
    fn test_learn_and_propose_guards() {
        let mut mgr = GuardManager::new(1, 3).unwrap();

        // Learn from keys
        let keys: Vec<Bytes> = (b'a'..=b'z')
            .map(|c| Bytes::from(vec![c]))
            .collect();

        mgr.learn_from_keys(&keys);
        assert!(mgr.sample_count() > 0);

        // Propose new guards
        let proposed = mgr.propose_guards();
        assert!(!proposed.is_empty());

        // Proposed guards should be sorted
        for window in proposed.windows(2) {
            assert!(window[0] < window[1]);
        }
    }

    #[test]
    fn test_update_guards() {
        let mut mgr = GuardManager::new(1, 1).unwrap();

        let new_guards = vec![
            Bytes::from("d"),
            Bytes::from("m"),
            Bytes::from("t"),
        ];

        mgr.update_guards(new_guards.clone()).unwrap();
        assert_eq!(mgr.guards().len(), 3);
        assert_eq!(mgr.slot_count(), 3);
    }

    #[test]
    fn test_update_guards_unsorted_fails() {
        let mut mgr = GuardManager::new(1, 1).unwrap();

        let bad_guards = vec![
            Bytes::from("z"),
            Bytes::from("a"), // Out of order!
        ];

        assert!(mgr.update_guards(bad_guards).is_err());
    }

    #[test]
    fn test_overlapping_slots() {
        let guards = vec![
            Bytes::from("d"),
            Bytes::from("m"),
            Bytes::from("t"),
        ];
        let mgr = GuardManager::with_guards(1, guards).unwrap();

        // Range [a, e) overlaps slots 0 and 1
        let slots = mgr.overlapping_slots(b"a", b"e");
        assert_eq!(slots, vec![0, 1]);

        // Range [e, p) overlaps slots 1 and 2
        let slots = mgr.overlapping_slots(b"e", b"p");
        assert_eq!(slots, vec![1, 2]);

        // Range [a, z) overlaps all slots
        let slots = mgr.overlapping_slots(b"a", b"z");
        assert_eq!(slots, vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_guard_registry() {
        let slot_counts = vec![32, 32, 32, 32, 32, 32, 32]; // L1..L7
        let registry = GuardRegistry::new(7, slot_counts).unwrap();

        // Access L1 manager
        let mgr = registry.manager(1).unwrap();
        assert_eq!(mgr.slot_count(), 32);

        // L0 should fail
        assert!(registry.manager(0).is_err());

        // L8 should fail (max is L7)
        assert!(registry.manager(8).is_err());
    }

    #[test]
    fn test_slot_range_for_key() {
        let guards = vec![Bytes::from("m"), Bytes::from("z")];
        let mgr = GuardManager::with_guards(1, guards).unwrap();

        let (slot_id, start, end) = mgr.slot_range_for_key(b"a");
        assert_eq!(slot_id, 0);
        assert!(start.is_none());
        assert_eq!(end.unwrap(), Bytes::from("m"));

        let (slot_id, start, end) = mgr.slot_range_for_key(b"p");
        assert_eq!(slot_id, 1);
        assert_eq!(start.unwrap(), Bytes::from("m"));
        assert_eq!(end.unwrap(), Bytes::from("z"));
    }
}
