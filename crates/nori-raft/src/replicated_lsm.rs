//! ReplicatedLSM - Fault-tolerant LSM engine with Raft consensus.
//!
//! Provides a high-level API for a replicated key-value store that combines:
//! - nori-lsm for storage (WAL, SSTable, compaction)
//! - nori-raft for consensus (leader election, log replication, read-index)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │  replicated_put(key, value)             │
//! │    ↓                                    │
//! │  serialize Command::Put                  │
//! │    ↓                                    │
//! │  raft.propose(command)                  │
//! │    ↓                                    │
//! │  [Raft log replication & commit]        │
//! │    ↓                                    │
//! │  apply_loop calls StateMachine.apply()  │
//! │    ↓                                    │
//! │  lsm.put(key, value)                    │
//! └─────────────────────────────────────────┘
//!
//! ┌─────────────────────────────────────────┐
//! │  replicated_get(key)                    │
//! │    ↓                                    │
//! │  raft.read_index() [linearizable]       │
//! │    ↓                                    │
//! │  lsm.get(key)                           │
//! └─────────────────────────────────────────┘
//! ```

use crate::config::RaftConfig;
use crate::error::{RaftError, Result};
use crate::lsm_adapter::LsmStateMachineAdapter;
use crate::log::RaftLog;
use crate::raft::Raft;
use crate::transport::RaftTransport;
use crate::types::*;
use crate::ReplicatedLog;
use bytes::Bytes;
use nori_lsm::raft_sm::{Command, LsmStateMachine};
use nori_lsm::{ATLLConfig, LsmEngine};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Replicated LSM engine with Raft consensus.
///
/// Provides a fault-tolerant, linearizable key-value store by combining:
/// - LSM tree for efficient storage
/// - Raft for consensus and replication
///
/// # Example
///
/// ```no_run
/// use nori_raft::replicated_lsm::ReplicatedLSM;
/// use nori_raft::{RaftConfig, NodeId, ConfigEntry};
/// use nori_lsm::ATLLConfig;
/// use bytes::Bytes;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Configure Raft
/// let node_id = NodeId::new("node1");
/// let raft_config = RaftConfig::default();
/// let lsm_config = ATLLConfig::default();
/// let initial_cluster = ConfigEntry::Single(vec![
///     NodeId::new("node1"),
///     NodeId::new("node2"),
///     NodeId::new("node3"),
/// ]);
///
/// // Create replicated LSM (detailed setup omitted)
/// // let replicated_lsm = ReplicatedLSM::new(...).await?;
///
/// // Write (goes through Raft consensus)
/// // replicated_lsm.replicated_put(Bytes::from("key"), Bytes::from("value"), None).await?;
///
/// // Read (linearizable via read-index)
/// // let value = replicated_lsm.replicated_get(b"key").await?;
/// # Ok(())
/// # }
/// ```
pub struct ReplicatedLSM {
    /// Raft consensus module
    raft: Arc<Raft>,

    /// Underlying LSM engine (wrapped for access)
    lsm_engine: Arc<LsmEngine>,
}

impl ReplicatedLSM {
    /// Create a new ReplicatedLSM instance.
    ///
    /// # Arguments
    /// - `node_id`: This node's ID in the Raft cluster
    /// - `raft_config`: Raft configuration (timeouts, etc.)
    /// - `lsm_config`: LSM engine configuration (data dir, compaction, etc.)
    /// - `raft_log`: Raft log storage
    /// - `transport`: RPC transport for Raft
    /// - `initial_config`: Initial cluster membership
    ///
    /// # Steps
    /// 1. Opens LSM engine
    /// 2. Wraps LSM in state machine adapter
    /// 3. Creates Raft instance with LSM as state machine
    /// 4. Returns ready-to-use ReplicatedLSM
    pub async fn new(
        node_id: NodeId,
        raft_config: RaftConfig,
        lsm_config: ATLLConfig,
        raft_log: RaftLog,
        transport: Arc<dyn RaftTransport>,
        initial_config: ConfigEntry,
        rpc_rx: Option<tokio::sync::mpsc::Receiver<crate::transport::RpcMessage>>,
    ) -> Result<Self> {
        // Open LSM engine on blocking thread pool (uses sync I/O internally)
        let lsm_engine = Arc::new(
            tokio::task::spawn_blocking(move || {
                futures::executor::block_on(LsmEngine::open(lsm_config))
            })
            .await
            .map_err(|e| RaftError::Internal {
                reason: format!("Task join error: {}", e),
            })?
            .map_err(|e| RaftError::Internal {
                reason: format!("Failed to open LSM engine: {}", e),
            })?,
        );

        // Wrap LSM in state machine adapter
        let lsm_sm = LsmStateMachine::new(lsm_engine.clone());
        let adapter = LsmStateMachineAdapter::new(lsm_sm);

        // Create Raft with LSM as state machine
        let raft = Arc::new(Raft::new(
            node_id,
            raft_config,
            raft_log,
            transport,
            initial_config,
            Some(Arc::new(Mutex::new(adapter))),
            rpc_rx,
        ));

        Ok(Self { raft, lsm_engine })
    }

