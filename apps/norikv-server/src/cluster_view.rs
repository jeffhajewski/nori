//! Cluster view management.
//!
//! Tracks cluster topology:
//! - Live nodes and their addresses
//! - Shard assignments (which nodes host which shards)
//! - Shard leadership (which node is leader for each shard)
//!
//! Provides streaming updates to clients via Meta.WatchCluster.

use crate::shard_manager::{ShardManager, ShardId};
use nori_raft::NodeId;
use norikv_transport_grpc::ClusterViewProvider;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Cluster view state.
///
/// Represents the current topology of the cluster.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterView {
    /// Monotonically increasing version number
    pub epoch: u64,

    /// All nodes in the cluster
    pub nodes: Vec<ClusterNode>,

    /// Shard assignments and leadership
    pub shards: Vec<ShardInfo>,
}

/// Information about a cluster node.
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterNode {
    /// Node ID
    pub id: String,

    /// Node address (host:port)
    pub addr: String,

    /// Node role ("leader", "follower", "candidate")
    pub role: String,
}

/// Information about a shard.
#[derive(Clone, Debug, PartialEq)]
pub struct ShardInfo {
    /// Shard ID (0..total_shards)
    pub id: ShardId,

    /// Replicas hosting this shard
    pub replicas: Vec<ShardReplica>,
}

/// Information about a shard replica.
#[derive(Clone, Debug, PartialEq)]
pub struct ShardReplica {
    /// Node ID hosting this replica
    pub node_id: String,

    /// Is this replica the leader?
    pub is_leader: bool,
}

/// Cluster view manager.
///
/// Tracks cluster state and notifies subscribers of changes.
pub struct ClusterViewManager {
    /// Current cluster view
    view: Arc<RwLock<ClusterView>>,

    /// Broadcast channel for cluster view updates
    /// Subscribers receive new ClusterView on each change
    update_tx: broadcast::Sender<ClusterView>,

    /// Shard manager (for querying shard state)
    shard_manager: Arc<ShardManager>,

    /// Server configuration
    node_id: NodeId,
    node_addr: String,
    total_shards: u32,
}

impl ClusterViewManager {
    /// Create a new cluster view manager.
    pub fn new(
        node_id: NodeId,
        node_addr: String,
        total_shards: u32,
        shard_manager: Arc<ShardManager>,
    ) -> Self {
        // Create broadcast channel with capacity for 16 queued updates
        let (update_tx, _) = broadcast::channel(16);

        // Initialize with empty view
        let initial_view = ClusterView {
            epoch: 0,
            nodes: vec![ClusterNode {
                id: node_id.to_string(),
                addr: node_addr.clone(),
                role: "unknown".to_string(),
            }],
            shards: vec![],
        };

        Self {
            view: Arc::new(RwLock::new(initial_view)),
            update_tx,
            shard_manager,
            node_id,
            node_addr,
            total_shards,
        }
    }

    /// Get the current cluster view.
    pub fn current(&self) -> ClusterView {
        self.view.read().clone()
    }

    /// Subscribe to cluster view updates.
    ///
    /// Returns a receiver that will receive new ClusterView on each change.
    pub fn subscribe(&self) -> broadcast::Receiver<ClusterView> {
        self.update_tx.subscribe()
    }

    /// Refresh cluster view from current shard state.
    ///
    /// Should be called periodically (e.g., every 1 second) to update the view.
    pub async fn refresh(&self) -> Result<(), ClusterViewError> {
        let mut new_view = {
            let current = self.view.read();
            ClusterView {
                epoch: current.epoch + 1,
                nodes: current.nodes.clone(), // TODO: Update from SWIM membership
                shards: vec![],
            }
        };

        // Query shard manager for all active shards
        // Note: This is a simplified implementation. In production, we'd:
        // 1. Track all shards (not just active ones)
        // 2. Get leadership info from Raft
        // 3. Track replica placement across cluster

        // For now, we'll query a subset of shards as a placeholder
        for shard_id in 0..self.total_shards.min(10) {
            if let Ok(shard) = self.shard_manager.get_shard(shard_id).await {
                let is_leader = shard.is_leader();

                new_view.shards.push(ShardInfo {
                    id: shard_id,
                    replicas: vec![ShardReplica {
                        node_id: self.node_id.to_string(),
                        is_leader,
                    }],
                });
            }
        }

        // Check if view actually changed
        let changed = {
            let current = self.view.read();
            *current != new_view
        };

        if changed {
            // Update stored view
            {
                let mut view = self.view.write();
                *view = new_view.clone();
            }

            // Notify subscribers (ignore if no subscribers)
            let _ = self.update_tx.send(new_view);
        }

        Ok(())
    }

    /// Start background refresh task.
    ///
    /// Refreshes cluster view every `interval` duration.
    pub fn start_refresh_task(self: Arc<Self>, interval: std::time::Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            loop {
                interval_timer.tick().await;

                if let Err(e) = self.refresh().await {
                    tracing::warn!("Failed to refresh cluster view: {:?}", e);
                }
            }
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClusterViewError {
    #[error("Shard manager error: {0}")]
    ShardManager(String),
}

/// Implementation of ClusterViewProvider trait for ClusterViewManager.
///
/// Allows the Meta service to access cluster view state.
impl ClusterViewProvider for ClusterViewManager {
    fn current(&self) -> norikv_transport_grpc::ClusterView {
        let view = self.view.read().clone();
        to_transport_cluster_view(&view)
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<norikv_transport_grpc::ClusterView> {
        // Create a new receiver from the update channel
        let mut rx = self.update_tx.subscribe();

        // Create a forwarding channel that converts types
        let (tx, new_rx) = tokio::sync::broadcast::channel(16);

        // Spawn task to convert types
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(view) => {
                        let transport_view = to_transport_cluster_view(&view);
                        if tx.send(transport_view).is_err() {
                            // No subscribers, exit
                            break;
                        }
                    }
                    Err(_) => {
                        // Channel closed
                        break;
                    }
                }
            }
        });

        new_rx
    }
}

/// Convert internal ClusterView to transport ClusterView.
fn to_transport_cluster_view(view: &ClusterView) -> norikv_transport_grpc::ClusterView {
    norikv_transport_grpc::ClusterView {
        epoch: view.epoch,
        nodes: view.nodes.iter().map(to_transport_cluster_node).collect(),
        shards: view.shards.iter().map(to_transport_shard_info).collect(),
    }
}

/// Convert internal ClusterNode to transport ClusterNode.
fn to_transport_cluster_node(node: &ClusterNode) -> norikv_transport_grpc::ClusterNode {
    norikv_transport_grpc::ClusterNode {
        id: node.id.clone(),
        addr: node.addr.clone(),
        role: node.role.clone(),
    }
}

/// Convert internal ShardInfo to transport ShardInfo.
fn to_transport_shard_info(shard: &ShardInfo) -> norikv_transport_grpc::ShardInfo {
    norikv_transport_grpc::ShardInfo {
        id: shard.id,
        replicas: shard.replicas.iter().map(to_transport_shard_replica).collect(),
    }
}

/// Convert internal ShardReplica to transport ShardReplica.
fn to_transport_shard_replica(replica: &ShardReplica) -> norikv_transport_grpc::ShardReplica {
    norikv_transport_grpc::ShardReplica {
        node_id: replica.node_id.clone(),
        is_leader: replica.is_leader,
    }
}
