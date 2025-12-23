//! Property-based test generators for distributed systems.
//!
//! Provides proptest strategies for:
//! - Operation sequences (put/get/delete)
//! - Cluster events (crashes, partitions, leader transfers)
//! - Network fault policies
//! - Complete test scenarios
//!
//! # Example
//!
//! ```ignore
//! use nori_testkit::generators::*;
//! use proptest::prelude::*;
//!
//! proptest! {
//!     #[test]
//!     fn test_linearizability(ops in operation_sequence(100)) {
//!         // Run ops through cluster, check linearizability
//!     }
//! }
//! ```

use bytes::Bytes;
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::fault_injection::NetworkFaultPolicy;
use crate::linearizability::Operation;

// =============================================================================
// Key/Value Generators
// =============================================================================

/// Generate a random key (1-64 bytes).
pub fn key() -> impl Strategy<Value = Bytes> {
    prop::collection::vec(any::<u8>(), 1..=64).prop_map(Bytes::from)
}

/// Generate a random value (0-256 bytes).
pub fn value() -> impl Strategy<Value = Bytes> {
    prop::collection::vec(any::<u8>(), 0..=256).prop_map(Bytes::from)
}

/// Generate a key from a fixed set (key_0, key_1, ..., key_{n-1}).
///
/// Useful for testing with a known key space to increase operation overlap.
pub fn key_from_set(n: usize) -> impl Strategy<Value = Bytes> {
    (0..n).prop_map(move |i| Bytes::from(format!("key_{}", i)))
}

/// Generate a value with a client/sequence marker for debugging.
///
/// Format: "value_{client_id}_{seq}"
pub fn value_with_marker(client_id: usize, seq: usize) -> Bytes {
    Bytes::from(format!("value_{}_{}", client_id, seq))
}

// =============================================================================
// Operation Generators
// =============================================================================

/// Generate a random KV operation with random keys/values.
pub fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        key().prop_map(|k| Operation::Get { key: k }),
        (key(), value()).prop_map(|(k, v)| Operation::Put { key: k, value: v }),
        key().prop_map(|k| Operation::Delete { key: k }),
    ]
}

/// Generate an operation with weighted probabilities.
///
/// # Arguments
/// * `put_weight` - Relative weight for Put operations
/// * `get_weight` - Relative weight for Get operations
/// * `delete_weight` - Relative weight for Delete operations
///
/// # Example
/// ```ignore
/// // 70% puts, 20% gets, 10% deletes
/// let op = operation_weighted(70, 20, 10);
/// ```
pub fn operation_weighted(
    put_weight: u32,
    get_weight: u32,
    delete_weight: u32,
) -> impl Strategy<Value = Operation> {
    prop_oneof![
        get_weight => key().prop_map(|k| Operation::Get { key: k }),
        put_weight => (key(), value()).prop_map(|(k, v)| Operation::Put { key: k, value: v }),
        delete_weight => key().prop_map(|k| Operation::Delete { key: k }),
    ]
}

/// Generate an operation using keys from a fixed set.
///
/// This increases the likelihood of operations affecting the same keys,
/// which is important for testing linearizability and conflict resolution.
pub fn operation_on_key_set(num_keys: usize) -> impl Strategy<Value = Operation> {
    prop_oneof![
        key_from_set(num_keys).prop_map(|k| Operation::Get { key: k }),
        (key_from_set(num_keys), value()).prop_map(|(k, v)| Operation::Put { key: k, value: v }),
        key_from_set(num_keys).prop_map(|k| Operation::Delete { key: k }),
    ]
}

/// Generate an operation on a fixed key set with weighted probabilities.
pub fn operation_on_key_set_weighted(
    num_keys: usize,
    put_weight: u32,
    get_weight: u32,
    delete_weight: u32,
) -> impl Strategy<Value = Operation> {
    prop_oneof![
        get_weight => key_from_set(num_keys).prop_map(|k| Operation::Get { key: k }),
        put_weight => (key_from_set(num_keys), value()).prop_map(|(k, v)| Operation::Put { key: k, value: v }),
        delete_weight => key_from_set(num_keys).prop_map(|k| Operation::Delete { key: k }),
    ]
}

// =============================================================================
// Operation Sequence Generators
// =============================================================================

/// Generate a sequence of random operations.
pub fn operation_sequence(len: usize) -> impl Strategy<Value = Vec<Operation>> {
    prop::collection::vec(operation(), len)
}

/// Generate a sequence of operations with length in a range.
pub fn operation_sequence_range(min: usize, max: usize) -> impl Strategy<Value = Vec<Operation>> {
    prop::collection::vec(operation(), min..=max)
}