    /// Start the replicated LSM.
    ///
    /// Starts Raft background tasks:
    /// - Election timer
    /// - Election loop
    /// - Heartbeat loop (for leaders)
    /// - Apply loop (applies committed entries to LSM)
    pub async fn start(&self) -> Result<()> {
        self.raft.start().await
    }

    /// Shutdown the replicated LSM.
    ///
    /// Gracefully shuts down Raft and LSM:
    /// - Stops Raft background tasks
    /// - Flushes LSM memtable
    /// - Syncs WAL
    pub async fn shutdown(&self) -> Result<()> {
        self.raft.shutdown()?;
        self.lsm_engine
            .shutdown()
            .await
            .map_err(|e| RaftError::Internal {
                reason: format!("LSM shutdown failed: {}", e),
            })
    }

    /// Replicated put operation.
    ///
    /// Writes a key-value pair through Raft consensus:
    /// 1. Serializes Put command
    /// 2. Proposes to Raft (appends to log, replicates)
    /// 3. Returns once committed (majority acknowledged)
    /// 4. Entry is applied to LSM via apply loop
    ///
    /// # Errors
    /// - `NotLeader`: If this node is not the current leader
    /// - `Internal`: If command serialization or proposal fails
    pub async fn replicated_put(
        &self,
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    ) -> Result<LogIndex> {
        // Serialize command
        let cmd = Command::Put {
            key,
            value,
            ttl,
        };

        let serialized = cmd.serialize().map_err(|e| RaftError::Internal {
            reason: format!("Failed to serialize command: {}", e),
        })?;

        // Propose through Raft
        self.raft.propose(serialized).await
    }

    /// Replicated delete operation.
    ///
    /// Deletes a key through Raft consensus:
    /// 1. Serializes Delete command
    /// 2. Proposes to Raft (appends to log, replicates)
    /// 3. Returns once committed (majority acknowledged)
    /// 4. Entry is applied to LSM via apply loop
    ///
    /// # Errors
    /// - `NotLeader`: If this node is not the current leader
    /// - `Internal`: If command serialization or proposal fails
    pub async fn replicated_delete(&self, key: Bytes) -> Result<LogIndex> {
        // Serialize command
        let cmd = Command::Delete { key };

        let serialized = cmd.serialize().map_err(|e| RaftError::Internal {
            reason: format!("Failed to serialize command: {}", e),
        })?;

        // Propose through Raft
        self.raft.propose(serialized).await
    }

