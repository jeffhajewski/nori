//! Raft state machine (Follower, Candidate, Leader roles and transitions).
//!
//! The state machine handles:
//! - Role transitions (Follower → Candidate → Leader → Follower)
//! - RPC handling (RequestVote, AppendEntries, etc.)
//! - Election timeouts and heartbeats
//! - Log replication and commitment
//!
//! # Persistent State (survives crashes)
//!
//! - `current_term`: Latest term server has seen
//! - `voted_for`: Candidate that received vote in current term (None if haven't voted)
//! - `log`: Log entries (stored in RaftLog)
//!
//! # Volatile State (all servers)
//!
//! - `commit_index`: Index of highest log entry known to be committed
//! - `last_applied`: Index of highest log entry applied to state machine
//!
//! # Volatile State (leaders only)
//!
//! - `next_index[]`: For each follower, index of next log entry to send
//! - `match_index[]`: For each follower, index of highest log entry known to be replicated

use crate::config::RaftConfig;
use crate::error::Result;
use crate::lease::LeaseState;
use crate::log::RaftLog;
use crate::snapshot::Snapshot;
use crate::transport::RaftTransport;
use crate::types::*;
use bytes::Bytes;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Pending snapshot being received in chunks.
///
/// Used by followers to buffer incoming snapshot chunks from the leader.
/// When all chunks are received (`done == true`), the snapshot can be installed.
#[derive(Debug)]
pub struct PendingSnapshot {
    /// Last included index in the snapshot
    pub last_included_index: LogIndex,
    /// Last included term in the snapshot
    pub last_included_term: Term,
    /// Buffer for snapshot data
    pub data: Vec<u8>,
    /// Total bytes received so far
    pub bytes_received: u64,
}

impl PendingSnapshot {
    /// Create a new pending snapshot.
    pub fn new(last_included_index: LogIndex, last_included_term: Term) -> Self {
        Self {
            last_included_index,
            last_included_term,
            data: Vec::new(),
            bytes_received: 0,
        }
    }

    /// Append chunk data at the given offset.
    ///
    /// Returns the new bytes_received count.
    pub fn append_chunk(&mut self, offset: u64, data: &[u8]) -> u64 {
        let offset = offset as usize;

        // Extend buffer if needed
        if offset + data.len() > self.data.len() {
            self.data.resize(offset + data.len(), 0);
        }

        // Copy chunk data
        self.data[offset..offset + data.len()].copy_from_slice(data);

        self.bytes_received = (offset + data.len()) as u64;
        self.bytes_received
    }
}

/// Complete snapshot ready for installation.
///
/// Created when all chunks of a snapshot have been received.
#[derive(Debug, Clone)]
pub struct CompletePendingSnapshot {
    /// Last included index in the snapshot
    pub last_included_index: LogIndex,
    /// Last included term in the snapshot
    pub last_included_term: Term,
    /// Complete snapshot data
    pub data: Bytes,
}

/// Raft node state machine.
///
/// Encapsulates all Raft state and logic for a single node.
/// Thread-safe via internal locking.
pub struct RaftState {
    /// This node's ID
    node_id: NodeId,

    /// Raft configuration (timeouts, limits, etc.)
    config: RaftConfig,

    /// Persistent state (term, voted_for, log)
    persistent: Arc<RwLock<PersistentState>>,

    /// Volatile state (role, commit_index, etc.)
    volatile: Arc<RwLock<VolatileState>>,

    /// Log storage
    log: RaftLog,

    /// Transport for RPC communication (stored for potential future use)
    #[allow(dead_code)]
    transport: Arc<dyn RaftTransport>,

    /// Observability meter for metrics and events
    meter: Arc<dyn nori_observe::Meter>,
}

/// Persistent state (must survive crashes).
///
/// Stored on disk (term/voted_for could be in a separate metadata file).
struct PersistentState {
    /// Latest term this server has seen (monotonically increasing)
    current_term: Term,

    /// Candidate that received vote in current term (None if haven't voted)
    voted_for: Option<NodeId>,
}

