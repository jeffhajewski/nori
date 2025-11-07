//! Server node composition.
//!
//! Wires together all components (Raft, LSM, transport) via dependency injection.

use crate::config::ServerConfig;
use nori_lsm::ATLLConfig;
use nori_raft::{log::RaftLog, transport::RpcMessage, ConfigEntry, NodeId, RaftConfig, ReplicatedLSM};
use norikv_transport_grpc::{GrpcRaftTransport, GrpcServer};
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

    /// Raft RPC receiver channel (if multi-node)
    raft_rpc_tx: Option<tokio::sync::mpsc::Sender<RpcMessage>>,

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

        let node_id = NodeId::new(&config.node_id);

        // Determine cluster mode: single-node or multi-node
        let is_single_node = config.cluster.seed_nodes.is_empty();

        let (transport, initial_config, raft_rpc_tx, raft_rpc_rx) = if is_single_node {
            tracing::info!("Starting in single-node mode (no seed nodes configured)");

            // Single-node: Use in-memory transport
            let transport = Arc::new(nori_raft::transport::InMemoryTransport::new(
                node_id.clone(),
                HashMap::new(),
            ));
            let config = ConfigEntry::Single(vec![node_id.clone()]);

            (transport as Arc<dyn nori_raft::transport::RaftTransport>, config, None, None)
        } else {
            tracing::info!("Starting in multi-node mode with {} seed nodes", config.cluster.seed_nodes.len());

            // Multi-node: Use gRPC transport
            // Build peer address map: NodeId -> "host:port"
            let mut peer_addrs = HashMap::new();

            // Parse own address from rpc_addr
            let own_addr = config.rpc_addr.clone();
            peer_addrs.insert(config.node_id.clone(), own_addr.clone());

            // Add seed nodes (using hostname as node_id for now)
            // TODO: Use proper node discovery to map node_id -> address
            for (i, seed) in config.cluster.seed_nodes.iter().enumerate() {
                let node_name = format!("node{}", i);
                peer_addrs.insert(node_name.clone(), seed.clone());
            }

            tracing::info!("Peer addresses configured: {:?}", peer_addrs);

            let transport = Arc::new(GrpcRaftTransport::new(peer_addrs.clone()));

            // Build cluster configuration from seed nodes
            // Use unique node IDs based on addresses to avoid duplicates
            let mut cluster_nodes = vec![node_id.clone()];

            // Add peer nodes (avoiding duplicates)
            for (peer_id, _addr) in &peer_addrs {
                let peer_node_id = NodeId::new(peer_id);
                if peer_node_id != node_id {
                    cluster_nodes.push(peer_node_id);
                }
            }

            let config = ConfigEntry::Single(cluster_nodes);

            // Create RPC receiver channel (bounded, capacity 1000)
            let (tx, rx) = tokio::sync::mpsc::channel(1000);

            (transport as Arc<dyn nori_raft::transport::RaftTransport>, config, Some(tx), Some(rx))
        };

        tracing::info!("Creating ReplicatedLSM with initial config: {:?}", initial_config);

        // Create ReplicatedLSM
        let replicated_lsm = ReplicatedLSM::new(
            node_id,
            raft_config,
            lsm_config,
            raft_log,
            transport,
            initial_config,
            raft_rpc_rx,
        )
        .await
        .map_err(|e| {
            NodeError::Initialization(format!("Failed to create ReplicatedLSM: {:?}", e))
        })?;

        tracing::info!("ReplicatedLSM created successfully");

        Ok(Self {
            config,
            replicated_lsm: Arc::new(replicated_lsm),
            raft_rpc_tx,
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

        // Wire Raft RPC channel if multi-node
        if let Some(raft_rpc_tx) = self.raft_rpc_tx.clone() {
            tracing::info!("Enabling Raft peer-to-peer RPC service");
            grpc_server = grpc_server.with_raft_rpc(raft_rpc_tx);
        }

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

    /// Get health status of the node.
    pub fn health(&self) -> norikv_transport_grpc::HealthStatus {
        let health_service = norikv_transport_grpc::HealthService::new(self.replicated_lsm.clone());
        health_service.check_health()
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
