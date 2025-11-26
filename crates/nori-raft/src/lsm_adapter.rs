//! Adapter to use LsmEngine as a Raft state machine.
//!
//! Bridges nori-lsm's LsmStateMachine to nori-raft's StateMachine trait.

use crate::error::{RaftError, Result};
use crate::snapshot::StateMachine;
use bytes::Bytes;
use nori_lsm::raft_sm::LsmStateMachine;
use tokio::sync::Mutex;

/// Adapter that implements Raft's StateMachine trait for LsmStateMachine.
///
/// This allows an LSM engine to be used as a replicated state machine in Raft.
/// Commands are serialized and applied through the Raft log for consensus.
pub struct LsmStateMachineAdapter {
    inner: Mutex<LsmStateMachine>,
}

impl LsmStateMachineAdapter {
    /// Create a new adapter wrapping an LsmStateMachine.
    pub fn new(lsm_sm: LsmStateMachine) -> Self {
        Self {
            inner: Mutex::new(lsm_sm),
        }
    }

    /// Get a reference to the wrapped LSM state machine.
    ///
    /// Note: This requires locking, so prefer to use sparingly.
    pub async fn lsm(&self) -> tokio::sync::MutexGuard<'_, LsmStateMachine> {
        self.inner.lock().await
    }
}

impl StateMachine for LsmStateMachineAdapter {
    /// Apply a command to the LSM engine.
    ///
    /// Called when a Raft log entry is committed.
    /// Commands must be serialized using LsmStateMachine's Command format.
    fn apply(&mut self, command: &[u8]) -> Result<()> {
        // We need to call async apply_command, but StateMachine trait is sync
        // Solution: Use tokio's block_in_place to run async code in sync context
        let inner = self.inner.get_mut();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                inner
                    .apply_command(command)
                    .await
                    .map_err(|e| RaftError::Internal {
                        reason: format!("LSM apply failed: {}", e),
                    })
            })
        })
    }

    /// Create a snapshot of the LSM state.
    ///
    /// Returns serialized LSM state that can be sent to followers.
    fn snapshot(&self) -> Result<Bytes> {
        // block_in_place doesn't work with immutable self, so we use tokio::task::block_in_place
        // with a lock
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let inner = self.inner.lock().await;
                inner.create_snapshot().map_err(|e| RaftError::Internal {
                    reason: format!("LSM snapshot failed: {}", e),
                })
            })
        })
    }

    /// Restore LSM state from a snapshot.
    ///
    /// Replaces the entire LSM state with the snapshot.
    fn restore(&mut self, snapshot: &[u8]) -> Result<()> {
        let inner = self.inner.get_mut();

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                inner
                    .restore_snapshot(snapshot)
                    .map_err(|e| RaftError::Internal {
                        reason: format!("LSM restore failed: {}", e),
                    })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_lsm::{ATLLConfig, LsmEngine, Version};
    use std::sync::Arc;
    use tempfile::TempDir;

    /// Helper to extract just the value from get() result, ignoring version
    fn get_value(result: Option<(Bytes, Version)>) -> Option<Bytes> {
        result.map(|(v, _)| v)
    }

    async fn create_test_adapter() -> (LsmStateMachineAdapter, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = Arc::new(LsmEngine::open(config).await.unwrap());
        let lsm_sm = LsmStateMachine::new(engine);
        let adapter = LsmStateMachineAdapter::new(lsm_sm);

        (adapter, temp_dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_adapter_apply() {
        let (mut adapter, _temp) = create_test_adapter().await;

        // Create a Put command
        use nori_lsm::raft_sm::Command;
        let cmd = Command::Put {
            key: Bytes::from("key1"),
            value: Bytes::from("value1"),
            ttl: None,
        };

        let serialized = cmd.serialize().unwrap();

        // Apply via adapter
        adapter.apply(&serialized).unwrap();

        // Verify via LSM
        let lsm = adapter.lsm().await;
        let result = lsm.engine().get(b"key1").await.unwrap();
        assert_eq!(get_value(result), Some(Bytes::from("value1")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_adapter_snapshot() {
        let (adapter, _temp) = create_test_adapter().await;

        // Write some data first
        use nori_lsm::raft_sm::Command;
        let cmd = Command::Put {
            key: Bytes::from("snap_key"),
            value: Bytes::from("snap_value"),
            ttl: None,
        };
        let serialized = cmd.serialize().unwrap();
        adapter.inner.lock().await.apply_command(&serialized).await.unwrap();

        // Create snapshot - should now contain actual data
        let snapshot = adapter.snapshot().unwrap();

        // Snapshot should contain serialized manifest data
        assert!(
            snapshot.len() > 0,
            "Snapshot should contain data (got {} bytes)",
            snapshot.len()
        );

        // Verify snapshot contains valid serialized data by checking it's not empty
        // and has reasonable size (manifest snapshots are typically 200-500 bytes)
        assert!(
            snapshot.len() < 10000,
            "Snapshot size seems unreasonable: {} bytes",
            snapshot.len()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_adapter_restore() {
        let (mut adapter, _temp) = create_test_adapter().await;

        // Write some data and create a snapshot
        use nori_lsm::raft_sm::Command;
        let cmd = Command::Put {
            key: Bytes::from("restore_key"),
            value: Bytes::from("restore_value"),
            ttl: None,
        };
        let serialized = cmd.serialize().unwrap();
        adapter.inner.get_mut().apply_command(&serialized).await.unwrap();

        let snapshot = adapter.snapshot().unwrap();

        // Restore from snapshot
        // Note: Full restoration requires SSTable file transfer, which isn't implemented yet
        // So we expect this to fail with a specific error message
        let result = adapter.restore(&snapshot);

        // Should fail with "not fully implemented" message
        assert!(
            result.is_err(),
            "Restore should fail until SSTable file transfer is implemented"
        );

        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("not fully implemented") || error_msg.contains("SSTable file transfer"),
                "Expected 'not fully implemented' error, got: {}",
                error_msg
            );
        }
    }
}
