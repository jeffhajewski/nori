//! Shard Manager - Manages per-shard Raft groups and LSM engines.
//!
//! Each shard has its own:
//! - Raft consensus group (leader election, log replication)
//! - LSM storage engine (WAL + memtable + SST)
//! - ReplicatedLSM wrapper (integrates Raft + LSM)
//!
//! The ShardManager provides:
//! - Shard lifecycle management (create, start, stop)
//! - Routing: given a shard_id, return the ReplicatedLSM instance
//! - Health monitoring: track which shards have leaders

use crate::config::ServerConfig;
use nori_lsm::ATLLConfig;
use nori_raft::{log::RaftLog, transport::RpcMessage, ConfigEntry, NodeId, RaftConfig, ReplicatedLSM};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shard identifier (0..total_shards)
pub type ShardId = u32;

/// Error types for ShardManager operations
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum ShardError {
    #[error("Shard {0} not found")]
    NotFound(ShardId),

    #[error("Shard {0} already exists")]
    AlreadyExists(ShardId),

    #[error("Failed to initialize shard {0}: {1}")]
    Initialization(ShardId, String),

    #[error("Failed to start shard {0}: {1}")]
    Startup(ShardId, String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Raft error: {0}")]
    Raft(String),
}

/// Metadata about a shard
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ShardInfo {
    /// Shard ID
    pub shard_id: ShardId,

    /// Whether this shard has a leader
    pub has_leader: bool,

    /// Current leader node (if known)
    pub leader: Option<NodeId>,

    /// Whether this node is the leader for this shard
    pub is_leader: bool,
}

/// Manages multiple shards, each with its own Raft group and LSM engine.
///
/// Architecture:
/// ```text
/// ShardManager
///   ├─ Shard 0 → ReplicatedLSM (Raft + LSM)
///   ├─ Shard 1 → ReplicatedLSM (Raft + LSM)
///   ├─ ...
///   └─ Shard 1023 → ReplicatedLSM (Raft + LSM)
/// ```
pub struct ShardManager {
    /// Server configuration
    config: ServerConfig,

    /// Map of shard_id → ReplicatedLSM
    shards: Arc<RwLock<HashMap<ShardId, Arc<ReplicatedLSM>>>>,

    /// Base Raft configuration (cloned for each shard)
    raft_config: RaftConfig,

    /// Base LSM configuration (cloned for each shard)
    lsm_config: ATLLConfig,

    /// Raft transport (shared across all shards)
    transport: Arc<dyn nori_raft::transport::RaftTransport>,

    /// Initial cluster configuration
    initial_config: ConfigEntry,

    /// Raft RPC channel senders (one per shard, if multi-node)
    /// Map: ShardId → mpsc::Sender<RpcMessage>
    rpc_senders: Arc<RwLock<HashMap<ShardId, tokio::sync::mpsc::Sender<RpcMessage>>>>,
}

impl ShardManager {
    /// Creates a new ShardManager.
    ///
    /// Does NOT create shards yet - call `create_shard()` or `create_all_shards()`.
    pub fn new(
        config: ServerConfig,
        raft_config: RaftConfig,
        lsm_config: ATLLConfig,
        transport: Arc<dyn nori_raft::transport::RaftTransport>,
        initial_config: ConfigEntry,
    ) -> Self {
        Self {
            config,
            shards: Arc::new(RwLock::new(HashMap::new())),
            raft_config,
            lsm_config,
            transport,
            initial_config,
            rpc_senders: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Creates a single shard (lazy initialization).
    ///
    /// Returns the ReplicatedLSM instance for the shard.
    pub async fn create_shard(&self, shard_id: ShardId) -> Result<Arc<ReplicatedLSM>, ShardError> {
        // Check if shard already exists
        {
            let shards = self.shards.read().await;
            if shards.contains_key(&shard_id) {
                return Err(ShardError::AlreadyExists(shard_id));
            }
        }

        tracing::info!("Creating shard {}", shard_id);

        // Create shard-specific directories
        let raft_dir = self.shard_raft_dir(shard_id);
        let lsm_dir = self.shard_lsm_dir(shard_id);

        std::fs::create_dir_all(&raft_dir)?;
        std::fs::create_dir_all(&lsm_dir)?;

        // Open Raft log for this shard
        let (raft_log, _) = RaftLog::open(&raft_dir)
            .await
            .map_err(|e| ShardError::Initialization(shard_id, format!("Failed to open raft log: {}", e)))?;

        // Clone and customize LSM config for this shard
        let mut shard_lsm_config = self.lsm_config.clone();
        shard_lsm_config.data_dir = lsm_dir.clone();

        // Create node ID with shard suffix (e.g., "node1-shard0")
        let node_id = NodeId::new(format!("{}-shard{}", self.config.node_id, shard_id));

        // Create RPC receiver channel for multi-node mode
        let raft_rpc_rx = if !self.config.cluster.seed_nodes.is_empty() {
            let (tx, rx) = tokio::sync::mpsc::channel(1000);
            self.rpc_senders.write().await.insert(shard_id, tx);
            Some(rx)
        } else {
            None
        };

        // Create per-shard initial config with suffixed node IDs
        // This ensures each shard's Raft group has the correct member IDs
        let shard_initial_config = match &self.initial_config {
            ConfigEntry::Single(nodes) => {
                // Map each base node ID to its shard-specific ID
                let shard_nodes: Vec<NodeId> = nodes
                    .iter()
                    .map(|base_id| NodeId::new(format!("{}-shard{}", base_id.as_str(), shard_id)))
                    .collect();
                ConfigEntry::Single(shard_nodes)
            }
            ConfigEntry::Joint { old, new } => {
                // Handle joint consensus (for membership changes)
                let old_shard: Vec<NodeId> = old
                    .iter()
                    .map(|base_id| NodeId::new(format!("{}-shard{}", base_id.as_str(), shard_id)))
                    .collect();
                let new_shard: Vec<NodeId> = new
                    .iter()
                    .map(|base_id| NodeId::new(format!("{}-shard{}", base_id.as_str(), shard_id)))
                    .collect();
                ConfigEntry::Joint { old: old_shard, new: new_shard }
            }
        };

        // Create ReplicatedLSM for this shard
        let replicated_lsm = ReplicatedLSM::new(
            node_id,
            self.raft_config.clone(),
            shard_lsm_config,
            raft_log,
            self.transport.clone(),
            shard_initial_config,
            raft_rpc_rx,
        )
        .await
        .map_err(|e| ShardError::Initialization(shard_id, format!("Failed to create ReplicatedLSM: {:?}", e)))?;

        let replicated_lsm = Arc::new(replicated_lsm);

        // Store in map
        self.shards.write().await.insert(shard_id, replicated_lsm.clone());

        tracing::info!("Shard {} created successfully", shard_id);

        Ok(replicated_lsm)
    }

    /// Creates all shards (0..total_shards).
    ///
    /// This is expensive - creates 1024 Raft groups and LSM engines.
    /// For development, you may want to create shards lazily (on first access).
    #[allow(dead_code)]
    pub async fn create_all_shards(&self) -> Result<(), ShardError> {
        let total_shards = self.config.cluster.total_shards;
        tracing::info!("Creating all {} shards...", total_shards);

        for shard_id in 0..total_shards {
            self.create_shard(shard_id).await?;
        }

        tracing::info!("All shards created successfully");
        Ok(())
    }

    /// Starts a specific shard (starts Raft background tasks).
    pub async fn start_shard(&self, shard_id: ShardId) -> Result<(), ShardError> {
        let shard = self.get_shard(shard_id).await?;

        shard
            .start()
            .await
            .map_err(|e| ShardError::Startup(shard_id, format!("Failed to start: {:?}", e)))?;

        tracing::info!("Shard {} started", shard_id);
        Ok(())
    }

    /// Starts all shards.
    #[allow(dead_code)]
    pub async fn start_all_shards(&self) -> Result<(), ShardError> {
        let shard_ids: Vec<ShardId> = self.shards.read().await.keys().copied().collect();

        tracing::info!("Starting {} shards...", shard_ids.len());

        for shard_id in shard_ids {
            self.start_shard(shard_id).await?;
        }

        tracing::info!("All shards started");
        Ok(())
    }

    /// Gets a shard's ReplicatedLSM instance.
    ///
    /// Returns NotFound if shard doesn't exist (hasn't been created yet).
    pub async fn get_shard(&self, shard_id: ShardId) -> Result<Arc<ReplicatedLSM>, ShardError> {
        let shards = self.shards.read().await;
        shards
            .get(&shard_id)
            .cloned()
            .ok_or(ShardError::NotFound(shard_id))
    }

    /// Gets or creates a shard (lazy initialization).
    ///
    /// If the shard doesn't exist, creates it and starts it.
    /// Useful for sparse shard distribution (only create shards with data).
    pub async fn get_or_create_shard(&self, shard_id: ShardId) -> Result<Arc<ReplicatedLSM>, ShardError> {
        // Fast path: shard already exists
        {
            let shards = self.shards.read().await;
            if let Some(shard) = shards.get(&shard_id) {
                return Ok(shard.clone());
            }
        }

        // Slow path: create and start shard
        let shard = self.create_shard(shard_id).await?;
        self.start_shard(shard_id).await?;
        Ok(shard)
    }

    /// Gets metadata about a shard.
    pub async fn shard_info(&self, shard_id: ShardId) -> Result<ShardInfo, ShardError> {
        let shard = self.get_shard(shard_id).await?;

        let is_leader = shard.is_leader();
        let leader = shard.leader();

        Ok(ShardInfo {
            shard_id,
            has_leader: leader.is_some(),
            leader,
            is_leader,
        })
    }

    /// Gets metadata for all shards.
    #[allow(dead_code)]
    pub async fn all_shard_info(&self) -> Vec<ShardInfo> {
        let shard_ids: Vec<ShardId> = self.shards.read().await.keys().copied().collect();

        let mut infos = Vec::with_capacity(shard_ids.len());
        for shard_id in shard_ids {
            if let Ok(info) = self.shard_info(shard_id).await {
                infos.push(info);
            }
        }

        infos
    }

    /// Returns the number of shards currently created.
    #[allow(dead_code)]
    pub async fn shard_count(&self) -> usize {
        self.shards.read().await.len()
    }

    /// Returns a list of all active shard IDs.
    ///
    /// Active shards are those that have been created (lazily or explicitly).
    pub async fn active_shards(&self) -> Vec<ShardId> {
        self.shards.read().await.keys().copied().collect()
    }

    /// Shuts down all shards gracefully.
    pub async fn shutdown(&self) -> Result<(), ShardError> {
        tracing::info!("Shutting down all shards...");

        let shard_ids: Vec<ShardId> = self.shards.read().await.keys().copied().collect();

        for shard_id in shard_ids {
            if let Ok(shard) = self.get_shard(shard_id).await {
                if let Err(e) = shard.shutdown().await {
                    tracing::warn!("Failed to shutdown shard {}: {:?}", shard_id, e);
                }
            }
        }

        tracing::info!("All shards shut down");
        Ok(())
    }

    /// Gets the Raft directory for a shard.
    fn shard_raft_dir(&self, shard_id: ShardId) -> PathBuf {
        self.config.raft_dir().join(format!("shard-{}", shard_id))
    }

    /// Gets the LSM directory for a shard.
    fn shard_lsm_dir(&self, shard_id: ShardId) -> PathBuf {
        self.config.lsm_dir().join(format!("shard-{}", shard_id))
    }

    /// Gets the RPC sender for a shard (used by gRPC transport to route messages).
    pub async fn get_rpc_sender(&self, shard_id: ShardId) -> Option<tokio::sync::mpsc::Sender<RpcMessage>> {
        self.rpc_senders.read().await.get(&shard_id).cloned()
    }
}

/// Implementation of ShardManagerOps trait for admin operations.
#[async_trait::async_trait]
impl norikv_transport_grpc::ShardManagerOps for ShardManager {
    async fn get_shard(&self, shard_id: u32) -> Result<Arc<ReplicatedLSM>, String> {
        self.get_shard(shard_id)
            .await
            .map_err(|e| format!("{:?}", e))
    }

    async fn shard_info(&self, shard_id: u32) -> Result<norikv_transport_grpc::ShardMetadata, String> {
        let info = self.shard_info(shard_id)
            .await
            .map_err(|e| format!("{:?}", e))?;

        let shard = self.get_shard(shard_id)
            .await
            .map_err(|e| format!("{:?}", e))?;

        let term = shard.raft().current_term();
        let commit_index = shard.raft().commit_index();

        Ok(norikv_transport_grpc::ShardMetadata {
            shard_id: info.shard_id,
            is_leader: info.is_leader,
            leader: info.leader,
            term: term.as_u64(),
            commit_index: commit_index.as_u64(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_raft::transport::InMemoryTransport;
    use std::time::Duration;

    async fn create_test_shard_manager() -> ShardManager {
        let temp_dir = tempfile::tempdir().unwrap();

        let config = ServerConfig {
            node_id: "test-node".to_string(),
            rpc_addr: "127.0.0.1:7447".to_string(),
            http_addr: "127.0.0.1:8447".to_string(),
            data_dir: temp_dir.path().to_path_buf(),
            cluster: crate::config::ClusterConfig {
                seed_nodes: vec![],
                total_shards: 8, // Small number for testing
                replication_factor: 3,
            },
            telemetry: crate::config::TelemetryConfig {
                prometheus: crate::config::PrometheusConfig {
                    enabled: false,
                    port: 9090,
                },
                otlp: crate::config::OtlpConfig::default(),
            },
        };

        let raft_config = RaftConfig::default();
        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = temp_dir.path().to_path_buf();

        let node_id = NodeId::new("test-node");
        let transport = Arc::new(InMemoryTransport::new(node_id.clone(), HashMap::new()));
        let initial_config = ConfigEntry::Single(vec![node_id]);

        ShardManager::new(config, raft_config, lsm_config, transport, initial_config)
    }

    #[tokio::test]
    async fn test_create_single_shard() {
        let manager = create_test_shard_manager().await;

        // Create shard 0
        let result = manager.create_shard(0).await;
        assert!(result.is_ok());

        // Shard should exist
        assert!(manager.get_shard(0).await.is_ok());

        // Creating again should fail
        let result = manager.create_shard(0).await;
        assert!(matches!(result, Err(ShardError::AlreadyExists(0))));
    }

    #[tokio::test]
    async fn test_get_or_create_shard() {
        let manager = create_test_shard_manager().await;

        // Shard doesn't exist yet
        assert!(manager.get_shard(1).await.is_err());

        // Get or create should create it
        let result = manager.get_or_create_shard(1).await;
        assert!(result.is_ok());

        // Now it should exist
        assert!(manager.get_shard(1).await.is_ok());

        // Get or create again should return existing
        let result = manager.get_or_create_shard(1).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_shard_count() {
        let manager = create_test_shard_manager().await;

        assert_eq!(manager.shard_count().await, 0);

        manager.create_shard(0).await.unwrap();
        assert_eq!(manager.shard_count().await, 1);

        manager.create_shard(1).await.unwrap();
        assert_eq!(manager.shard_count().await, 2);
    }

    #[tokio::test]
    async fn test_shard_info() {
        let manager = create_test_shard_manager().await;

        manager.create_shard(0).await.unwrap();
        manager.start_shard(0).await.unwrap();

        // Wait for leader election with retry
        let mut is_leader = false;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(info) = manager.shard_info(0).await {
                if info.is_leader {
                    is_leader = true;
                    break;
                }
            }
        }

        let info = manager.shard_info(0).await.unwrap();
        assert_eq!(info.shard_id, 0);
        // In single-node mode, should be leader
        assert!(is_leader, "Node did not become leader within 2 seconds");
    }
}
