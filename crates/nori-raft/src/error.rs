//! Raft error types.

use thiserror::Error;

use crate::types::{LogIndex, NodeId, Term};

/// Raft errors.
#[derive(Error, Debug)]
pub enum RaftError {
    /// Not the leader (cannot handle write/propose).
    #[error("Not leader (current leader: {leader:?})")]
    NotLeader { leader: Option<NodeId> },

    /// Quorum unavailable (not enough replicas reachable).
    #[error("Quorum unavailable (need {needed}, have {available})")]
    QuorumUnavailable { needed: usize, available: usize },

    /// Commit timeout (write took too long to commit).
    #[error("Commit timeout after {elapsed_ms}ms")]
    CommitTimeout { elapsed_ms: u64 },

    /// Term mismatch (request from old term).
    #[error("Term mismatch (current: {current}, request: {request})")]
    TermMismatch { current: Term, request: Term },

    /// Log inconsistency (follower log doesn't match leader's prev_log).
    #[error("Log inconsistency at index {index} (expected term {expected_term}, got {actual_term:?})")]
    LogInconsistency {
        index: LogIndex,
        expected_term: Term,
        actual_term: Option<Term>,
    },

    /// Snapshot install failed.
    #[error("Snapshot install failed: {reason}")]
    SnapshotFailed { reason: String },

    /// Configuration error (invalid Raft config).
    #[error("Configuration error: {reason}")]
    ConfigError { reason: String },

    /// I/O error (WAL, network, etc.).
    #[error("I/O error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// Serialization error.
    #[error("Serialization error: {source}")]
    Serialization {
        #[from]
        source: bincode::Error,
    },

    /// WAL/Storage error.
    #[error("Storage error: {source}")]
    Storage {
        #[from]
        source: nori_wal::SegmentError,
    },

    /// Internal error (bug).
    #[error("Internal error: {reason}")]
    Internal { reason: String },
}

/// Raft result type.
pub type Result<T> = std::result::Result<T, RaftError>;
