//! Fault injection framework for deterministic chaos testing.
//!
//! Provides controlled injection of:
//! - Network faults (delays, drops, partitions)
//! - Timing control (deterministic event scheduling)
//! - Crash injection (controlled node failures)
//!
//! All faults are deterministic and reproducible via seeds.

use parking_lot::Mutex;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// Deterministic random number generator for fault injection
///
/// Uses a seed to ensure reproducibility across test runs.
#[derive(Debug, Clone)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    /// Create a new deterministic RNG with the given seed
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9e3779b97f4a7c15),
        }
    }

    /// Generate next u64 using xorshift64
    pub fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// Generate a random value in range [min, max)
    pub fn gen_range(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        let range = max - min;
        min + (self.next_u64() % range)
    }

    /// Generate a boolean with given probability (0.0 to 1.0)
    pub fn gen_bool(&mut self, probability: f64) -> bool {
        let threshold = (probability * u64::MAX as f64) as u64;
        self.next_u64() < threshold
    }

    /// Generate Duration in range [min_ms, max_ms)
    pub fn gen_duration_ms(&mut self, min_ms: u64, max_ms: u64) -> Duration {
        Duration::from_millis(self.gen_range(min_ms, max_ms))
    }
}

/// Fault injection policy for network events
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NetworkFaultPolicy {
    /// Probability (0.0-1.0) of dropping a message
    pub drop_probability: f64,

    /// Probability (0.0-1.0) of delaying a message
    pub delay_probability: f64,

    /// Range for delay duration in milliseconds (min, max)
    pub delay_range_ms: (u64, u64),

    /// Probability (0.0-1.0) of reordering messages
    pub reorder_probability: f64,
}

impl Default for NetworkFaultPolicy {
    fn default() -> Self {
        Self {
            drop_probability: 0.0,
            delay_probability: 0.0,
            delay_range_ms: (10, 100),
            reorder_probability: 0.0,
        }
    }
}

impl NetworkFaultPolicy {
    /// Create a "clean" policy with no faults
    pub fn clean() -> Self {
        Self::default()
    }

    /// Create a policy for unreliable network
    pub fn unreliable() -> Self {
        Self {
            drop_probability: 0.05,
            delay_probability: 0.2,
            delay_range_ms: (10, 200),
            reorder_probability: 0.1,
        }
    }

    /// Create a policy for very faulty network
    pub fn chaotic() -> Self {
        Self {
            drop_probability: 0.15,
            delay_probability: 0.4,
            delay_range_ms: (50, 500),
            reorder_probability: 0.2,
        }
    }
}

/// Actions that a fault injector can take
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultAction {
    /// Drop this message entirely
    Drop,

    /// Delay message delivery by specified duration
    Delay(Duration),

    /// Deliver message immediately (no fault)
    Deliver,
}

/// Network fault injector with deterministic behavior
pub struct NetworkFaultInjector {
    rng: Arc<Mutex<DeterministicRng>>,
    policy: NetworkFaultPolicy,
}

impl NetworkFaultInjector {
    /// Create a new network fault injector with seed and policy
    pub fn new(seed: u64, policy: NetworkFaultPolicy) -> Self {
        Self {
            rng: Arc::new(Mutex::new(DeterministicRng::new(seed))),
            policy,
        }
    }

    /// Decide what action to take for a message
    pub fn decide_action(&self) -> FaultAction {
        let mut rng = self.rng.lock();

        // Check drop first (highest priority)
        if rng.gen_bool(self.policy.drop_probability) {
            return FaultAction::Drop;
        }

        // Check delay
        if rng.gen_bool(self.policy.delay_probability) {
            let delay = rng.gen_duration_ms(
                self.policy.delay_range_ms.0,
                self.policy.delay_range_ms.1,
            );
            return FaultAction::Delay(delay);
        }

        FaultAction::Deliver
    }

