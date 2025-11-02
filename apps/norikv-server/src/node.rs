//! Server node composition.
//!
//! Wires together all components (Raft, LSM, transport) via dependency injection.

use crate::config::ServerConfig;
use nori_lsm::ATLLConfig;
use nori_raft::{log::RaftLog, ConfigEntry, NodeId, RaftConfig, ReplicatedLSM};
use norikv_transport_grpc::GrpcServer;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Server node - main composition root.
///
/// Holds all components and manages their lifecycle.
pub struct Node {
    /// Node configuration
    config: ServerConfig,

    /// Replicated LSM (Raft + LSM integration)
    replicated_lsm: Arc<ReplicatedLSM>,

    /// gRPC server (optional, created on start)
    grpc_server: Option<GrpcServer>,
}

impl Node {
    /// Create a new node from configuration.
    ///
    /// This wires together all components:
    /// 1. Opens Raft log
    /// 2. Opens LSM engine
    /// 3. Creates ReplicatedLSM
    /// 4. Sets up transport
    /// 5. Initializes membership (future)
    /// 6. Sets up placement (future)
    pub async fn new(config: ServerConfig) -> Result<Self, NodeError> {
        tracing::info!("Initializing node: {}", config.node_id);

        // Create data directories
        let raft_dir = config.raft_dir();
        let lsm_dir = config.lsm_dir();

        std::fs::create_dir_all(&raft_dir)
            .map_err(|e| NodeError::Initialization(format!("Failed to create raft dir: {}", e)))?;

        std::fs::create_dir_all(&lsm_dir)
            .map_err(|e| NodeError::Initialization(format!("Failed to create lsm dir: {}", e)))?;

        tracing::info!("Raft directory: {}", raft_dir.display());
        tracing::info!("LSM directory: {}", lsm_dir.display());

        // Open Raft log
        let (raft_log, _) = RaftLog::open(&raft_dir)
            .await
            .map_err(|e| NodeError::Initialization(format!("Failed to open raft log: {}", e)))?;

        tracing::info!("Raft log opened");

        // Configure Raft
        let mut raft_config = RaftConfig::default();
        raft_config.election_timeout_min = Duration::from_millis(150);
        raft_config.election_timeout_max = Duration::from_millis(300);

        // Configure LSM
        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = lsm_dir.clone();

        // Create transport (currently in-memory for single-node)
        // TODO: Replace with TCP transport for multi-node
        let node_id = NodeId::new(&config.node_id);
        let transport = Arc::new(nori_raft::transport::InMemoryTransport::new(
            node_id.clone(),
            HashMap::new(),
        ));

        // Initial cluster configuration (single node for now)
        // TODO: Load from config.cluster.seed_nodes
        let initial_config = ConfigEntry::Single(vec![node_id.clone()]);

        tracing::info!("Creating ReplicatedLSM");

        // Create ReplicatedLSM
        let replicated_lsm = ReplicatedLSM::new(
            node_id,
            raft_config,
            lsm_config,
            raft_log,
            transport,
            initial_config,
            None, // No RPC receiver for single-node
        )
        .await
        .map_err(|e| {
            NodeError::Initialization(format!("Failed to create ReplicatedLSM: {:?}", e))
        })?;

        tracing::info!("ReplicatedLSM created successfully");

        Ok(Self {
            config,
            replicated_lsm: Arc::new(replicated_lsm),
            grpc_server: None,
        })
    }

    /// Start the node.
    ///
    /// Starts all background tasks:
    /// - Raft election timer
    /// - Raft heartbeat loop
    /// - LSM compaction
    /// - gRPC server
    /// - SWIM membership (future)
    pub async fn start(&mut self) -> Result<(), NodeError> {
        tracing::info!("Starting node");

        // Start ReplicatedLSM (starts Raft background tasks)
        self.replicated_lsm
            .start()
            .await
            .map_err(|e| NodeError::Startup(format!("Failed to start ReplicatedLSM: {:?}", e)))?;

        tracing::info!("ReplicatedLSM started");

        // Wait for leader election (single-node should elect itself immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        if self.replicated_lsm.is_leader() {
            tracing::info!("Node is leader");
        } else {
            tracing::warn!("Node is not leader");
        }

        // Start gRPC server
        let addr: std::net::SocketAddr = self.config.rpc_addr
            .parse()
            .map_err(|e| NodeError::Startup(format!("Invalid rpc_addr: {}", e)))?;

        let mut grpc_server = GrpcServer::new(addr, self.replicated_lsm.clone());
        grpc_server.start().await
            .map_err(|e| NodeError::Startup(format!("Failed to start gRPC server: {:?}", e)))?;

        tracing::info!("gRPC server started on {}", addr);
        self.grpc_server = Some(grpc_server);

        // TODO: Join SWIM cluster

        Ok(())
    }

    /// Shutdown the node gracefully.
    ///
    /// Stops all background tasks and flushes data:
    /// - gRPC server
    /// - SWIM leave (future)
    /// - Raft shutdown
    /// - LSM shutdown (flush memtable, sync WAL)
    pub async fn shutdown(mut self) -> Result<(), NodeError> {
        tracing::info!("Shutting down node");

        // Stop gRPC server
        if let Some(grpc_server) = self.grpc_server.take() {
            grpc_server.shutdown().await
                .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown gRPC server: {:?}", e)))?;
            tracing::info!("gRPC server shutdown complete");
        }

        // TODO: SWIM leave

        // Shutdown ReplicatedLSM
        self.replicated_lsm
            .shutdown()
            .await
            .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown ReplicatedLSM: {:?}", e)))?;

        tracing::info!("Node shutdown complete");

        Ok(())
    }

    /// Get reference to ReplicatedLSM (for testing/debugging).
    pub fn replicated_lsm(&self) -> &Arc<ReplicatedLSM> {
        &self.replicated_lsm
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("Initialization error: {0}")]
    Initialization(String),

    #[error("Startup error: {0}")]
    Startup(String),

    #[error("Shutdown error: {0}")]
    Shutdown(String),

    #[error("Runtime error: {0}")]
    Runtime(String),
}