    /// Replicated get operation (linearizable read).
    ///
    /// Reads a key with linearizability guarantee:
    /// 1. Calls raft.read_index() to ensure we're still leader
    /// 2. Reads from local LSM engine
    /// 3. Returns value if found
    ///
    /// Uses leader leases for fast reads when available,
    /// falls back to read-index protocol (quorum check) otherwise.
    ///
    /// # Errors
    /// - `NotLeader`: If this node is not the current leader
    /// - `Internal`: If LSM read fails
    /// Gets a value by key with linearizable read semantics.
    ///
    /// # Returns
    /// - `Ok(Some((value, term, index)))` if key exists
    /// - `Ok(None)` if key does not exist or is deleted
    /// - `Err` on errors
    pub async fn replicated_get(&self, key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>> {
        // Ensure linearizability via read-index
        self.raft.read_index().await?;

        // Read from local LSM
        let result = self.lsm_engine
            .get(key)
            .await
            .map_err(|e| RaftError::Internal {
                reason: format!("LSM get failed: {}", e),
            })?;

        // Convert Version to (Term, LogIndex)
        Ok(result.map(|(value, version)| {
            (value, Term(version.term), LogIndex(version.index))
        }))
    }

    /// Check if this node is the leader.
    pub fn is_leader(&self) -> bool {
        self.raft.is_leader()
    }

    /// Get the current leader (if known).
    pub fn leader(&self) -> Option<NodeId> {
        self.raft.leader()
    }

    /// Get reference to underlying Raft instance.
    pub fn raft(&self) -> &Arc<Raft> {
        &self.raft
    }

    /// Get reference to underlying LSM engine.
    pub fn lsm(&self) -> &Arc<LsmEngine> {
        &self.lsm_engine
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::InMemoryTransport;
    use std::collections::HashMap;
    use tempfile::TempDir;

    /// Helper to extract just the value from replicated_get() result, ignoring version
    fn get_value(result: Option<(Bytes, Term, LogIndex)>) -> Option<Bytes> {
        result.map(|(v, _, _)| v)
    }

    async fn create_test_replicated_lsm(
        node_id: &str,
    ) -> Result<(ReplicatedLSM, TempDir, TempDir)> {
        // Create temp dirs for Raft log and LSM data
        let raft_dir = TempDir::new().unwrap();
        let lsm_dir = TempDir::new().unwrap();

        // Configure Raft
        let mut raft_config = RaftConfig::default();
        raft_config.election_timeout_min = Duration::from_millis(150);
        raft_config.election_timeout_max = Duration::from_millis(300);

        // Configure LSM
        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = lsm_dir.path().to_path_buf();

        // Create Raft log
        let (raft_log, _) = RaftLog::open(raft_dir.path()).await.unwrap();

        // Create transport
        let transport: Arc<dyn RaftTransport> = Arc::new(InMemoryTransport::new(
            NodeId::new(node_id),
            HashMap::new(),
        ));

        // Create single-node cluster for testing
        let initial_config = ConfigEntry::Single(vec![NodeId::new(node_id)]);

        // Create replicated LSM
        let replicated_lsm = ReplicatedLSM::new(
            NodeId::new(node_id),
            raft_config,
            lsm_config,
            raft_log,
            transport,
            initial_config,
            None, // No RPC receiver for single-node tests
        )
        .await?;

        Ok((replicated_lsm, raft_dir, lsm_dir))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_replicated_lsm_creation() {
        let (replicated_lsm, _raft_dir, _lsm_dir) = create_test_replicated_lsm("n1")
            .await
            .unwrap();

        // Verify initial state
        assert!(!replicated_lsm.is_leader());
        assert_eq!(replicated_lsm.leader(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_replicated_lsm_lifecycle() {
        let (replicated_lsm, _raft_dir, _lsm_dir) = create_test_replicated_lsm("n1")
            .await
            .unwrap();

        // Start
        replicated_lsm.start().await.unwrap();

        // Wait for leader election (single node should elect itself)
        tokio::time::sleep(Duration::from_millis(500)).await;

        assert!(replicated_lsm.is_leader());

        // Shutdown
        replicated_lsm.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_replicated_put_get() {
        let (replicated_lsm, _raft_dir, _lsm_dir) = create_test_replicated_lsm("n1")
            .await
            .unwrap();

        replicated_lsm.start().await.unwrap();

        // Wait for leader election
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Put via Raft
        let key = Bytes::from("test_key");
        let value = Bytes::from("test_value");

        let index = replicated_lsm
            .replicated_put(key.clone(), value.clone(), None)
            .await
            .unwrap();

        assert!(index.0 > 0);

        // Wait for apply (apply loop runs every 10ms, so give it some time)
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Get (linearizable read)
        let result = replicated_lsm.replicated_get(b"test_key").await.unwrap();
        assert_eq!(get_value(result), Some(Bytes::from("test_value")));

        replicated_lsm.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_replicated_delete() {
        let (replicated_lsm, _raft_dir, _lsm_dir) = create_test_replicated_lsm("n1")
            .await
            .unwrap();

        replicated_lsm.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Put then delete
        let key = Bytes::from("delete_me");
        replicated_lsm
            .replicated_put(key.clone(), Bytes::from("value"), None)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Delete via Raft
        replicated_lsm
            .replicated_delete(key.clone())
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify deleted
        let result = replicated_lsm.replicated_get(b"delete_me").await.unwrap();
        assert_eq!(result, None);

        replicated_lsm.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_not_leader_error() {
        let (replicated_lsm, _raft_dir, _lsm_dir) = create_test_replicated_lsm("n1")
            .await
            .unwrap();

        // Don't start - node will not be leader

        // Try to put (should fail)
        let result = replicated_lsm
            .replicated_put(Bytes::from("key"), Bytes::from("value"), None)
            .await;

        assert!(matches!(result, Err(RaftError::NotLeader { .. })));
    }
}
