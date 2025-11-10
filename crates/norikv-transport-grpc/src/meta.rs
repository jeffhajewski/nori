//! Meta service implementation.
//!
//! Handles WatchCluster - streaming cluster membership updates.

use crate::proto::{self, meta_server::Meta};
use tonic::{Request, Response, Status};
use std::sync::Arc;

/// Trait for accessing cluster view state.
///
/// This allows the Meta service to be generic over the cluster view provider.
/// The server app will implement this using ClusterViewManager.
pub trait ClusterViewProvider: Send + Sync {
    /// Get the current cluster view.
    fn current(&self) -> ClusterView;

    /// Subscribe to cluster view updates.
    ///
    /// Returns a receiver that will receive new ClusterView on each change.
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ClusterView>;
}

/// Cluster view state (internal representation).
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterView {
    /// Monotonically increasing version number
    pub epoch: u64,

    /// All nodes in the cluster
    pub nodes: Vec<ClusterNode>,

    /// Shard assignments and leadership
    pub shards: Vec<ShardInfo>,
}

/// Information about a cluster node (internal representation).
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterNode {
    /// Node ID
    pub id: String,

    /// Node address (host:port)
    pub addr: String,

    /// Node role (\"leader\", \"follower\", \"candidate\")
    pub role: String,
}

/// Information about a shard (internal representation).
#[derive(Clone, Debug, PartialEq)]
pub struct ShardInfo {
    /// Shard ID (0..total_shards)
    pub id: u32,

    /// Replicas hosting this shard
    pub replicas: Vec<ShardReplica>,
}

/// Information about a shard replica (internal representation).
#[derive(Clone, Debug, PartialEq)]
pub struct ShardReplica {
    /// Node ID hosting this replica
    pub node_id: String,

    /// Is this replica the leader?
    pub is_leader: bool,
}

/// Meta service implementation.
///
/// Streams cluster topology updates to clients via WatchCluster.
pub struct MetaService {
    /// Cluster view provider (typically ClusterViewManager)
    cluster_view: Option<Arc<dyn ClusterViewProvider>>,
}

impl MetaService {
    /// Create a new Meta service.
    pub fn new() -> Self {
        Self {
            cluster_view: None,
        }
    }

    /// Create a Meta service with a cluster view provider.
    pub fn with_cluster_view(cluster_view: Arc<dyn ClusterViewProvider>) -> Self {
        Self {
            cluster_view: Some(cluster_view),
        }
    }
}

impl Default for MetaService {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert internal ClusterView to proto ClusterView.
fn to_proto_cluster_view(view: &ClusterView) -> proto::ClusterView {
    proto::ClusterView {
        epoch: view.epoch,
        nodes: view.nodes.iter().map(to_proto_cluster_node).collect(),
        shards: view.shards.iter().map(to_proto_shard_info).collect(),
    }
}

/// Convert internal ClusterNode to proto ClusterNode.
fn to_proto_cluster_node(node: &ClusterNode) -> proto::ClusterNode {
    proto::ClusterNode {
        id: node.id.clone(),
        addr: node.addr.clone(),
        role: node.role.clone(),
    }
}

/// Convert internal ShardInfo to proto ShardInfo.
fn to_proto_shard_info(shard: &ShardInfo) -> proto::ShardInfo {
    proto::ShardInfo {
        id: shard.id,
        replicas: shard.replicas.iter().map(to_proto_shard_replica).collect(),
    }
}

/// Convert internal ShardReplica to proto ShardReplica.
fn to_proto_shard_replica(replica: &ShardReplica) -> proto::ShardReplica {
    proto::ShardReplica {
        node_id: replica.node_id.clone(),
        leader: replica.is_leader,
    }
}

#[tonic::async_trait]
impl Meta for MetaService {
    type WatchClusterStream =
        tokio_stream::wrappers::ReceiverStream<Result<proto::ClusterView, Status>>;

    async fn watch_cluster(
        &self,
        _request: Request<proto::ClusterView>,
    ) -> Result<Response<Self::WatchClusterStream>, Status> {
        // Check if cluster view is available
        let cluster_view = self.cluster_view.as_ref().ok_or_else(|| {
            tracing::error!("WatchCluster called but no cluster view provider is configured");
            Status::internal("Cluster view not available")
        })?;

        tracing::info!("Client subscribing to cluster view updates");

        // Create channel for streaming updates
        let (tx, rx) = tokio::sync::mpsc::channel(16);

        // Get current view and send as first message
        let current_view = cluster_view.current();
        let proto_view = to_proto_cluster_view(&current_view);

        if tx.send(Ok(proto_view)).await.is_err() {
            tracing::warn!("Client disconnected before receiving initial cluster view");
            return Err(Status::cancelled("Client disconnected"));
        }

        tracing::debug!("Sent initial cluster view: epoch={}", current_view.epoch);

        // Subscribe to updates
        let mut update_rx = cluster_view.subscribe();

        // Spawn task to forward updates to client
        tokio::spawn(async move {
            loop {
                match update_rx.recv().await {
                    Ok(view) => {
                        tracing::debug!("Received cluster view update: epoch={}", view.epoch);
                        let proto_view = to_proto_cluster_view(&view);

                        if tx.send(Ok(proto_view)).await.is_err() {
                            tracing::info!("Client disconnected from cluster view stream");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Client lagged behind, skipped {} cluster view updates", skipped);
                        // Continue receiving, client will get the next update
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("Cluster view channel closed");
                        break;
                    }
                }
            }
            tracing::debug!("Cluster view stream task terminated");
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}
