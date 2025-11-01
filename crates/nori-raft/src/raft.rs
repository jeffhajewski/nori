//! Main Raft consensus module - wires everything together.
//!
//! The Raft struct implements the ReplicatedLog trait and manages:
//! - RaftState (core state machine)
//! - Background tasks (election loop, heartbeat loop, apply loop)
//! - Lifecycle (start/shutdown)

use crate::config::RaftConfig;
use crate::election::election_loop;
use crate::error::{RaftError, Result};
use crate::lease;
use crate::log::RaftLog;
use crate::read_index;
use crate::replication::{apply_loop, heartbeat_loop};
use crate::rpc_handler::rpc_handler_loop;
use crate::snapshot::{Snapshot, StateMachine};
use crate::state::RaftState;
use crate::timer::ElectionTimer;
use crate::transport::{RaftTransport, RpcReceiver};
use crate::types::*;
use crate::ReplicatedLog;
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

/// Main Raft consensus module.
///
/// Wraps RaftState and manages background tasks for election, heartbeat, and apply.
pub struct Raft {
    /// Core state machine
    state: Arc<RaftState>,

    /// Configuration
    config: RaftConfig,

    /// Transport for RPC
    transport: Arc<dyn RaftTransport>,

    /// Election timer
    election_timer: Arc<ElectionTimer>,

    /// Shutdown signal
    shutdown_tx: broadcast::Sender<()>,

    /// Apply channel sender (for creating new subscriptions)
    applied_tx: Arc<Mutex<mpsc::Sender<(LogIndex, Bytes)>>>,

    /// Optional state machine (e.g., LSM engine)
    /// When provided, committed entries are automatically applied to it
    state_machine: Option<Arc<Mutex<dyn StateMachine>>>,

    /// Optional RPC receiver (for handling incoming RPCs)
    /// When provided, spawns RPC handler loop to process incoming messages
    rpc_rx: Arc<Mutex<Option<RpcReceiver>>>,
}

impl Raft {
    /// Create a new Raft instance.
    ///
    /// Arguments:
    /// - `node_id`: This node's ID
    /// - `config`: Raft configuration
    /// - `log`: Log storage
    /// - `transport`: RPC transport
    /// - `initial_config`: Initial cluster membership
    /// - `state_machine`: Optional state machine (e.g., LSM engine)
    /// - `rpc_rx`: Optional RPC receiver (for multi-node communication)
    pub fn new(
        node_id: NodeId,
        config: RaftConfig,
        log: RaftLog,
        transport: Arc<dyn RaftTransport>,
        initial_config: ConfigEntry,
        state_machine: Option<Arc<Mutex<dyn StateMachine>>>,
        rpc_rx: Option<RpcReceiver>,
    ) -> Self {
        let state = Arc::new(RaftState::new(
            node_id,
            config.clone(),
            log,
            transport.clone(),
            initial_config,
        ));

        let election_timer = Arc::new(ElectionTimer::new(config.clone()));
        let (shutdown_tx, _) = broadcast::channel(16);
        let (applied_tx, _applied_rx) = mpsc::channel(1024);

        Self {
            state,
            config,
            transport,
            election_timer,
            shutdown_tx,
            applied_tx: Arc::new(Mutex::new(applied_tx)),
            state_machine,
            rpc_rx: Arc::new(Mutex::new(rpc_rx)),
        }
    }

