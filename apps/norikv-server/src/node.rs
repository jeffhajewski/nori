//! Server node composition.
//!
//! Wires together all components (Raft, LSM, transport) via dependency injection.

use crate::cluster_view::ClusterViewManager;
use crate::config::ServerConfig;
use crate::health::HealthChecker;
use crate::shard_manager::ShardManager;
use nori_lsm::ATLLConfig;
use nori_raft::{ConfigEntry, NodeId, RaftConfig};
use nori_swim::{Membership, SwimMembership};
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

    /// Shard manager (manages 1024 Raft groups + LSM engines)
    shard_manager: Arc<ShardManager>,

    /// Cluster view manager (tracks topology and streams updates)
    cluster_view: Arc<ClusterViewManager>,

    /// Health checker (monitors shard health)
    health_checker: Arc<HealthChecker>,

    /// SWIM membership (optional, for multi-node clusters)
    swim: Option<Arc<SwimMembership>>,

    /// gRPC server (optional, created on start)
    grpc_server: Option<GrpcServer>,

    /// HTTP REST API server (optional, created on start)
    http_server: Option<crate::http::HttpServer>,

    /// Cluster view refresh task handle
    cluster_view_task: Option<tokio::task::JoinHandle<()>>,
}

impl Node {
    /// Create a new node from configuration.
    ///
    /// This wires together all components:
    /// 1. Creates ShardManager (manages 1024 Raft groups + LSM engines)
    /// 2. Sets up transport
    /// 3. Initializes membership (if multi-node)
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

        let (transport, initial_config) = if is_single_node {
            tracing::info!("Starting in single-node mode (no seed nodes configured)");

            // Single-node: Use in-memory transport
            let transport = Arc::new(nori_raft::transport::InMemoryTransport::new(
                node_id.clone(),
                HashMap::new(),
            ));
            let config = ConfigEntry::Single(vec![node_id.clone()]);

            (transport as Arc<dyn nori_raft::transport::RaftTransport>, config)
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
            let mut cluster_nodes = vec![node_id.clone()];

            // Add peer nodes (avoiding duplicates)
            for (peer_id, _addr) in &peer_addrs {
                let peer_node_id = NodeId::new(peer_id);
                if peer_node_id != node_id {
                    cluster_nodes.push(peer_node_id);
                }
            }

            let config = ConfigEntry::Single(cluster_nodes);

            (transport as Arc<dyn nori_raft::transport::RaftTransport>, config)
        };

        tracing::info!("Creating ShardManager with {} total shards", config.cluster.total_shards);

        // Create ShardManager (lazy shard initialization - shards created on demand)
        let shard_manager = ShardManager::new(
            config.clone(),
            raft_config,
            lsm_config,
            transport,
            initial_config,
        );

        tracing::info!("ShardManager created successfully");

        let shard_manager_arc = Arc::new(shard_manager);

        // Create cluster view manager
        tracing::info!("Creating ClusterViewManager");
        let cluster_view = Arc::new(ClusterViewManager::new(
            node_id.clone(),
            config.rpc_addr.clone(),
            config.cluster.total_shards,
            shard_manager_arc.clone(),
        ));
        tracing::info!("ClusterViewManager created");

        // Create health checker
        tracing::info!("Creating HealthChecker");
        let health_checker = Arc::new(HealthChecker::new(
            config.node_id.clone(),
            config.cluster.total_shards,
            shard_manager_arc.clone(),
        ));
        tracing::info!("HealthChecker created");

        // Create SWIM membership if multi-node
        let swim = if !is_single_node {
            tracing::info!("Creating SWIM membership for cluster");
            let swim_addr = config.rpc_addr.parse().map_err(|e| {
                NodeError::Initialization(format!("Invalid rpc_addr for SWIM: {}", e))
            })?;
            let swim = Arc::new(SwimMembership::new(config.node_id.clone(), swim_addr));
            Some(swim)
        } else {
            tracing::info!("Single-node mode: SWIM membership disabled");
            None
        };

