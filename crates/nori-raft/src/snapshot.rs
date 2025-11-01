//! Raft snapshot support for log compaction.
//!
//! Per Raft paper §7 (Log compaction):
//! - Snapshots capture the state machine's state at a point in time
//! - Allow Raft to discard old log entries
//! - Used to bring slow followers up to date (InstallSnapshot RPC)
//!
//! # Design
//!
//! A snapshot contains:
//! - **last_included_index**: Log index of last entry in snapshot
//! - **last_included_term**: Term of last entry in snapshot
//! - **config**: Cluster configuration at snapshot point
//! - **data**: State machine state (opaque bytes)
//!
//! # Snapshot Creation
//!
//! Leader creates snapshots periodically when log grows too large:
//! 1. State machine serializes its state
//! 2. Snapshot metadata recorded (index, term, config)
//! 3. Log entries ≤ last_included_index can be discarded
//!
//! # Snapshot Installation
//!
//! Follower installs snapshot from leader when:
//! - Follower too far behind (leader already compacted needed entries)
//! - Faster than replaying all log entries
//!
//! Process:
//! 1. Leader sends InstallSnapshot RPC
//! 2. Follower replaces state machine with snapshot data
//! 3. Follower discards entire log
//! 4. Follower updates last_applied and commit_index

use crate::error::{RaftError, Result};
use crate::types::*;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Snapshot metadata.
///
/// Describes the point in the log where the snapshot was taken.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotMetadata {
    /// Index of last log entry included in snapshot
    pub last_included_index: LogIndex,

    /// Term of last log entry included in snapshot
    pub last_included_term: Term,

    /// Cluster configuration at snapshot point
    pub config: ConfigEntry,
}

/// Complete snapshot (metadata + data).
///
/// The data is the serialized state machine state.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Snapshot metadata
    pub metadata: SnapshotMetadata,

    /// State machine state (serialized)
    pub data: Bytes,
}

impl Snapshot {
    /// Create a new snapshot.
    pub fn new(
        last_included_index: LogIndex,
        last_included_term: Term,
        config: ConfigEntry,
        data: Bytes,
    ) -> Self {
        Self {
            metadata: SnapshotMetadata {
                last_included_index,
                last_included_term,
                config,
            },
            data,
        }
    }

    /// Get the size of the snapshot data in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Serialize snapshot to writer.
    ///
    /// Format:
    /// 1. Metadata (bincode)
    /// 2. Data length (u64 little-endian)
    /// 3. Data (raw bytes)
    pub fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        // Write metadata
        let metadata_bytes = bincode::serialize(&self.metadata).map_err(|e| {
            RaftError::Internal {
                reason: format!("Failed to serialize snapshot metadata: {}", e),
            }
        })?;

        let metadata_len = metadata_bytes.len() as u64;
        writer
            .write_all(&metadata_len.to_le_bytes())
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to write metadata length: {}", e),
            })?;

        writer
            .write_all(&metadata_bytes)
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to write metadata: {}", e),
            })?;

        // Write data length
        let data_len = self.data.len() as u64;
        writer
            .write_all(&data_len.to_le_bytes())
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to write data length: {}", e),
            })?;

        // Write data
        writer.write_all(&self.data).map_err(|e| RaftError::Internal {
            reason: format!("Failed to write snapshot data: {}", e),
        })?;

        Ok(())
    }

    /// Deserialize snapshot from reader.
    pub fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        // Read metadata length
        let mut len_buf = [0u8; 8];
        reader
            .read_exact(&mut len_buf)
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to read metadata length: {}", e),
            })?;
        let metadata_len = u64::from_le_bytes(len_buf) as usize;

        // Read metadata
        let mut metadata_bytes = vec![0u8; metadata_len];
        reader
            .read_exact(&mut metadata_bytes)
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to read metadata: {}", e),
            })?;

        let metadata: SnapshotMetadata =
            bincode::deserialize(&metadata_bytes).map_err(|e| RaftError::Internal {
                reason: format!("Failed to deserialize metadata: {}", e),
            })?;

        // Read data length
        reader
            .read_exact(&mut len_buf)
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to read data length: {}", e),
            })?;
        let data_len = u64::from_le_bytes(len_buf) as usize;

        // Read data
        let mut data_bytes = vec![0u8; data_len];
        reader
            .read_exact(&mut data_bytes)
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to read snapshot data: {}", e),
            })?;

        Ok(Self {
            metadata,
            data: Bytes::from(data_bytes),
        })
    }
}