    /// Start the Raft instance.
    ///
    /// Spawns background tasks:
    /// - Election timer
    /// - Election loop
    /// - Heartbeat loop (when leader)
    /// - Apply loop
    /// - RPC handler loop (if RPC receiver provided)
    pub async fn start(&self) -> Result<()> {
        // Start election timer
        let timer_clone = self.election_timer.clone();
        tokio::spawn(async move {
            timer_clone.run().await;
        });

        // Start election loop
        let state_clone = self.state.clone();
        let config_clone = self.config.clone();
        let transport_clone = self.transport.clone();
        let timeout_rx = self.election_timer.subscribe();
        let shutdown_rx1 = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            election_loop(
                state_clone,
                config_clone,
                transport_clone,
                timeout_rx,
                shutdown_rx1,
            )
            .await;
        });

        // Start heartbeat loop (runs for all nodes, but only sends when leader)
        let state_clone = self.state.clone();
        let config_clone = self.config.clone();
        let transport_clone = self.transport.clone();
        let shutdown_rx2 = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            heartbeat_loop(state_clone, config_clone, transport_clone, shutdown_rx2).await;
        });

        // Start apply loop
        let state_clone = self.state.clone();
        let applied_tx = self.applied_tx.lock().await.clone();
        let shutdown_rx3 = self.shutdown_tx.subscribe();
        let state_machine_clone = self.state_machine.clone();

        tokio::spawn(async move {
            apply_loop(state_clone, applied_tx, shutdown_rx3, state_machine_clone).await;
        });

        // Start RPC handler loop (if receiver provided)
        let rpc_rx_opt = self.rpc_rx.lock().await.take();
        if let Some(rpc_rx) = rpc_rx_opt {
            let state_clone = self.state.clone();
            let election_timer_clone = self.election_timer.clone();
            let shutdown_rx4 = self.shutdown_tx.subscribe();

            tokio::spawn(async move {
                rpc_handler_loop(state_clone, rpc_rx, election_timer_clone, shutdown_rx4).await;
            });
        }

        Ok(())
    }

    /// Shutdown the Raft instance.
    ///
    /// Sends shutdown signal to all background tasks.
    pub fn shutdown(&self) -> Result<()> {
        self.election_timer.shutdown();
        let _ = self.shutdown_tx.send(());
        Ok(())
    }

    /// Create a snapshot of the current state machine.
    ///
    /// Returns a Snapshot containing:
    /// - Metadata (last_included_index, last_included_term, config)
    /// - Serialized state machine data
    ///
    /// This method should be called periodically to bound log growth.
    pub async fn create_snapshot(&self) -> Result<Snapshot> {
        // Require state machine to be present
        let sm = self.state_machine.as_ref().ok_or_else(|| RaftError::Internal {
            reason: "Cannot create snapshot without state machine".to_string(),
        })?;

        // Get current state
        let (last_applied, config) = {
            let volatile = self.state.volatile_state().read();
            (volatile.last_applied, volatile.config.clone())
        };

        // Get term of last applied entry
        let last_term = if last_applied == LogIndex::ZERO {
            Term::ZERO
        } else {
            self.state
                .log_ref()
                .get(last_applied)
                .await?
                .map(|e| e.term)
                .unwrap_or(Term::ZERO)
        };

        // Create snapshot from state machine
        let sm_lock = sm.lock().await;
        let data = sm_lock.snapshot()?;
        drop(sm_lock);

        Ok(Snapshot::new(last_applied, last_term, config, data))
    }
}

#[async_trait::async_trait]
impl ReplicatedLog for Raft {
    /// Propose a new command to be replicated.
    ///
    /// If this node is the leader, appends the command to the log and replicates it.
    /// Returns the log index where the command was stored.
    async fn propose(&self, cmd: Bytes) -> Result<LogIndex> {
        // Check if we're leader
        if self.state.role() != Role::Leader {
            return Err(RaftError::NotLeader {
                leader: self.state.leader(),
            });
        }

        // Get current term
        let term = self.state.current_term();

        // Append to local log
        let last_index = self.state.log_ref().last_index().await;
        let next_index = last_index.next();

        let entry = LogEntry::new(term, next_index, cmd);
        let index = entry.index;
        self.state.log_ref().append(entry).await?;

        // Reset election timer (we're actively leading)
        self.election_timer.reset();

        // Replication happens in background via heartbeat loop
        // Commitment happens in background via advance_commit_index

        Ok(index)
    }