/// Volatile state (lost on crash, recomputed on recovery).
pub struct VolatileState {
    /// Current role (Follower, Candidate, or Leader)
    pub role: Role,

    /// Current leader (if known)
    /// Followers track this to redirect client requests
    pub leader_id: Option<NodeId>,

    /// Highest log index known to be committed
    pub commit_index: LogIndex,

    /// Highest log index applied to state machine
    pub last_applied: LogIndex,

    /// Highest log index included in the last snapshot
    /// Used to determine when to create a new snapshot
    pub last_snapshot_index: LogIndex,

    /// Cached snapshot for sending to followers
    /// Updated when create_snapshot is called
    pub last_snapshot: Option<Arc<Snapshot>>,

    /// Pending snapshot being received from leader (chunk buffering)
    /// Set when receiving InstallSnapshot with offset == 0
    /// Cleared when snapshot is complete and ready for installation
    pub pending_snapshot: Option<PendingSnapshot>,

    /// Complete pending snapshot ready for installation
    /// Set when all chunks received (done == true)
    /// Consumed by apply_loop to restore state machine
    pub complete_pending_snapshot: Option<CompletePendingSnapshot>,

    /// Leader-specific state (only valid when role == Leader)
    pub leader_state: Option<LeaderState>,

    /// Last time we heard from the leader (for election timeout)
    pub last_heartbeat: Instant,

    /// Current cluster configuration
    pub config: ConfigEntry,
}

/// Leader-specific volatile state.
///
/// Only valid when role == Leader.
/// Tracks replication progress for each follower.
pub struct LeaderState {
    /// For each peer, index of next log entry to send
    /// Initialized to leader's last_index + 1
    pub next_index: HashMap<NodeId, LogIndex>,

    /// For each peer, index of highest log entry known to be replicated
    /// Initialized to 0
    pub match_index: HashMap<NodeId, LogIndex>,

    /// Lease state (for fast linearizable reads)
    /// Leader can serve reads without quorum while lease is valid
    pub lease: LeaseState,
}

impl RaftState {
    /// Create a new Raft state machine.
    ///
    /// Arguments:
    /// - `node_id`: This node's ID
    /// - `config`: Raft configuration
    /// - `log`: Log storage
    /// - `transport`: RPC transport
    /// - `initial_config`: Initial cluster membership
    /// - `meter`: Observability meter for metrics and events
    pub fn new(
        node_id: NodeId,
        config: RaftConfig,
        log: RaftLog,
        transport: Arc<dyn RaftTransport>,
        initial_config: ConfigEntry,
        meter: Arc<dyn nori_observe::Meter>,
    ) -> Self {
        Self {
            node_id,
            config,
            persistent: Arc::new(RwLock::new(PersistentState {
                current_term: Term::ZERO,
                voted_for: None,
            })),
            volatile: Arc::new(RwLock::new(VolatileState {
                role: Role::Follower,
                leader_id: None,
                commit_index: LogIndex::ZERO,
                last_applied: LogIndex::ZERO,
                last_snapshot_index: LogIndex::ZERO,
                last_snapshot: None,
                pending_snapshot: None,
                complete_pending_snapshot: None,
                leader_state: None,
                last_heartbeat: Instant::now(),
                config: initial_config,
            })),
            log,
            transport,
            meter,
        }
    }

    /// Get the current role.
    pub fn role(&self) -> Role {
        self.volatile.read().role
    }

    /// Get the current term.
    pub fn current_term(&self) -> Term {
        self.persistent.read().current_term
    }

    /// Get the current leader (if known).
    pub fn leader(&self) -> Option<NodeId> {
        self.volatile.read().leader_id.clone()
    }

    /// Get the commit index.
    pub fn commit_index(&self) -> LogIndex {
        self.volatile.read().commit_index
    }

    /// Get this node's ID.
    pub fn node_id(&self) -> &NodeId {
        &self.node_id
    }

    /// Get a reference to the log.
    pub fn log_ref(&self) -> &RaftLog {
        &self.log
    }

    /// Get a reference to the volatile state.
    pub fn volatile_state(&self) -> &Arc<RwLock<VolatileState>> {
        &self.volatile
    }

