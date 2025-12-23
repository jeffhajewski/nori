//! Chaos simulator and linearizability checker for distributed systems testing.
//!
//! This crate provides tools for validating distributed systems correctness:
//! - Linearizability checking (Jepsen-style)
//! - Fault injection (network, disk, crashes)
//! - Property-based test generators
//!
//! # Usage
//!
//! ```ignore
//! use nori_testkit::linearizability::*;
//!
//! let mut history = History::new();
//! history.record_invoke(1, Operation::Put("key", "value"));
//! history.record_return(1, Result::Ok(()));
//!
//! assert!(history.is_linearizable());
//! ```

pub mod linearizability;
pub mod fault_injection;
pub mod generators;

// Re-export key types
pub use linearizability::{History, Operation, OperationResult, LinearizabilityError};
pub use fault_injection::{
    DeterministicRng, NetworkFaultPolicy, NetworkFaultInjector, FaultAction, ChaosScheduler,
};
pub use generators::{ClusterEvent, TestScenario};