/// State machine trait for Raft.
///
/// Implemented by the application (e.g., LSM engine) to provide
/// snapshotting and command application.
pub trait StateMachine: Send + Sync {
    /// Apply a committed command to the state machine.
    ///
    /// Called when Raft commits a log entry.
    /// The command is opaque bytes that the state machine interprets.
    fn apply(&mut self, command: &[u8]) -> Result<()>;

    /// Create a snapshot of the current state machine state.
    ///
    /// Returns the serialized state as bytes.
    /// Should be deterministic and complete.
    fn snapshot(&self) -> Result<Bytes>;

    /// Restore state machine from a snapshot.
    ///
    /// Replaces the entire state machine state with the snapshot.
    /// Called when installing a snapshot from leader.
    fn restore(&mut self, snapshot: &[u8]) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_snapshot_creation() {
        let config = ConfigEntry::Single(vec![
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
        ]);

        let snapshot = Snapshot::new(
            LogIndex(100),
            Term(5),
            config.clone(),
            Bytes::from("test data"),
        );

        assert_eq!(snapshot.metadata.last_included_index, LogIndex(100));
        assert_eq!(snapshot.metadata.last_included_term, Term(5));
        assert_eq!(snapshot.metadata.config, config);
        assert_eq!(snapshot.data, Bytes::from("test data"));
        assert_eq!(snapshot.size(), 9);
    }

    #[test]
    fn test_snapshot_serialization() {
        let config = ConfigEntry::Single(vec![NodeId::new("n1")]);
        let snapshot = Snapshot::new(
            LogIndex(42),
            Term(7),
            config,
            Bytes::from("hello world"),
        );

        // Serialize
        let mut buffer = Vec::new();
        snapshot.write_to(&mut buffer).unwrap();

        // Deserialize
        let mut cursor = Cursor::new(buffer);
        let restored = Snapshot::read_from(&mut cursor).unwrap();

        // Verify
        assert_eq!(restored.metadata, snapshot.metadata);
        assert_eq!(restored.data, snapshot.data);
    }

    #[test]
    fn test_snapshot_large_data() {
        let config = ConfigEntry::Single(vec![NodeId::new("n1")]);
        let large_data = vec![0u8; 1024 * 1024]; // 1MB
        let snapshot = Snapshot::new(
            LogIndex(1000),
            Term(10),
            config,
            Bytes::from(large_data.clone()),
        );

        assert_eq!(snapshot.size(), 1024 * 1024);

        // Serialize and deserialize
        let mut buffer = Vec::new();
        snapshot.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let restored = Snapshot::read_from(&mut cursor).unwrap();

        assert_eq!(restored.data.len(), large_data.len());
        assert_eq!(restored.data, Bytes::from(large_data));
    }

    #[test]
    fn test_snapshot_empty_data() {
        let config = ConfigEntry::Single(vec![NodeId::new("n1")]);
        let snapshot = Snapshot::new(LogIndex(0), Term(0), config, Bytes::new());

        assert_eq!(snapshot.size(), 0);

        let mut buffer = Vec::new();
        snapshot.write_to(&mut buffer).unwrap();

        let mut cursor = Cursor::new(buffer);
        let restored = Snapshot::read_from(&mut cursor).unwrap();

        assert_eq!(restored.data.len(), 0);
    }
}
