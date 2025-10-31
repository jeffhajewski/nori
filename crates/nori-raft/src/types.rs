//! Core Raft types: Term, Index, Log Entries, RPC messages.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Raft term number (monotonically increasing).
///
/// Terms establish logical clocks in Raft. Each term has at most one leader.
/// When a server starts an election, it increments its term.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Term(pub u64);

impl Term {
    pub const ZERO: Term = Term(0);

    pub fn next(self) -> Term {
        Term(self.0 + 1)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Term {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}", self.0)
    }
}

/// Log index (1-indexed, 0 is sentinel for "no entry").
///
/// Raft logs are 1-indexed. Index 0 represents "no entry" or "before the log".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LogIndex(pub u64);

impl LogIndex {
    pub const ZERO: LogIndex = LogIndex(0);

    pub fn next(self) -> LogIndex {
        LogIndex(self.0 + 1)
    }

    pub fn prev(self) -> Option<LogIndex> {
        if self.0 > 0 {
            Some(LogIndex(self.0 - 1))
        } else {
            None
        }
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LogIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "I{}", self.0)
    }
}

/// Node identifier (unique across cluster).
///
/// NodeId is a string to support DNS names, UUIDs, or IP:port combinations.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub String);

impl NodeId {
    pub fn new(id: impl Into<String>) -> Self {
        NodeId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Log entry (command + metadata).
///
/// Each entry contains:
/// - `term`: Term when entry was created (for conflict detection)
/// - `index`: Position in log (for addressing)
/// - `command`: Opaque command bytes (interpreted by state machine)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub term: Term,
    pub index: LogIndex,
    pub command: Bytes,
}

impl LogEntry {
    pub fn new(term: Term, index: LogIndex, command: Bytes) -> Self {
        Self {
            term,
            index,
            command,
        }
    }
}

/// RequestVote RPC request.
///
/// Sent by candidate to all peers during election.
/// Asks for vote to become leader in current term.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVoteRequest {
    /// Candidate's term
    pub term: Term,

    /// Candidate requesting vote
    pub candidate_id: NodeId,

    /// Index of candidate's last log entry
    pub last_log_index: LogIndex,

    /// Term of candidate's last log entry
    pub last_log_term: Term,
}

/// RequestVote RPC response.
///
/// Sent by voter back to candidate.
/// Grants or denies vote based on log up-to-dateness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestVoteResponse {
    /// Current term, for candidate to update itself
    pub term: Term,

    /// True if candidate received vote
    pub vote_granted: bool,
}

/// AppendEntries RPC request.
///
/// Sent by leader to replicate log entries and/or send heartbeats.
/// Empty entries list = heartbeat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesRequest {
    /// Leader's term
    pub term: Term,

    /// Leader's ID (so follower can redirect clients)
    pub leader_id: NodeId,

    /// Index of log entry immediately preceding new ones
    pub prev_log_index: LogIndex,

    /// Term of prev_log_index entry
    pub prev_log_term: Term,

    /// Log entries to store (empty for heartbeat)
    pub entries: Vec<LogEntry>,

    /// Leader's commit index
    pub leader_commit: LogIndex,
}

/// AppendEntries RPC response.
///
/// Sent by follower back to leader.
/// Success indicates log consistency check passed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppendEntriesResponse {
    /// Current term, for leader to update itself
    pub term: Term,

    /// True if follower contained entry matching prev_log_index/term
    pub success: bool,

    /// Hint for leader to backtrack faster on conflict
    pub conflict_index: Option<LogIndex>,

    /// Follower's last log index (for match_index tracking)
    pub last_log_index: LogIndex,
}

/// InstallSnapshot RPC request.
///
/// Sent by leader when follower is too far behind (log compacted).
/// Transfers snapshot in chunks for large snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSnapshotRequest {
    /// Leader's term
    pub term: Term,

    /// Leader's ID
    pub leader_id: NodeId,

    /// Index of last entry in snapshot
    pub last_included_index: LogIndex,

    /// Term of last_included_index
    pub last_included_term: Term,

    /// Byte offset of chunk in snapshot
    pub offset: u64,

    /// Snapshot chunk data
    pub data: Bytes,

    /// True if this is the last chunk
    pub done: bool,
}

/// InstallSnapshot RPC response.
///
/// Sent by follower to acknowledge chunk receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallSnapshotResponse {
    /// Current term, for leader to update itself
    pub term: Term,

    /// Bytes successfully processed so far
    pub bytes_stored: u64,
}