/// Generate a sequence using a fixed key set (better for linearizability testing).
pub fn operation_sequence_on_key_set(
    num_keys: usize,
    len: usize,
) -> impl Strategy<Value = Vec<Operation>> {
    prop::collection::vec(operation_on_key_set(num_keys), len)
}

/// Generate a workload-like sequence: mostly puts, some gets, few deletes.
///
/// This mimics typical KV store access patterns.
pub fn workload_sequence(len: usize) -> impl Strategy<Value = Vec<Operation>> {
    prop::collection::vec(operation_weighted(70, 20, 10), len)
}

// =============================================================================
// Cluster Event Generators
// =============================================================================

/// Events that can occur in a distributed cluster during testing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClusterEvent {
    /// Partition a node from the cluster (network isolation).
    PartitionNode(usize),

    /// Heal a previously partitioned node (restore connectivity).
    HealNode(usize),

    /// Crash a node (simulates process failure).
    CrashNode(usize),

    /// Restart a previously crashed node.
    RestartNode(usize),

    /// Inject latency to a specific node.
    SlowNode {
        node: usize,
        delay_ms: u64,
    },

    /// Restore normal latency for a node.
    RestoreNode(usize),

    /// Trigger leader election (step down current leader).
    TriggerElection,

    /// No-op event (useful for timing without side effects).
    Noop,
}

/// Generate a random cluster event for a cluster of given size.
pub fn cluster_event(num_nodes: usize) -> impl Strategy<Value = ClusterEvent> {
    let node_range = 0..num_nodes;

    prop_oneof![
        // Network events
        (node_range.clone()).prop_map(ClusterEvent::PartitionNode),
        (node_range.clone()).prop_map(ClusterEvent::HealNode),
        // Crash events
        (node_range.clone()).prop_map(ClusterEvent::CrashNode),
        (node_range.clone()).prop_map(ClusterEvent::RestartNode),
        // Latency events
        (node_range.clone(), 10u64..500u64).prop_map(|(node, delay_ms)| {
            ClusterEvent::SlowNode { node, delay_ms }
        }),
        (node_range).prop_map(ClusterEvent::RestoreNode),
        // Global events
        Just(ClusterEvent::TriggerElection),
        Just(ClusterEvent::Noop),
    ]
}

/// Generate a sequence of cluster events.
pub fn cluster_event_sequence(
    num_nodes: usize,
    len: usize,
) -> impl Strategy<Value = Vec<ClusterEvent>> {
    prop::collection::vec(cluster_event(num_nodes), len)
}

/// Generate a "safe" event sequence that avoids crashing all nodes.
///
/// Ensures at least one node remains healthy for quorum.
pub fn safe_cluster_event_sequence(
    num_nodes: usize,
    len: usize,
) -> impl Strategy<Value = Vec<ClusterEvent>> {
    // Only allow partition/crash of minority nodes
    let max_failed = num_nodes / 2;

    prop::collection::vec(cluster_event(num_nodes), len).prop_map(move |events| {
        let mut partitioned = 0usize;
        let mut crashed = 0usize;

        events
            .into_iter()
            .filter(|e| {
                match e {
                    ClusterEvent::PartitionNode(_) => {
                        if partitioned < max_failed {
                            partitioned += 1;
                            true
                        } else {
                            false
                        }
                    }
                    ClusterEvent::HealNode(_) => {
                        partitioned = partitioned.saturating_sub(1);
                        true
                    }
                    ClusterEvent::CrashNode(_) => {
                        if crashed < max_failed {
                            crashed += 1;
                            true
                        } else {
                            false
                        }
                    }
                    ClusterEvent::RestartNode(_) => {
                        crashed = crashed.saturating_sub(1);
                        true
                    }
                    _ => true,
                }
            })
            .collect()
    })
}

// =============================================================================
// Network Fault Policy Generator
// =============================================================================

/// Generate a random network fault policy.
pub fn network_fault_policy() -> impl Strategy<Value = NetworkFaultPolicy> {
    (
        0.0f64..0.3f64,          // drop_probability (0-30%)
        0.0f64..0.5f64,          // delay_probability (0-50%)
        (1u64..50u64, 50u64..500u64), // delay_range_ms
        0.0f64..0.3f64,          // reorder_probability (0-30%)
    )
        .prop_map(|(drop_prob, delay_prob, delay_range, reorder_prob)| {
            NetworkFaultPolicy {
                drop_probability: drop_prob,
                delay_probability: delay_prob,
                delay_range_ms: delay_range,
                reorder_probability: reorder_prob,
            }
        })
}