    /// Get a reference to the Raft configuration.
    pub fn config(&self) -> &RaftConfig {
        &self.config
    }

    /// Get the cached snapshot for sending to followers.
    pub fn last_snapshot(&self) -> Option<Arc<Snapshot>> {
        self.volatile.read().last_snapshot.clone()
    }

    /// Set the cached snapshot (called after create_snapshot).
    pub fn set_last_snapshot(&self, snapshot: Arc<Snapshot>) {
        let mut volatile = self.volatile.write();
        volatile.last_snapshot_index = snapshot.metadata.last_included_index;
        volatile.last_snapshot = Some(snapshot);
    }

    /// Take the complete pending snapshot if available.
    ///
    /// Returns the snapshot and clears the pending state.
    /// Called by apply_loop to install received snapshots.
    pub fn take_complete_pending_snapshot(&self) -> Option<CompletePendingSnapshot> {
        let mut volatile = self.volatile.write();
        volatile.complete_pending_snapshot.take()
    }

    /// Set the current term (for testing).
    #[cfg(test)]
    pub fn set_current_term(&self, term: Term) {
        let mut persistent = self.persistent.write();
        persistent.current_term = term;
    }

    /// Handle RequestVote RPC.
    ///
    /// Invoked by candidate to gather votes.
    /// Returns vote granted/denied based on log up-to-dateness.
    pub async fn handle_request_vote(
        &self,
        request: RequestVoteRequest,
    ) -> Result<RequestVoteResponse> {
        // Get log info first (before acquiring locks to avoid holding across await)
        let last_log_term = self.log.last_term().await;
        let last_log_index = self.log.last_index().await;

        let mut persistent = self.persistent.write();
        let mut volatile = self.volatile.write();

        // If request term > current term, update our term and become follower
        if request.term > persistent.current_term {
            self.step_down_inner(&mut persistent, &mut volatile, request.term);
        }

        let mut vote_granted = false;

        // Grant vote if:
        // 1. request.term >= current_term
        // 2. Haven't voted for anyone else in this term
        // 3. Candidate's log is at least as up-to-date as ours
        if request.term >= persistent.current_term {
            let already_voted = persistent
                .voted_for
                .as_ref()
                .is_some_and(|id| id != &request.candidate_id);

            if !already_voted {
                // Check log up-to-dateness
                let log_ok = request.last_log_term > last_log_term
                    || (request.last_log_term == last_log_term
                        && request.last_log_index >= last_log_index);

                if log_ok {
                    vote_granted = true;
                    persistent.voted_for = Some(request.candidate_id.clone());
                    volatile.last_heartbeat = Instant::now(); // Reset election timer
                }
            }
        }

        Ok(RequestVoteResponse {
            term: persistent.current_term,
            vote_granted,
        })
    }