/// ReadIndex RPC request (linearizable reads).
///
/// Sent by leader to quorum to ensure it's still leader.
/// Prevents stale reads during network partition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadIndexRequest {
    /// Leader's term
    pub term: Term,

    /// Unique read ID (for tracking)
    pub read_id: u64,

    /// Leader's commit index at request time
    pub commit_index: LogIndex,
}

/// ReadIndex RPC response.
///
/// Sent by follower to acknowledge leader is still leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadIndexResponse {
    /// Current term, for leader to update itself
    pub term: Term,

    /// True if follower acknowledges leader
    pub ack: bool,

    /// Read ID (matches request)
    pub read_id: u64,
}

/// Configuration entry (for membership changes).
///
/// Raft uses joint consensus for safe membership changes.
/// A configuration entry is replicated like a normal command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConfigEntry {
    /// Single configuration (normal case)
    Single(Vec<NodeId>),

    /// Joint configuration (during transition)
    /// Requires majority in both C_old AND C_new for quorum
    Joint {
        old: Vec<NodeId>,
        new: Vec<NodeId>,
    },
}

impl ConfigEntry {
    /// Get all nodes in configuration (union for joint config)
    pub fn all_nodes(&self) -> Vec<NodeId> {
        match self {
            ConfigEntry::Single(nodes) => nodes.clone(),
            ConfigEntry::Joint { old, new } => {
                let mut all = old.clone();
                for node in new {
                    if !all.contains(node) {
                        all.push(node.clone());
                    }
                }
                all
            }
        }
    }

    /// Check if we have quorum of nodes
    pub fn has_quorum(&self, votes: &[NodeId]) -> bool {
        match self {
            ConfigEntry::Single(nodes) => {
                let quorum = nodes.len() / 2 + 1;
                let vote_count = nodes.iter().filter(|n| votes.contains(n)).count();
                vote_count >= quorum
            }
            ConfigEntry::Joint { old, new } => {
                let old_quorum = old.len() / 2 + 1;
                let new_quorum = new.len() / 2 + 1;
                let old_votes = old.iter().filter(|n| votes.contains(n)).count();
                let new_votes = new.iter().filter(|n| votes.contains(n)).count();
                old_votes >= old_quorum && new_votes >= new_quorum
            }
        }
    }
}

/// Raft role (Follower, Candidate, or Leader).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    Follower,
    Candidate,
    Leader,
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Follower => write!(f, "Follower"),
            Role::Candidate => write!(f, "Candidate"),
            Role::Leader => write!(f, "Leader"),
        }
    }
}

/// Snapshot metadata (describes snapshot contents).
///
/// Snapshots capture state machine state at a point in time.
/// Metadata tracks which log entries are included.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    /// Last log index included in snapshot
    pub last_included_index: LogIndex,

    /// Term of last_included_index
    pub last_included_term: Term,

    /// Configuration at snapshot time
    pub config: ConfigEntry,

    /// Snapshot data size (bytes)
    pub size_bytes: u64,

    /// Creation timestamp (Unix milliseconds)
    pub created_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_ordering() {
        assert!(Term(2) > Term(1));
        assert_eq!(Term(5).next(), Term(6));
    }

    #[test]
    fn test_log_index_ordering() {
        assert!(LogIndex(10) > LogIndex(5));
        assert_eq!(LogIndex(5).next(), LogIndex(6));
        assert_eq!(LogIndex(5).prev(), Some(LogIndex(4)));
        assert_eq!(LogIndex(0).prev(), None);
    }

    #[test]
    fn test_config_quorum_single() {
        let config = ConfigEntry::Single(vec![
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
        ]);

        // Need 2 of 3 for quorum
        assert!(config.has_quorum(&[NodeId::new("n1"), NodeId::new("n2")]));
        assert!(!config.has_quorum(&[NodeId::new("n1")]));
    }

    #[test]
    fn test_config_quorum_joint() {
        let config = ConfigEntry::Joint {
            old: vec![NodeId::new("n1"), NodeId::new("n2"), NodeId::new("n3")],
            new: vec![NodeId::new("n3"), NodeId::new("n4"), NodeId::new("n5")],
        };

        // Need majority in both old AND new
        // Old: 2 of 3, New: 2 of 3
        assert!(config.has_quorum(&[
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
            NodeId::new("n4")
        ]));

        // Only majority in old, not new
        assert!(!config.has_quorum(&[NodeId::new("n1"), NodeId::new("n2")]));

        // Only majority in new, not old
        assert!(!config.has_quorum(&[NodeId::new("n4"), NodeId::new("n5")]));
    }
}