/// Generate a "mild" fault policy suitable for most tests.
pub fn mild_fault_policy() -> impl Strategy<Value = NetworkFaultPolicy> {
    (
        0.0f64..0.05f64,         // drop_probability (0-5%)
        0.0f64..0.2f64,          // delay_probability (0-20%)
        (1u64..10u64, 10u64..100u64), // delay_range_ms
        0.0f64..0.1f64,          // reorder_probability (0-10%)
    )
        .prop_map(|(drop_prob, delay_prob, delay_range, reorder_prob)| {
            NetworkFaultPolicy {
                drop_probability: drop_prob,
                delay_probability: delay_prob,
                delay_range_ms: delay_range,
                reorder_probability: reorder_prob,
            }
        })
}

// =============================================================================
// Test Scenario Generator (Composite)
// =============================================================================

/// A complete test scenario combining operations and cluster events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestScenario {
    /// Sequence of KV operations to execute.
    pub operations: Vec<Operation>,

    /// Cluster events to inject, with timing.
    /// Each tuple is (after_operation_index, event).
    /// The event fires after the operation at that index completes.
    pub events: Vec<(usize, ClusterEvent)>,

    /// Network fault policy to apply throughout the test.
    pub fault_policy: NetworkFaultPolicy,

    /// Random seed for reproducibility.
    pub seed: u64,
}

impl TestScenario {
    /// Get events that should fire after operation at index `op_idx`.
    pub fn events_after(&self, op_idx: usize) -> impl Iterator<Item = &ClusterEvent> {
        self.events
            .iter()
            .filter(move |(idx, _)| *idx == op_idx)
            .map(|(_, event)| event)
    }
}

/// Generate a complete test scenario.
///
/// # Arguments
/// * `num_nodes` - Number of nodes in the cluster
/// * `num_ops` - Number of operations to generate
/// * `num_events` - Number of cluster events to inject
pub fn test_scenario(
    num_nodes: usize,
    num_ops: usize,
    num_events: usize,
) -> impl Strategy<Value = TestScenario> {
    (
        operation_sequence_on_key_set(20, num_ops), // Use 20 keys for overlap
        prop::collection::vec((0..num_ops, cluster_event(num_nodes)), num_events),
        network_fault_policy(),
        any::<u64>(), // seed
    )
        .prop_map(|(operations, events, fault_policy, seed)| TestScenario {
            operations,
            events,
            fault_policy,
            seed,
        })
}

/// Generate a linearizability test scenario.
///
/// Focused on operations that will exercise linearizability checking:
/// - Uses small key set for high conflict rate
/// - Balanced read/write ratio
/// - Includes partition events
pub fn linearizability_scenario(
    num_nodes: usize,
    num_ops: usize,
) -> impl Strategy<Value = TestScenario> {
    let num_events = num_ops / 10; // One event per 10 operations

    (
        prop::collection::vec(
            operation_on_key_set_weighted(10, 50, 40, 10), // 10 keys, balanced r/w
            num_ops,
        ),
        prop::collection::vec(
            (0..num_ops, cluster_event(num_nodes)),
            num_events,
        ),
        mild_fault_policy(),
        any::<u64>(),
    )
        .prop_map(|(operations, events, fault_policy, seed)| TestScenario {
            operations,
            events,
            fault_policy,
            seed,
        })
}

/// Generate a write-heavy stress test scenario (L0 storm).
///
/// High write ratio to stress compaction and WAL.
pub fn write_storm_scenario(num_ops: usize) -> impl Strategy<Value = TestScenario> {
    (
        prop::collection::vec(
            operation_weighted(90, 5, 5), // 90% puts
            num_ops,
        ),
        Just(vec![]), // No cluster events
        Just(NetworkFaultPolicy::clean()),
        any::<u64>(),
    )
        .prop_map(|(operations, events, fault_policy, seed)| TestScenario {
            operations,
            events,
            fault_policy,
            seed,
        })
}

// =============================================================================
// Timing Generators
// =============================================================================

/// Generate a random duration in milliseconds.
pub fn duration_ms(min: u64, max: u64) -> impl Strategy<Value = Duration> {
    (min..=max).prop_map(Duration::from_millis)
}

