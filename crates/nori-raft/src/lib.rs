//! nori-raft: Raft consensus with read-index and leases.
//!
//! Full-featured Raft implementation with:
//! - Leader election with randomized timeouts
//! - Log replication with efficient backtracking
//! - Leader leases for fast linearizable reads
//! - Read-index for fallback linearizable reads
//! - InstallSnapshot for log compaction
//! - Joint consensus for safe membership changes
//! - VizEvent observability integration
//!
//! Based on the Raft paper (Ongaro & Ousterhout, 2014) with extensions.

pub mod config;
pub mod error;
pub mod types;
pub mod transport;
pub mod log;
pub mod state;
pub mod timer;
pub mod election;
pub mod replication;
pub mod raft;
pub mod lease;
pub mod read_index;

// Core modules (to be implemented)
// pub mod snapshot;
// pub mod reconfig;
// pub mod metrics;

pub use config::RaftConfig;
pub use error::{RaftError, Result};
pub use types::*;
pub use raft::Raft;

/// ReplicatedLog trait (per context/31_consensus.yaml).
///
/// Provides a high-level interface to the Raft consensus module.
/// This trait is implemented by the main Raft struct.
#[async_trait::async_trait]
pub trait ReplicatedLog: Send + Sync {
    /// Propose a new command to be replicated.
    ///
    /// If this node is the leader, the command is appended to the log and replicated.
    /// Returns the log index where the command was stored.
    ///
    /// Returns `NotLeader` error if this node is not the leader.
    async fn propose(&self, cmd: bytes::Bytes) -> Result<LogIndex>;

    /// Perform a linearizable read (via read-index).
    ///
    /// Ensures that any read following this call sees all writes committed before the call.
    /// Uses read-index protocol (quorum check) if leader lease expired.
    ///
    /// Returns `NotLeader` error if this node is not the leader.
    async fn read_index(&self) -> Result<()>;

    /// Check if this node is the leader.
    fn is_leader(&self) -> bool;

    /// Get the current leader (if known).
    fn leader(&self) -> Option<NodeId>;

    /// Install a snapshot from the leader.
    ///
    /// Used when a follower is too far behind (log compacted).
    /// The snapshot contains the full state machine state up to a point.
    async fn install_snapshot(&self, snap: Box<dyn std::io::Read + Send>) -> Result<()>;

    /// Subscribe to applied log entries.
    ///
    /// Returns a channel that receives (log_index, command) pairs as they are applied.
    /// The state machine should consume from this channel to apply commands.
    fn subscribe_applied(&self) -> tokio::sync::mpsc::Receiver<(LogIndex, bytes::Bytes)>;
}