        Ok(Self {
            config,
            shard_manager: shard_manager_arc,
            cluster_view,
            health_checker,
            swim,
            grpc_server: None,
            http_server: None,
            cluster_view_task: None,
        })
    }

    /// Start the node.
    ///
    /// Starts all background tasks:
    /// - Creates and starts shard 0 (primary shard)
    /// - Starts gRPC server
    /// - Starts SWIM membership (if multi-node)
    ///
    /// Note: Additional shards are created lazily on first access.
    pub async fn start(&mut self) -> Result<(), NodeError> {
        tracing::info!("Starting node");

        // Create and start shard 0 (primary shard for initial operations)
        // Note: Additional shards are created lazily on first access via MultiShardBackend
        tracing::info!("Creating shard 0 (primary shard)");
        let shard_0 = self.shard_manager
            .get_or_create_shard(0)
            .await
            .map_err(|e| NodeError::Startup(format!("Failed to create shard 0: {:?}", e)))?;

        tracing::info!("Shard 0 created and started");

        // Wait for leader election (single-node should elect itself immediately)
        tokio::time::sleep(Duration::from_millis(500)).await;

        if shard_0.is_leader() {
            tracing::info!("Shard 0 is leader (other shards will be created on demand)");
        } else {
            tracing::warn!("Shard 0 is not leader yet");
        }

        // Start gRPC server with multi-shard routing
        let addr: std::net::SocketAddr = self.config.rpc_addr
            .parse()
            .map_err(|e| NodeError::Startup(format!("Invalid rpc_addr: {}", e)))?;

        // Create multi-shard backend for request routing
        let backend = Arc::new(crate::multi_shard_backend::MultiShardBackend::new(
            self.shard_manager.clone(),
            self.config.cluster.total_shards,
        ));

        let mut grpc_server = norikv_transport_grpc::GrpcServer::with_backend(addr, backend)
            .with_cluster_view(self.cluster_view.clone() as Arc<dyn norikv_transport_grpc::ClusterViewProvider>);

        // Wire Raft RPC channel if multi-node
        // Note: For now, only shard 0's RPC channel is wired
        // Phase 1.4 will implement per-shard RPC routing
        if let Some(raft_rpc_tx) = self.shard_manager.get_rpc_sender(0).await {
            tracing::info!("Enabling Raft peer-to-peer RPC service for shard 0");
            grpc_server = grpc_server.with_raft_rpc(raft_rpc_tx);
        }

        grpc_server.start().await
            .map_err(|e| NodeError::Startup(format!("Failed to start gRPC server: {:?}", e)))?;

        tracing::info!("gRPC server started on {}", addr);
        self.grpc_server = Some(grpc_server);

        // Start HTTP REST API server
        let http_addr: std::net::SocketAddr = self.config.http_addr
            .parse()
            .map_err(|e| NodeError::Startup(format!("Invalid http_addr: {}", e)))?;

        let mut http_server = crate::http::HttpServer::new(http_addr, self.health_checker.clone());
        http_server.start().await
            .map_err(|e| NodeError::Startup(format!("Failed to start HTTP server: {:?}", e)))?;

        tracing::info!("HTTP server started on {}", http_addr);
        self.http_server = Some(http_server);

        // Start cluster view refresh task (updates every 1 second)
        tracing::info!("Starting cluster view refresh task");
        let cluster_view_task = self.cluster_view.clone().start_refresh_task(Duration::from_secs(1));
        self.cluster_view_task = Some(cluster_view_task);
        tracing::info!("Cluster view refresh task started");

        // Start SWIM membership and join cluster
        if let Some(swim) = &self.swim {
            tracing::info!("Starting SWIM membership");
            swim.start().await
                .map_err(|e| NodeError::Startup(format!("Failed to start SWIM: {:?}", e)))?;

            // Join cluster via seed nodes
            if !self.config.cluster.seed_nodes.is_empty() {
                for seed in &self.config.cluster.seed_nodes {
                    let seed_addr = seed.parse().map_err(|e| {
                        NodeError::Startup(format!("Invalid seed node address: {}", e))
                    })?;

                    tracing::info!("Joining cluster via seed: {}", seed_addr);
                    swim.join(seed_addr).await
                        .map_err(|e| NodeError::Startup(format!("Failed to join cluster: {:?}", e)))?;

                    // For now, just join one seed successfully
                    break;
                }
            }

            tracing::info!("SWIM membership initialized");
        }

        Ok(())
    }

    /// Shutdown the node gracefully.
    ///
    /// Stops all background tasks and flushes data:
    /// - Cluster view refresh task
    /// - HTTP server
    /// - gRPC server
    /// - SWIM membership
    /// - All shard Raft groups and LSM engines
    pub async fn shutdown(mut self) -> Result<(), NodeError> {
        tracing::info!("Shutting down node");

        // Stop cluster view refresh task
        if let Some(task) = self.cluster_view_task.take() {
            tracing::info!("Stopping cluster view refresh task");
            task.abort();
            tracing::info!("Cluster view refresh task stopped");
        }

        // Stop HTTP server
        if let Some(http_server) = self.http_server.take() {
            http_server.shutdown().await
                .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown HTTP server: {:?}", e)))?;
            tracing::info!("HTTP server shutdown complete");
        }

        // Stop gRPC server
        if let Some(grpc_server) = self.grpc_server.take() {
            grpc_server.shutdown().await
                .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown gRPC server: {:?}", e)))?;
            tracing::info!("gRPC server shutdown complete");
        }

        // SWIM leave
        if let Some(swim) = &self.swim {
            tracing::info!("Leaving SWIM cluster");
            swim.shutdown().await
                .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown SWIM: {:?}", e)))?;
            tracing::info!("SWIM shutdown complete");
        }

        // Shutdown all shards
        self.shard_manager
            .shutdown()
            .await
            .map_err(|e| NodeError::Shutdown(format!("Failed to shutdown ShardManager: {:?}", e)))?;

        tracing::info!("Node shutdown complete");

        Ok(())
    }

    /// Get reference to ShardManager (for testing/debugging).
    pub fn shard_manager(&self) -> &Arc<ShardManager> {
        &self.shard_manager
    }

    /// Get reference to ClusterViewManager.
    pub fn cluster_view(&self) -> &Arc<ClusterViewManager> {
        &self.cluster_view
    }

    /// Get reference to HealthChecker.
    pub fn health_checker(&self) -> &Arc<HealthChecker> {
        &self.health_checker
    }

    /// Get comprehensive health status of the node (all shards).
    pub async fn health(&self) -> crate::health::ServerHealthStatus {
        self.health_checker.check().await
    }

    /// Quick health check (for load balancers).
    pub async fn health_quick(&self) -> bool {
        self.health_checker.check_quick().await
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
