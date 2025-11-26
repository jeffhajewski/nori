//! gRPC server that hosts all services.

use crate::admin::{AdminService, ShardManagerOps};
use crate::kv::KvService;
use crate::kv_backend::{KvBackend, SingleShardBackend};
use crate::meta::{ClusterViewProvider, MetaService};
use crate::raft_service::RaftService;
use crate::vector::VectorService;
use crate::vector_backend::VectorBackend;
use crate::proto::{
    admin_server::AdminServer, kv_server::KvServer, meta_server::MetaServer,
    raft_server::RaftServer, vector_server::VectorServer,
};
use nori_observe::Meter;
use nori_raft::transport::RpcMessage;
use nori_raft::ReplicatedLSM;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tonic::transport::Server;

/// gRPC server wrapper.
///
/// Hosts all gRPC services (Kv, Meta, Admin, Vector, optionally Raft) and manages the server lifecycle.
pub struct GrpcServer {
    addr: SocketAddr,
    backend: Arc<dyn KvBackend>,
    vector_backend: Option<Arc<dyn VectorBackend>>,
    cluster_view: Option<Arc<dyn ClusterViewProvider>>,
    shard_manager: Option<Arc<dyn ShardManagerOps>>,
    meter: Option<Arc<dyn Meter>>,
    raft_rpc_tx: Option<tokio::sync::mpsc::Sender<RpcMessage>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_handle: Option<JoinHandle<Result<(), tonic::transport::Error>>>,
}

impl GrpcServer {
    /// Create a new gRPC server (single-shard mode).
    ///
    /// # Arguments
    /// - `addr`: Socket address to bind to
    /// - `replicated_lsm`: ReplicatedLSM instance for KV operations
    pub fn new(addr: SocketAddr, replicated_lsm: Arc<ReplicatedLSM>) -> Self {
        // Wrap in SingleShardBackend for backwards compatibility
        let backend: Arc<dyn KvBackend> = Arc::new(SingleShardBackend::new(replicated_lsm));
        Self {
            addr,
            backend,
            vector_backend: None,
            cluster_view: None,
            shard_manager: None,
            meter: None,
            raft_rpc_tx: None,
            shutdown_tx: None,
            server_handle: None,
        }
    }

    /// Create a new gRPC server with a custom backend.
    ///
    /// # Arguments
    /// - `addr`: Socket address to bind to
    /// - `backend`: KvBackend implementation (single-shard or multi-shard routing)
    pub fn with_backend(addr: SocketAddr, backend: Arc<dyn KvBackend>) -> Self {
        Self {
            addr,
            backend,
            vector_backend: None,
            cluster_view: None,
            shard_manager: None,
            meter: None,
            raft_rpc_tx: None,
            shutdown_tx: None,
            server_handle: None,
        }
    }

    /// Set the cluster view provider for Meta service.
    ///
    /// If provided, the Meta.WatchCluster service will stream cluster topology updates.
    ///
    /// # Arguments
    /// - `cluster_view`: ClusterViewProvider implementation
    pub fn with_cluster_view(mut self, cluster_view: Arc<dyn ClusterViewProvider>) -> Self {
        self.cluster_view = Some(cluster_view);
        self
    }

    /// Set the shard manager for Admin service.
    ///
    /// If provided, Admin service operations (SnapshotShard, etc.) will be enabled.
    ///
    /// # Arguments
    /// - `shard_manager`: ShardManagerOps implementation
    pub fn with_shard_manager(mut self, shard_manager: Arc<dyn ShardManagerOps>) -> Self {
        self.shard_manager = Some(shard_manager);
        self
    }

    /// Set the vector backend for Vector service.
    ///
    /// If provided, Vector service operations (CreateIndex, Insert, Search, etc.) will be enabled.
    ///
    /// # Arguments
    /// - `vector_backend`: VectorBackend implementation (single-shard or multi-shard routing)
    pub fn with_vector_backend(mut self, vector_backend: Arc<dyn VectorBackend>) -> Self {
        self.vector_backend = Some(vector_backend);
        self
    }

    /// Set the metrics meter.
    ///
    /// If provided, KvService will track request counts and latencies.
    ///
    /// # Arguments
    /// - `meter`: Meter implementation for metrics collection
    pub fn with_meter(mut self, meter: Arc<dyn Meter>) -> Self {
        self.meter = Some(meter);
        self
    }

