//! Raft state machine implementation for LsmEngine.
//!
//! Provides StateMachine trait implementation to integrate LSM with Raft consensus.
//! Commands are serialized as bincode and applied atomically to the LSM engine.

use crate::{Error, LsmEngine, Result};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

/// Command that can be replicated via Raft.
///
/// Represents atomic operations on the LSM engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Command {
    /// Put key-value pair with optional TTL
    Put {
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    },
    /// Delete key
    Delete { key: Bytes },
}

impl Command {
    /// Serialize command to bytes using bincode.
    pub fn serialize(&self) -> Result<Bytes> {
        bincode::serialize(self)
            .map(Bytes::from)
            .map_err(|e| Error::Internal(format!("Failed to serialize command: {}", e)))
    }

    /// Deserialize command from bytes using bincode.
    pub fn deserialize(data: &[u8]) -> Result<Self> {
        bincode::deserialize(data)
            .map_err(|e| Error::Internal(format!("Failed to deserialize command: {}", e)))
    }
}

/// Wrapper for LsmEngine that implements Raft's StateMachine trait.
///
/// This adapter allows LSM to be used as a replicated state machine in Raft.
/// Commands are serialized and applied through the Raft log.
pub struct LsmStateMachine {
    engine: Arc<LsmEngine>,
}

impl LsmStateMachine {
    /// Create a new LSM state machine wrapper.
    pub fn new(engine: Arc<LsmEngine>) -> Self {
        Self { engine }
    }

    /// Get reference to the underlying LSM engine.
    pub fn engine(&self) -> &Arc<LsmEngine> {
        &self.engine
    }
}

// Note: We can't implement the Raft StateMachine trait directly here
// because it would create a circular dependency (nori-lsm -> nori-raft -> nori-lsm).
// Instead, we provide an async apply method that can be called from a Raft adapter.

impl LsmStateMachine {
    /// Apply a command to the LSM engine.
    ///
    /// Called by Raft when a log entry is committed.
    /// Commands are deserialized and applied atomically.
    pub async fn apply_command(&mut self, command: &[u8]) -> Result<()> {
        let cmd = Command::deserialize(command)?;

        match cmd {
            Command::Put { key, value, ttl } => {
                self.engine.put(key, value, ttl).await?;
            }
            Command::Delete { key } => {
                self.engine.delete(&key).await?;
            }
        }

        Ok(())
    }

    /// Create a snapshot of the LSM state.
    ///
    /// For now, returns empty snapshot. Full snapshot support would require:
    /// - Iterating over all LSM levels
    /// - Serializing SSTable file references
    /// - Pausing compaction during snapshot
    ///
    /// TODO: Implement full snapshot in Phase 4 of integration.
    pub fn create_snapshot(&self) -> Result<Bytes> {
        // Placeholder: empty snapshot
        // In production, this would serialize the manifest + SSTable references
        Ok(Bytes::new())
    }

    /// Restore LSM state from a snapshot.
    ///
    /// For now, does nothing. Full restore would require:
    /// - Deserializing manifest
    /// - Copying/linking SSTable files
    /// - Rebuilding memtable
    ///
    /// TODO: Implement full restore in Phase 4 of integration.
    pub fn restore_snapshot(&mut self, _snapshot: &[u8]) -> Result<()> {
        // Placeholder: no-op
        // In production, this would restore manifest + SSTable files
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ATLLConfig;
    use tempfile::TempDir;

    async fn create_test_engine() -> (Arc<LsmEngine>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let mut config = ATLLConfig::default();
        config.data_dir = temp_dir.path().to_path_buf();

        let engine = Arc::new(LsmEngine::open(config).await.unwrap());
        (engine, temp_dir)
    }

    #[tokio::test]
    async fn test_command_serialization() {
        let cmd = Command::Put {
            key: Bytes::from("test_key"),
            value: Bytes::from("test_value"),
            ttl: None,
        };

        let serialized = cmd.serialize().unwrap();
        let deserialized = Command::deserialize(&serialized).unwrap();

        match deserialized {
            Command::Put { key, value, ttl } => {
                assert_eq!(key, Bytes::from("test_key"));
                assert_eq!(value, Bytes::from("test_value"));
                assert_eq!(ttl, None);
            }
            _ => panic!("Expected Put command"),
        }
    }

    #[tokio::test]
    async fn test_apply_put_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine.clone());

        let cmd = Command::Put {
            key: Bytes::from("key1"),
            value: Bytes::from("value1"),
            ttl: None,
        };

        let serialized = cmd.serialize().unwrap();
        sm.apply_command(&serialized).await.unwrap();

        // Verify the value was written
        let result = engine.get(b"key1").await.unwrap();
        assert_eq!(result, Some(Bytes::from("value1")));
    }

    #[tokio::test]
    async fn test_apply_delete_command() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine.clone());

        // First write a value
        engine.put(Bytes::from("key1"), Bytes::from("value1"), None)
            .await
            .unwrap();

        // Then delete it via command
        let cmd = Command::Delete {
            key: Bytes::from("key1"),
        };

        let serialized = cmd.serialize().unwrap();
        sm.apply_command(&serialized).await.unwrap();

        // Verify the value was deleted
        let result = engine.get(b"key1").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_snapshot_create_restore() {
        let (engine, _temp) = create_test_engine().await;
        let mut sm = LsmStateMachine::new(engine);

        // Create snapshot (currently no-op)
        let snapshot = sm.create_snapshot().unwrap();
        assert_eq!(snapshot.len(), 0);

        // Restore snapshot (currently no-op)
        sm.restore_snapshot(&snapshot).unwrap();
    }
}