    /// Handle AppendEntries RPC.
    ///
    /// Invoked by leader to:
    /// - Replicate log entries
    /// - Send heartbeats (empty entries)
    pub async fn handle_append_entries(
        &self,
        request: AppendEntriesRequest,
    ) -> Result<AppendEntriesResponse> {
        // Phase 1: Update term if needed and check basic conditions
        let (current_term, should_reject) = {
            let mut persistent = self.persistent.write();
            let mut volatile = self.volatile.write();

            // If request term > current term, update and step down
            if request.term > persistent.current_term {
                self.step_down_inner(&mut persistent, &mut volatile, request.term);
            }

            // Check if we should reject (term too old)
            let should_reject = request.term < persistent.current_term;

            // Valid AppendEntries from current leader - reset election timer
            if !should_reject {
                volatile.last_heartbeat = Instant::now();
                volatile.leader_id = Some(request.leader_id.clone());

                // If we're a candidate or leader, step down to follower
                // (This prevents split-brain when two leaders exist in the same term)
                if volatile.role == Role::Candidate || volatile.role == Role::Leader {
                    if volatile.role == Role::Leader {
                        tracing::warn!(
                            term = %persistent.current_term,
                            "Leader stepping down after receiving AppendEntries from {}",
                            request.leader_id
                        );
                    }
                    volatile.role = Role::Follower;
                    volatile.leader_state = None;
                }
            }

            (persistent.current_term, should_reject)
        };

        if should_reject {
            let last_log_index = self.log.last_index().await;
            return Ok(AppendEntriesResponse {
                term: current_term,
                success: false,
                conflict_index: None,
                last_log_index,
            });
        }

        // Phase 2: Check log consistency (no locks held)
        let log_ok = if request.prev_log_index == LogIndex::ZERO {
            // Empty log - always ok
            true
        } else {
            // Check if we have entry at prev_log_index with matching term
            if let Some(entry) = self.log.get(request.prev_log_index).await? {
                entry.term == request.prev_log_term
            } else {
                false
            }
        };

        if !log_ok {
            // Log inconsistency - send conflict hint for fast backtracking
            let conflict_index = request.prev_log_index.prev();
            let last_log_index = self.log.last_index().await;
            return Ok(AppendEntriesResponse {
                term: current_term,
                success: false,
                conflict_index,
                last_log_index,
            });
        }

        // Phase 3: Append entries (no locks held)
        if !request.entries.is_empty() {
            // Truncate log from first conflicting entry forward
            let first_new_index = request.prev_log_index.next();
            self.log.truncate(first_new_index).await?;

            // Append new entries
            self.log.append_batch(request.entries).await?;
        }

        // Phase 4: Update commit index
        let last_new_index = self.log.last_index().await;
        {
            let mut volatile = self.volatile.write();
            if request.leader_commit > volatile.commit_index {
                volatile.commit_index = std::cmp::min(request.leader_commit, last_new_index);
            }
        }

        Ok(AppendEntriesResponse {
            term: current_term,
            success: true,
            conflict_index: None,
            last_log_index: last_new_index,
        })
    }