/// Generate inter-operation delay (simulates think time).
pub fn think_time() -> impl Strategy<Value = Duration> {
    prop_oneof![
        3 => Just(Duration::ZERO),           // 30% immediate
        5 => duration_ms(1, 10),              // 50% short delay
        2 => duration_ms(10, 100),            // 20% longer delay
    ]
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    proptest! {
        #[test]
        fn test_key_generation(k in key()) {
            prop_assert!(!k.is_empty());
            prop_assert!(k.len() <= 64);
        }

        #[test]
        fn test_value_generation(v in value()) {
            prop_assert!(v.len() <= 256);
        }

        #[test]
        fn test_key_from_set(k in key_from_set(10)) {
            let s = String::from_utf8_lossy(&k);
            prop_assert!(s.starts_with("key_"));
        }

        #[test]
        fn test_operation_generation(op in operation()) {
            match op {
                Operation::Get { key } => prop_assert!(!key.is_empty()),
                Operation::Put { key, value: _ } => prop_assert!(!key.is_empty()),
                Operation::Delete { key } => prop_assert!(!key.is_empty()),
            }
        }

        #[test]
        fn test_operation_sequence_length(ops in operation_sequence(50)) {
            prop_assert_eq!(ops.len(), 50);
        }

        #[test]
        fn test_cluster_event_valid_node_index(event in cluster_event(5)) {
            let valid = match event {
                ClusterEvent::PartitionNode(n) => n < 5,
                ClusterEvent::HealNode(n) => n < 5,
                ClusterEvent::CrashNode(n) => n < 5,
                ClusterEvent::RestartNode(n) => n < 5,
                ClusterEvent::SlowNode { node, delay_ms: _ } => node < 5,
                ClusterEvent::RestoreNode(n) => n < 5,
                ClusterEvent::TriggerElection => true,
                ClusterEvent::Noop => true,
            };
            prop_assert!(valid);
        }

        #[test]
        fn test_network_fault_policy_valid(policy in network_fault_policy()) {
            prop_assert!(policy.drop_probability >= 0.0);
            prop_assert!(policy.drop_probability <= 1.0);
            prop_assert!(policy.delay_probability >= 0.0);
            prop_assert!(policy.delay_probability <= 1.0);
            prop_assert!(policy.delay_range_ms.0 <= policy.delay_range_ms.1);
        }

        #[test]
        fn test_scenario_generation(scenario in test_scenario(3, 20, 5)) {
            prop_assert_eq!(scenario.operations.len(), 20);
            prop_assert_eq!(scenario.events.len(), 5);

            // All event indices should be valid
            for (idx, _) in &scenario.events {
                prop_assert!(*idx < 20);
            }
        }

        #[test]
        fn test_linearizability_scenario(scenario in linearizability_scenario(3, 100)) {
            prop_assert_eq!(scenario.operations.len(), 100);

            // Check we have a mix of operations
            let puts = scenario.operations.iter().filter(|op| matches!(op, Operation::Put { .. })).count();
            let gets = scenario.operations.iter().filter(|op| matches!(op, Operation::Get { .. })).count();

            // With 50/40/10 weights, we expect roughly these ratios (but random, so just sanity check)
            prop_assert!(puts > 0, "Should have some puts");
            prop_assert!(gets > 0, "Should have some gets");
        }

        #[test]
        fn test_write_storm_scenario(scenario in write_storm_scenario(100)) {
            prop_assert_eq!(scenario.operations.len(), 100);
            prop_assert!(scenario.events.is_empty());

            // Should be mostly puts
            let puts = scenario.operations.iter().filter(|op| matches!(op, Operation::Put { .. })).count();
            prop_assert!(puts > 70, "Write storm should be mostly puts, got {}", puts);
        }
    }

    #[test]
    fn test_value_with_marker() {
        let v = value_with_marker(3, 42);
        assert_eq!(v.as_ref(), b"value_3_42");
    }

    #[test]
    fn test_scenario_events_after() {
        let scenario = TestScenario {
            operations: vec![
                Operation::Get { key: Bytes::from("k1") },
                Operation::Put { key: Bytes::from("k2"), value: Bytes::from("v2") },
                Operation::Delete { key: Bytes::from("k3") },
            ],
            events: vec![
                (0, ClusterEvent::PartitionNode(1)),
                (0, ClusterEvent::SlowNode { node: 2, delay_ms: 100 }),
                (2, ClusterEvent::HealNode(1)),
            ],
            fault_policy: NetworkFaultPolicy::clean(),
            seed: 12345,
        };

        let events_0: Vec<_> = scenario.events_after(0).collect();
        assert_eq!(events_0.len(), 2);

        let events_1: Vec<_> = scenario.events_after(1).collect();
        assert_eq!(events_1.len(), 0);

        let events_2: Vec<_> = scenario.events_after(2).collect();
        assert_eq!(events_2.len(), 1);
    }
}