    /// Perform a linearizable read.
    ///
    /// Uses leader leases for fast reads (no network) when lease is valid.
    /// Falls back to read-index protocol (quorum check) when lease expired.
    async fn read_index(&self) -> Result<()> {
        // Check if we're leader
        if self.state.role() != Role::Leader {
            return Err(RaftError::NotLeader {
                leader: self.state.leader(),
            });
        }

        // Try lease-based fast read first
        let lease_valid = {
            let volatile = self.state.volatile_state().read();
            if let Some(leader_state) = &volatile.leader_state {
                lease::can_read_with_lease(volatile.role, Some(&leader_state.lease))
            } else {
                false
            }
        };

        if lease_valid {
            // Fast path: lease is valid, serve read immediately
            Ok(())
        } else {
            // Slow path: lease expired or unavailable, use read-index protocol
            read_index::read_index(
                self.state.clone(),
                &self.config,
                self.transport.clone(),
            )
            .await
        }
    }

    /// Check if this node is the leader.
    fn is_leader(&self) -> bool {
        self.state.role() == Role::Leader
    }

    /// Get the current leader (if known).
    fn leader(&self) -> Option<NodeId> {
        self.state.leader()
    }

    /// Install a snapshot from the leader.
    ///
    /// Reads snapshot from the reader and restores the state machine.
    async fn install_snapshot(&self, mut snap: Box<dyn std::io::Read + Send>) -> Result<()> {
        // Read snapshot from stream
        let snapshot = Snapshot::read_from(&mut snap)?;

        // Restore state machine if provided
        if let Some(ref sm) = self.state_machine {
            let mut sm_lock = sm.lock().await;
            sm_lock.restore(&snapshot.data)?;
        } else {
            return Err(RaftError::Internal {
                reason: "Cannot install snapshot without state machine".to_string(),
            });
        }

        // Update volatile state
        {
            let mut volatile = self.state.volatile_state().write();
            volatile.last_applied = snapshot.metadata.last_included_index;
            volatile.commit_index = snapshot.metadata.last_included_index;
            volatile.config = snapshot.metadata.config.clone();
        }

        // Truncate log (entries before snapshot are no longer needed)
        self.state
            .log_ref()
            .truncate(snapshot.metadata.last_included_index.next())
            .await?;

        Ok(())
    }

    /// Subscribe to applied log entries.
    ///
    /// Returns a channel that receives (log_index, command) pairs as they are applied.
    fn subscribe_applied(&self) -> mpsc::Receiver<(LogIndex, Bytes)> {
        // Create a new receiver from the apply loop
        // Note: This is a simplified implementation. In production, you'd want
        // a broadcast-like mechanism to support multiple subscribers.
        let (tx, rx) = mpsc::channel(1024);

        // Replace the sender in applied_tx so new applies go to this receiver
        // This is not ideal for multiple subscribers, but works for the basic case
        tokio::spawn({
            let applied_tx = self.applied_tx.clone();
            async move {
                let mut lock = applied_tx.lock().await;
                *lock = tx;
            }
        });

        rx
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;
    use std::collections::HashMap;
    use tempfile::TempDir;

    async fn create_test_raft() -> (Raft, TempDir) {
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

        let raft = Raft::new(NodeId::new("n1"), config, log, transport, initial_config, None, None);
        (raft, temp_dir)
    }

    #[tokio::test]
    async fn test_raft_new() {
        let (raft, _temp) = create_test_raft().await;
        assert!(!raft.is_leader());
        assert_eq!(raft.leader(), None);
    }

    #[tokio::test]
    async fn test_raft_propose_not_leader() {
        let (raft, _temp) = create_test_raft().await;

        let cmd = Bytes::from("test");
        let result = raft.propose(cmd).await;

        assert!(matches!(result, Err(RaftError::NotLeader { .. })));
    }

    #[tokio::test]
    async fn test_raft_start_shutdown() {
        let (raft, _temp) = create_test_raft().await;

        // Start Raft
        raft.start().await.unwrap();

        // Give it a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Shutdown
        raft.shutdown().unwrap();
    }
}