    /// Handle InstallSnapshot RPC.
    ///
    /// Invoked by leader when follower is too far behind (log compacted).
    /// Receives snapshot in chunks and buffers until complete.
    /// When all chunks received (done == true), marks snapshot ready for installation.
    pub async fn handle_install_snapshot(
        &self,
        request: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse> {
        let mut persistent = self.persistent.write();
        let mut volatile = self.volatile.write();

        // If request term > current term, update and step down
        if request.term > persistent.current_term {
            self.step_down_inner(&mut persistent, &mut volatile, request.term);
        }

        // Reject if term < current_term
        if request.term < persistent.current_term {
            return Ok(InstallSnapshotResponse {
                term: persistent.current_term,
                bytes_stored: 0,
            });
        }

        // Valid InstallSnapshot from current leader - reset election timer
        volatile.last_heartbeat = Instant::now();
        volatile.leader_id = Some(request.leader_id.clone());

        // If offset == 0, start a new pending snapshot
        if request.offset == 0 {
            tracing::info!(
                leader = ?request.leader_id,
                last_included_index = %request.last_included_index,
                last_included_term = %request.last_included_term,
                "Starting to receive snapshot from leader"
            );
            volatile.pending_snapshot = Some(PendingSnapshot::new(
                request.last_included_index,
                request.last_included_term,
            ));
        }

        // Append chunk to pending snapshot
        let bytes_stored = if let Some(ref mut pending) = volatile.pending_snapshot {
            // Verify snapshot metadata matches
            if pending.last_included_index != request.last_included_index
                || pending.last_included_term != request.last_included_term
            {
                tracing::warn!(
                    expected_index = %pending.last_included_index,
                    expected_term = %pending.last_included_term,
                    got_index = %request.last_included_index,
                    got_term = %request.last_included_term,
                    "Snapshot metadata mismatch, resetting"
                );
                // Reset and start fresh
                volatile.pending_snapshot = Some(PendingSnapshot::new(
                    request.last_included_index,
                    request.last_included_term,
                ));
                let pending = volatile.pending_snapshot.as_mut().unwrap();
                pending.append_chunk(request.offset, &request.data)
            } else {
                pending.append_chunk(request.offset, &request.data)
            }
        } else {
            // No pending snapshot and offset != 0 - stale chunk, ignore
            tracing::warn!(
                offset = request.offset,
                "Received snapshot chunk with no pending snapshot, ignoring"
            );
            return Ok(InstallSnapshotResponse {
                term: persistent.current_term,
                bytes_stored: 0,
            });
        };

        // If done, move to complete pending snapshot
        if request.done {
            if let Some(pending) = volatile.pending_snapshot.take() {
                tracing::info!(
                    last_included_index = %pending.last_included_index,
                    last_included_term = %pending.last_included_term,
                    bytes = pending.bytes_received,
                    "Snapshot fully received, ready for installation"
                );

                volatile.complete_pending_snapshot = Some(CompletePendingSnapshot {
                    last_included_index: pending.last_included_index,
                    last_included_term: pending.last_included_term,
                    data: Bytes::from(pending.data),
                });
            }
        }

        Ok(InstallSnapshotResponse {
            term: persistent.current_term,
            bytes_stored,
        })
    }

    /// Handle ReadIndex RPC.
    ///
    /// Invoked by leader during read-index protocol to confirm leadership.
    /// Follower acknowledges with current term.
    pub async fn handle_read_index(
        &self,
        request: ReadIndexRequest,
    ) -> Result<ReadIndexResponse> {
        let mut persistent = self.persistent.write();
        let mut volatile = self.volatile.write();

        // If request term > current term, update and step down
        if request.term > persistent.current_term {
            self.step_down_inner(&mut persistent, &mut volatile, request.term);
        }

        // Reject if term < current_term
        if request.term < persistent.current_term {
            return Ok(ReadIndexResponse {
                term: persistent.current_term,
                ack: false,
                read_id: request.read_id,
            });
        }

        // Valid ReadIndex from current leader - reset election timer
        volatile.last_heartbeat = Instant::now();

        // Acknowledge leadership
        Ok(ReadIndexResponse {
            term: persistent.current_term,
            ack: true,
            read_id: request.read_id,
        })
    }

    /// Step down to follower role.
    ///
    /// Called when we discover a higher term or when a candidate loses election.
    fn step_down_inner(
        &self,
        persistent: &mut PersistentState,
        volatile: &mut VolatileState,
        new_term: Term,
    ) {
        persistent.current_term = new_term;
        persistent.voted_for = None;
        volatile.role = Role::Follower;
        volatile.leader_state = None;
        volatile.last_heartbeat = Instant::now();
    }

    /// Check if election timeout has elapsed (follower/candidate only).
    ///
    /// Returns true if we should start an election.
    pub fn election_timeout_elapsed(&self) -> bool {
        let volatile = self.volatile.read();
        if volatile.role == Role::Leader {
            return false;
        }

        let elapsed = volatile.last_heartbeat.elapsed();
        let timeout = self.config.random_election_timeout();
        elapsed > timeout
    }

    /// Transition to candidate and start election.
    ///
    /// Returns the new term we're running for.
    pub async fn start_election(&self) -> Result<Term> {
        let mut persistent = self.persistent.write();
        let mut volatile = self.volatile.write();

        // Increment term
        persistent.current_term = persistent.current_term.next();
        let term = persistent.current_term;

        // Vote for self
        persistent.voted_for = Some(self.node_id.clone());

        // Transition to candidate
        volatile.role = Role::Candidate;
        volatile.leader_state = None;
        volatile.last_heartbeat = Instant::now();

        Ok(term)
    }

    /// Transition to leader (after winning election).
    ///
    /// Initializes leader state (next_index[], match_index[]).
    pub async fn become_leader(&self) -> Result<()> {
        // Get last log index before acquiring lock (to avoid holding lock across await)
        let last_log_index = self.log.last_index().await;

        let mut volatile = self.volatile.write();
        volatile.role = Role::Leader;
        volatile.leader_id = Some(self.node_id.clone());

        // Initialize leader state
        let all_nodes = volatile.config.all_nodes();

        let mut next_index = HashMap::new();
        let mut match_index = HashMap::new();

        for node in all_nodes {
            if node != self.node_id {
                next_index.insert(node.clone(), last_log_index.next());
                match_index.insert(node, LogIndex::ZERO);
            }
        }

        volatile.leader_state = Some(LeaderState {
            next_index,
            match_index,
            lease: LeaseState::new(&self.config),
        });

        let term = self.persistent.read().current_term;

        // Drop the volatile write lock before emitting event
        drop(volatile);

        // Emit VizEvent for leader election
        self.meter.emit(nori_observe::VizEvent::Raft(nori_observe::RaftEvt {
            shard: self.config.shard_id,
            term: term.as_u64(),
            kind: nori_observe::RaftKind::LeaderElected {
                node: self.node_id_as_u32(),
            },
        }));

        Ok(())
    }

    /// Helper to convert NodeId to u32 for metrics
    fn node_id_as_u32(&self) -> u32 {
        // Simple hash of node ID for now
        // In production, you'd want a proper node ID → numeric mapping
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.node_id.as_str().hash(&mut hasher);
        hasher.finish() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn create_test_state() -> (RaftState, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let (log, _) = RaftLog::open(temp_dir.path()).await.unwrap();

        let config = RaftConfig::default();
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new("n1"),
            HashMap::new(),
        ));

        let initial_config = ConfigEntry::Single(vec![
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
        ]);

        let meter = Arc::new(nori_observe::NoopMeter::default());
        let state = RaftState::new(NodeId::new("n1"), config, log, transport, initial_config, meter);
        (state, temp_dir)
    }