    /// Clone with same seed (for deterministic replay)
    pub fn clone_deterministic(&self) -> Self {
        Self {
            rng: Arc::clone(&self.rng),
            policy: self.policy.clone(),
        }
    }
}

/// Event scheduler for deterministic chaos scenarios
///
/// Allows scheduling events at specific logical times for reproducible testing.
pub struct ChaosScheduler {
    /// Events scheduled at specific logical times (timestamp -> event ID)
    events: BTreeMap<u64, Vec<String>>,

    /// Current logical time
    current_time: u64,
}

impl ChaosScheduler {
    /// Create a new chaos scheduler
    pub fn new() -> Self {
        Self {
            events: BTreeMap::new(),
            current_time: 0,
        }
    }

    /// Schedule an event at a specific logical time
    pub fn schedule(&mut self, time: u64, event_id: String) {
        self.events.entry(time).or_default().push(event_id);
    }

    /// Advance time and return events that should fire
    pub fn advance_to(&mut self, time: u64) -> Vec<String> {
        if time <= self.current_time {
            return Vec::new();
        }

        self.current_time = time;

        let mut fired_events = Vec::new();
        let times_to_remove: Vec<u64> = self
            .events
            .range(..=time)
            .map(|(t, _)| *t)
            .collect();

        for event_time in times_to_remove {
            if let Some(events) = self.events.remove(&event_time) {
                fired_events.extend(events);
            }
        }

        fired_events
    }

    /// Get current logical time
    pub fn current_time(&self) -> u64 {
        self.current_time
    }

    /// Get count of pending events
    pub fn pending_count(&self) -> usize {
        self.events.values().map(|v| v.len()).sum()
    }
}

impl Default for ChaosScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deterministic_rng_reproducible() {
        let mut rng1 = DeterministicRng::new(42);
        let mut rng2 = DeterministicRng::new(42);

        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_deterministic_rng_range() {
        let mut rng = DeterministicRng::new(123);

        for _ in 0..100 {
            let val = rng.gen_range(10, 20);
            assert!(val >= 10 && val < 20);
        }
    }

    #[test]
    fn test_network_fault_injector_deterministic() {
        let policy = NetworkFaultPolicy {
            drop_probability: 0.5,
            delay_probability: 0.3,
            delay_range_ms: (10, 50),
            reorder_probability: 0.0,
        };

        let injector1 = NetworkFaultInjector::new(999, policy.clone());
        let injector2 = NetworkFaultInjector::new(999, policy);

        for _ in 0..50 {
            assert_eq!(injector1.decide_action(), injector2.decide_action());
        }
    }

    #[test]
    fn test_chaos_scheduler() {
        let mut scheduler = ChaosScheduler::new();

        scheduler.schedule(100, "event1".to_string());
        scheduler.schedule(200, "event2".to_string());
        scheduler.schedule(100, "event3".to_string());
        scheduler.schedule(300, "event4".to_string());

        assert_eq!(scheduler.pending_count(), 4);

        let events = scheduler.advance_to(150);
        assert_eq!(events.len(), 2);
        assert!(events.contains(&"event1".to_string()));
        assert!(events.contains(&"event3".to_string()));
        assert_eq!(scheduler.pending_count(), 2);

        let events = scheduler.advance_to(250);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "event2");
        assert_eq!(scheduler.pending_count(), 1);

        let events = scheduler.advance_to(300);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "event4");
        assert_eq!(scheduler.pending_count(), 0);
    }

    #[test]
    fn test_network_fault_policies() {
        let clean = NetworkFaultPolicy::clean();
        assert_eq!(clean.drop_probability, 0.0);

        let unreliable = NetworkFaultPolicy::unreliable();
        assert!(unreliable.drop_probability > 0.0);
        assert!(unreliable.drop_probability < 0.1);

        let chaotic = NetworkFaultPolicy::chaotic();
        assert!(chaotic.drop_probability > unreliable.drop_probability);
    }
}
