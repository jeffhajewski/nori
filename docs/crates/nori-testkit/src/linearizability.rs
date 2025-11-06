//! Linearizability checker for KV store operations.
//!
//! Implements a simplified Wing & Gong algorithm for verifying linearizability.
//! For KV stores, we can use a state-based approach where each operation
//! either reads or modifies a single key.
//!
//! # Algorithm Overview
//!
//! 1. Record all operations with real-time ordering (invocation → return)
//! 2. Try to find a valid sequential ordering that:
//!    - Respects real-time order (if op1 completes before op2 starts, op1 must precede op2)
//!    - Produces correct results (reads return values from previous writes)
//!
//! # Complexity
//!
//! For n operations, worst case is O(n!) but with pruning typically O(n^2) or better.

use bytes::Bytes;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;

/// Unique identifier for an operation (typically thread/client ID + sequence number)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OperationId(pub u64);

/// Timestamp for operation ordering
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn now() -> Self {
        // Use nanoseconds since some epoch for high precision
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        let elapsed = start.elapsed();
        Timestamp(elapsed.as_nanos() as u64)
    }
}

/// KV store operation types
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Get(key) → reads value
    Get { key: Bytes },
    /// Put(key, value) → writes value
    Put { key: Bytes, value: Bytes },
    /// Delete(key) → removes key
    Delete { key: Bytes },
}

/// Result of an operation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationResult {
    /// Get returned a value
    GetOk(Option<Bytes>),
    /// Put succeeded
    PutOk,
    /// Delete succeeded
    DeleteOk,
    /// Operation failed (not leader, timeout, etc.)
    Error(String),
}

/// A completed operation with timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedOperation {
    pub id: OperationId,
    pub op: Operation,
    pub result: OperationResult,
    pub invoke_time: Timestamp,
    pub return_time: Timestamp,
}

impl CompletedOperation {
    /// Returns true if this operation happened before `other` in real time
    pub fn happens_before(&self, other: &CompletedOperation) -> bool {
        self.return_time < other.invoke_time
    }

    /// Returns true if operations overlap in time (concurrent)
    pub fn concurrent_with(&self, other: &CompletedOperation) -> bool {
        !(self.happens_before(other) || other.happens_before(self))
    }
}

/// History of operations for linearizability checking
#[derive(Debug, Clone, Default)]
pub struct History {
    /// All completed operations
    operations: Vec<CompletedOperation>,
    /// In-flight operations (invoked but not yet returned)
    pending: Arc<Mutex<HashMap<OperationId, (Operation, Timestamp)>>>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the invocation of an operation
    pub fn invoke(&self, id: OperationId, op: Operation) {
        let timestamp = Timestamp::now();
        self.pending.lock().insert(id, (op, timestamp));
    }

    /// Record the return of an operation
    pub fn complete(&mut self, id: OperationId, result: OperationResult) {
        let return_time = Timestamp::now();

        if let Some((op, invoke_time)) = self.pending.lock().remove(&id) {
            self.operations.push(CompletedOperation {
                id,
                op,
                result,
                invoke_time,
                return_time,
            });
        }
    }

    /// Get all completed operations
    pub fn operations(&self) -> &[CompletedOperation] {
        &self.operations
    }

    /// Check if the history is linearizable
    ///
    /// Uses a state-based search to find a valid linearization.
    /// Returns Ok(()) if linearizable, Err with counterexample otherwise.
    pub fn check_linearizability(&self) -> Result<(), LinearizabilityError> {
        if self.operations.is_empty() {
            return Ok(());
        }

        // Build happens-before graph
        let n = self.operations.len();
        let mut must_precede: Vec<Vec<usize>> = vec![Vec::new(); n];

        for i in 0..n {
            for j in 0..n {
                if i != j && self.operations[i].happens_before(&self.operations[j]) {
                    must_precede[i].push(j);
                }
            }
        }

        // Try to find a valid linearization using backtracking
        let mut state = KvState::new();
        let mut linearization = Vec::new();
        let mut remaining: Vec<usize> = (0..n).collect();

        if self.try_linearize(&mut state, &mut linearization, &mut remaining, &must_precede) {
            Ok(())
        } else {
            Err(LinearizabilityError::NotLinearizable {
                operations: self.operations.clone(),
                partial_order: must_precede,
            })
        }
    }

    /// Recursive backtracking search for valid linearization
    fn try_linearize(
        &self,
        state: &mut KvState,
        linearization: &mut Vec<usize>,
        remaining: &mut Vec<usize>,
        must_precede: &[Vec<usize>],
    ) -> bool {
        // Base case: all operations linearized
        if remaining.is_empty() {
            return true;
        }

        // Try each remaining operation
        for i in 0..remaining.len() {
            let op_idx = remaining[i];

            // Check if all operations that must come before this one are already linearized
            // We can schedule op_idx if no remaining operation has op_idx as a successor
            let can_schedule = remaining.iter().all(|&other_idx| {
                other_idx == op_idx || !must_precede[other_idx].contains(&op_idx)
            });

            if !can_schedule {
                continue;
            }

            // Try to execute this operation
            let operation = &self.operations[op_idx];
            let snapshot = state.snapshot();

            if state.execute(operation) {
                // Valid execution, continue search
                linearization.push(op_idx);
                remaining.remove(i);

                if self.try_linearize(state, linearization, remaining, must_precede) {
                    return true;
                }

                // Backtrack
                remaining.insert(i, op_idx);
                linearization.pop();
            }

            state.restore(snapshot);
        }

        false
    }
}