    #[tokio::test]
    async fn test_raft_state_initial_state() {
        let (state, _temp) = create_test_state().await;

        assert_eq!(state.role(), Role::Follower);
        assert_eq!(state.current_term(), Term::ZERO);
        assert_eq!(state.leader(), None);
    }

    #[tokio::test]
    async fn test_raft_state_handle_request_vote_grants() {
        let (state, _temp) = create_test_state().await;

        let request = RequestVoteRequest {
            term: Term(5),
            candidate_id: NodeId::new("n2"),
            last_log_index: LogIndex::ZERO,
            last_log_term: Term::ZERO,
        };

        let response = state.handle_request_vote(request).await.unwrap();
        assert!(response.vote_granted);
        assert_eq!(response.term, Term(5));
    }

    #[tokio::test]
    async fn test_raft_state_handle_request_vote_rejects_stale_term() {
        let (state, _temp) = create_test_state().await;

        // Update to term 10
        {
            let mut persistent = state.persistent.write();
            persistent.current_term = Term(10);
        }

        let request = RequestVoteRequest {
            term: Term(5), // Stale term
            candidate_id: NodeId::new("n2"),
            last_log_index: LogIndex::ZERO,
            last_log_term: Term::ZERO,
        };

        let response = state.handle_request_vote(request).await.unwrap();
        assert!(!response.vote_granted);
        assert_eq!(response.term, Term(10));
    }

    // ========== Snapshot Transfer Tests ==========

    #[test]
    fn test_pending_snapshot_append_chunk() {
        let mut pending = PendingSnapshot::new(LogIndex(100), Term(5));

        // Append first chunk
        let chunk1 = b"Hello, ";
        let bytes = pending.append_chunk(0, chunk1);
        assert_eq!(bytes, chunk1.len() as u64);
        assert_eq!(pending.bytes_received, chunk1.len() as u64);
        assert_eq!(&pending.data[..], chunk1);

        // Append second chunk
        let chunk2 = b"World!";
        let bytes = pending.append_chunk(chunk1.len() as u64, chunk2);
        assert_eq!(bytes, (chunk1.len() + chunk2.len()) as u64);
        assert_eq!(pending.data, b"Hello, World!");
    }

    #[test]
    fn test_pending_snapshot_non_sequential_chunks() {
        let mut pending = PendingSnapshot::new(LogIndex(100), Term(5));

        // Append second chunk first (simulating out-of-order)
        let chunk2 = b"World!";
        pending.append_chunk(7, chunk2);

        // Then first chunk
        let chunk1 = b"Hello, ";
        pending.append_chunk(0, chunk1);

        // Both chunks should be in place
        assert_eq!(pending.data, b"Hello, World!");
    }

