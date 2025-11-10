//! gRPC server that hosts all services.

use crate::admin::AdminService;
use crate::kv::KvService;
use crate::kv_backend::{KvBackend, SingleShardBackend};
use crate::meta::MetaService;
use crate::raft_service::RaftService;
use crate::proto::{
    admin_server::AdminServer, kv_server::KvServer, meta_server::MetaServer,
    raft_server::RaftServer,
};
use nori_raft::transport::RpcMessage;
use nori_raft::ReplicatedLSM;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tonic::transport::Server;

/// gRPC server wrapper.
///
/// Hosts all gRPC services (Kv, Meta, Admin, optionally Raft) and manages the server lifecycle.
pub struct GrpcServer {
    addr: SocketAddr,
    backend: Arc<dyn KvBackend>,
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
            raft_rpc_tx: None,
            shutdown_tx: None,
            server_handle: None,
        }
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
        let kv_service = KvService::new(self.backend.clone());
        let meta_service = MetaService::new();
        let admin_service = AdminService::new();

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