    /// Set the Raft RPC channel.
    ///
    /// If provided, the Raft service will be enabled and RPCs will be forwarded to this channel.
    ///
    /// # Arguments
    /// - `rpc_tx`: Channel to forward Raft RPCs to Raft core
    pub fn with_raft_rpc(mut self, rpc_tx: tokio::sync::mpsc::Sender<RpcMessage>) -> Self {
        self.raft_rpc_tx = Some(rpc_tx);
        self
    }

    /// Start the gRPC server.
    ///
    /// Spawns a background task to run the server.
    /// Returns immediately after starting.
    pub async fn start(&mut self) -> Result<(), GrpcServerError> {
        tracing::info!("Starting gRPC server on {}", self.addr);

        // Create services
        let kv_service = if let Some(meter) = &self.meter {
            tracing::info!("Enabling KV metrics collection");
            KvService::with_meter(self.backend.clone(), meter.clone())
        } else {
            KvService::new(self.backend.clone())
        };

        // Create Meta service with cluster view if available
        let meta_service = if let Some(cluster_view) = &self.cluster_view {
            tracing::info!("Enabling Meta.WatchCluster service with cluster view");
            MetaService::with_cluster_view(cluster_view.clone())
        } else {
            tracing::warn!("Meta.WatchCluster service starting without cluster view");
            MetaService::new()
        };

        // Create Admin service with shard manager if available
        let admin_service = if let Some(shard_manager) = &self.shard_manager {
            tracing::info!("Enabling Admin service with shard management operations");
            AdminService::new(shard_manager.clone())
        } else {
            tracing::warn!("Admin service starting without shard manager - SnapshotShard will be unavailable");
            // Create a dummy shard manager that returns errors
            use crate::admin::ShardMetadata;

            struct DummyShardManager;

            #[async_trait::async_trait]
            impl ShardManagerOps for DummyShardManager {
                async fn get_shard(&self, shard_id: u32) -> Result<Arc<nori_raft::ReplicatedLSM>, String> {
                    Err(format!("Shard manager not configured (shard {})", shard_id))
                }

                async fn shard_info(&self, shard_id: u32) -> Result<ShardMetadata, String> {
                    Err(format!("Shard manager not configured (shard {})", shard_id))
                }
            }

            AdminService::new(Arc::new(DummyShardManager))
        };

        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Build the server
        let mut server_builder = Server::builder()
            .add_service(KvServer::new(kv_service))
            .add_service(MetaServer::new(meta_service))
            .add_service(AdminServer::new(admin_service));

        // Add Raft service if RPC channel provided
        if let Some(rpc_tx) = self.raft_rpc_tx.clone() {
            tracing::info!("Enabling Raft peer-to-peer service");
            let raft_service = RaftService::new(rpc_tx);
            server_builder = server_builder.add_service(RaftServer::new(raft_service));
        }

        // Add Vector service if backend provided
        if let Some(vector_backend) = &self.vector_backend {
            tracing::info!("Enabling Vector service for similarity search");
            let vector_service = if let Some(meter) = &self.meter {
                VectorService::with_meter(vector_backend.clone(), meter.clone())
            } else {
                VectorService::new(vector_backend.clone())
            };
            server_builder = server_builder.add_service(VectorServer::new(vector_service));
        }

        let server = server_builder.serve_with_shutdown(self.addr, async {
            shutdown_rx.await.ok();
        });

        // Spawn server task
        let handle = tokio::spawn(async move {
            tracing::info!("gRPC server task started");
            let result = server.await;
            tracing::info!("gRPC server task stopped");
            result
        });

        self.server_handle = Some(handle);

        // Give the server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        tracing::info!("gRPC server started successfully");
        Ok(())
    }

    /// Shutdown the gRPC server gracefully.
    pub async fn shutdown(mut self) -> Result<(), GrpcServerError> {
        tracing::info!("Shutting down gRPC server");

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for server task to finish
        if let Some(handle) = self.server_handle.take() {
            handle
                .await
                .map_err(|e| GrpcServerError::ShutdownError(e.to_string()))?
                .map_err(|e| GrpcServerError::ServerError(e.to_string()))?;
        }

        tracing::info!("gRPC server shutdown complete");
        Ok(())
    }

    /// Get the bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GrpcServerError {
    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Shutdown error: {0}")]
    ShutdownError(String),
}