    #[tokio::test]
    async fn test_handle_install_snapshot_single_chunk() {
        let (state, _temp) = create_test_state().await;

        let snapshot_data = b"test snapshot data";
        let request = InstallSnapshotRequest {
            term: Term(1),
            leader_id: NodeId::new("leader"),
            last_included_index: LogIndex(100),
            last_included_term: Term(1),
            offset: 0,
            data: Bytes::from(&snapshot_data[..]),
            done: true,
        };

        let response = state.handle_install_snapshot(request).await.unwrap();
        assert_eq!(response.term, Term(1));
        assert_eq!(response.bytes_stored, snapshot_data.len() as u64);

        // Verify complete snapshot is ready
        let complete = state.take_complete_pending_snapshot();
        assert!(complete.is_some());
        let complete = complete.unwrap();
        assert_eq!(complete.last_included_index, LogIndex(100));
        assert_eq!(complete.last_included_term, Term(1));
        assert_eq!(complete.data.as_ref(), snapshot_data);
    }

    #[tokio::test]
    async fn test_handle_install_snapshot_multi_chunk() {
        let (state, _temp) = create_test_state().await;

        let chunk1 = b"first chunk ";
        let chunk2 = b"second chunk";

        // First chunk
        let request1 = InstallSnapshotRequest {
            term: Term(1),
            leader_id: NodeId::new("leader"),
            last_included_index: LogIndex(100),
            last_included_term: Term(1),
            offset: 0,
            data: Bytes::from(&chunk1[..]),
            done: false,
        };

        let response1 = state.handle_install_snapshot(request1).await.unwrap();
        assert_eq!(response1.bytes_stored, chunk1.len() as u64);

        // Verify snapshot is pending but not complete
        assert!(state.take_complete_pending_snapshot().is_none());

        // Second chunk (final)
        let request2 = InstallSnapshotRequest {
            term: Term(1),
            leader_id: NodeId::new("leader"),
            last_included_index: LogIndex(100),
            last_included_term: Term(1),
            offset: chunk1.len() as u64,
            data: Bytes::from(&chunk2[..]),
            done: true,
        };

        let response2 = state.handle_install_snapshot(request2).await.unwrap();
        assert_eq!(response2.bytes_stored, (chunk1.len() + chunk2.len()) as u64);

        // Verify complete snapshot is ready
        let complete = state.take_complete_pending_snapshot();
        assert!(complete.is_some());
        let complete = complete.unwrap();
        assert_eq!(complete.data.as_ref(), b"first chunk second chunk");
    }

    #[tokio::test]
    async fn test_handle_install_snapshot_rejects_stale_term() {
        let (state, _temp) = create_test_state().await;

        // Set current term higher
        state.set_current_term(Term(5));

        let request = InstallSnapshotRequest {
            term: Term(3), // Stale term
            leader_id: NodeId::new("leader"),
            last_included_index: LogIndex(100),
            last_included_term: Term(3),
            offset: 0,
            data: Bytes::from("data"),
            done: true,
        };

        let response = state.handle_install_snapshot(request).await.unwrap();
        assert_eq!(response.term, Term(5));
        assert_eq!(response.bytes_stored, 0);

        // Verify no snapshot was stored
        assert!(state.take_complete_pending_snapshot().is_none());
    }

    #[tokio::test]
    async fn test_handle_install_snapshot_stale_chunk_ignored() {
        let (state, _temp) = create_test_state().await;

        // Send a chunk with offset != 0 but no pending snapshot
        let request = InstallSnapshotRequest {
            term: Term(1),
            leader_id: NodeId::new("leader"),
            last_included_index: LogIndex(100),
            last_included_term: Term(1),
            offset: 100, // Non-zero offset with no pending snapshot
            data: Bytes::from("orphan chunk"),
            done: false,
        };

        let response = state.handle_install_snapshot(request).await.unwrap();
        assert_eq!(response.bytes_stored, 0); // Should be rejected

        // Verify no pending snapshot was created
        assert!(state.take_complete_pending_snapshot().is_none());
    }
}