/// KV store state for linearizability checking
#[derive(Debug, Clone)]
struct KvState {
    store: BTreeMap<Bytes, Bytes>,
}

impl KvState {
    fn new() -> Self {
        Self {
            store: BTreeMap::new(),
        }
    }

    fn snapshot(&self) -> BTreeMap<Bytes, Bytes> {
        self.store.clone()
    }

    fn restore(&mut self, snapshot: BTreeMap<Bytes, Bytes>) {
        self.store = snapshot;
    }

    /// Execute an operation and verify the result matches expectation
    fn execute(&mut self, op: &CompletedOperation) -> bool {
        match (&op.op, &op.result) {
            (Operation::Get { key }, OperationResult::GetOk(expected)) => {
                let actual = self.store.get(key).cloned();
                actual.as_ref() == expected.as_ref()
            }
            (Operation::Put { key, value }, OperationResult::PutOk) => {
                self.store.insert(key.clone(), value.clone());
                true
            }
            (Operation::Delete { key }, OperationResult::DeleteOk) => {
                self.store.remove(key);
                true
            }
            (_, OperationResult::Error(_)) => {
                // Errors can happen at any time, don't affect state
                true
            }
            _ => {
                // Mismatch between operation and result
                false
            }
        }
    }
}

/// Linearizability checking error
#[derive(Debug, Error)]
pub enum LinearizabilityError {
    #[error("History is not linearizable")]
    NotLinearizable {
        operations: Vec<CompletedOperation>,
        partial_order: Vec<Vec<usize>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_linearizable_sequential_puts() {
        let mut history = History::new();

        // Sequential operations: Put(k1) → Put(k2) → Get(k1)
        history.invoke(OperationId(1), Operation::Put {
            key: Bytes::from("k1"),
            value: Bytes::from("v1"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(1), OperationResult::PutOk);

        std::thread::sleep(Duration::from_millis(10));

        history.invoke(OperationId(2), Operation::Put {
            key: Bytes::from("k2"),
            value: Bytes::from("v2"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(2), OperationResult::PutOk);

        std::thread::sleep(Duration::from_millis(10));

        history.invoke(OperationId(3), Operation::Get {
            key: Bytes::from("k1"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(3), OperationResult::GetOk(Some(Bytes::from("v1"))));

        assert!(history.check_linearizability().is_ok());
    }

    #[test]
    fn test_nonlinearizable_stale_read() {
        let mut history = History::new();

        // Put(k1, v1) → Put(k1, v2) → Get(k1) returns v1 (STALE!)
        history.invoke(OperationId(1), Operation::Put {
            key: Bytes::from("k1"),
            value: Bytes::from("v1"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(1), OperationResult::PutOk);

        std::thread::sleep(Duration::from_millis(10));

        history.invoke(OperationId(2), Operation::Put {
            key: Bytes::from("k1"),
            value: Bytes::from("v2"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(2), OperationResult::PutOk);

        std::thread::sleep(Duration::from_millis(10));

        history.invoke(OperationId(3), Operation::Get {
            key: Bytes::from("k1"),
        });
        std::thread::sleep(Duration::from_millis(10));
        history.complete(OperationId(3), OperationResult::GetOk(Some(Bytes::from("v1"))));

        assert!(history.check_linearizability().is_err());
    }

    #[test]
    fn test_linearizable_concurrent_operations() {
        let mut history = History::new();

        // Concurrent Put and Get can be linearized
        let t1 = Timestamp(100);
        let t2 = Timestamp(150);
        let t3 = Timestamp(200);
        let t4 = Timestamp(250);

        // Operation 1: Put(k1, v1) [100-200]
        history.operations.push(CompletedOperation {
            id: OperationId(1),
            op: Operation::Put {
                key: Bytes::from("k1"),
                value: Bytes::from("v1"),
            },
            result: OperationResult::PutOk,
            invoke_time: t1,
            return_time: t3,
        });

        // Operation 2: Get(k1) → Some(v1) [150-250] (overlaps with Put)
        history.operations.push(CompletedOperation {
            id: OperationId(2),
            op: Operation::Get {
                key: Bytes::from("k1"),
            },
            result: OperationResult::GetOk(Some(Bytes::from("v1"))),
            invoke_time: t2,
            return_time: t4,
        });

        // Should be linearizable: Put → Get
        assert!(history.check_linearizability().is_ok());
    }
}
