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
use crate::state::RaftState;
use crate::timer::ElectionTimer;
use crate::transport::RaftTransport;
use crate::types::*;
use crate::ReplicatedLog;
use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

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
    applied_tx: Arc<tokio::sync::Mutex<mpsc::Sender<(LogIndex, Bytes)>>>,
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
    pub fn new(
        node_id: NodeId,
        config: RaftConfig,
        log: RaftLog,
        transport: Arc<dyn RaftTransport>,
        initial_config: ConfigEntry,
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
            applied_tx: Arc::new(tokio::sync::Mutex::new(applied_tx)),
        }
    }

    /// Start the Raft instance.
    ///
    /// Spawns background tasks:
    /// - Election timer
    /// - Election loop
    /// - Heartbeat loop (when leader)
    /// - Apply loop
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

        tokio::spawn(async move {
            apply_loop(state_clone, applied_tx, shutdown_rx3).await;
        });

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
    /// Placeholder for now - will be implemented in Priority 4.
    async fn install_snapshot(&self, _snap: Box<dyn std::io::Read + Send>) -> Result<()> {
        // TODO: Implement snapshot installation
        Err(RaftError::Internal {
            reason: "Snapshot installation not yet implemented".to_string(),
        })
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

        let raft = Raft::new(NodeId::new("n1"), config, log, transport, initial_config);
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
