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
use nori_swim::Membership;
use norikv_transport_grpc::ClusterViewProvider;
use parking_lot::RwLock;
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
                // Nodes are updated by topology watcher (handle_membership_event)
                // Not updated here to avoid race conditions
                nodes: current.nodes.clone(),
                shards: vec![],
            }
        };

        // Query shard manager for all active shards on this node
        // Note: In a multi-node cluster, each node tracks its own active shards.
        // The full cluster view would aggregate shard info from all nodes.
        let active_shards = self.shard_manager.active_shards().await;

        for shard_id in active_shards {
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

    /// Start topology watcher task.
    ///
    /// Subscribes to SWIM membership events and updates ClusterView accordingly.
    /// Returns a task handle that can be aborted on shutdown.
    pub fn start_topology_watcher(
        self: Arc<Self>,
        swim: Arc<nori_swim::SwimMembership>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut events = swim.events();

            tracing::info!("Topology watcher started, listening for SWIM events");

            loop {
                match events.recv().await {
                    Ok(event) => {
                        if let Err(e) = self.handle_membership_event(event).await {
                            tracing::error!("Failed to handle membership event: {:?}", e);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Topology watcher lagged, skipped {} events", skipped);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("SWIM event channel closed, topology watcher exiting");
                        break;
                    }
                }
            }
        })
    }

    /// Handle a SWIM membership event.
    async fn handle_membership_event(
        &self,
        event: nori_swim::MembershipEvent,
    ) -> Result<(), ClusterViewError> {
        use nori_swim::MembershipEvent;

        match event {
            MembershipEvent::MemberJoined { id, addr } => {
                tracing::info!("Member joined: {} at {}", id, addr);
                self.add_node(id, addr.to_string()).await?;
            }
            MembershipEvent::MemberSuspect { id, incarnation } => {
                tracing::warn!("Member suspected: {} (incarnation {})", id, incarnation);
                self.update_node_role(&id, "suspect").await?;
            }
            MembershipEvent::MemberFailed { id } => {
                tracing::warn!("Member failed: {}", id);
                self.update_node_role(&id, "failed").await?;
            }
            MembershipEvent::MemberLeft { id } => {
                tracing::info!("Member left: {}", id);
                self.remove_node(&id).await?;
            }
            MembershipEvent::MemberAlive { id, incarnation } => {
                tracing::info!("Member alive: {} (incarnation {})", id, incarnation);
                self.update_node_role(&id, "follower").await?;
            }
        }

        // Trigger a refresh to update shard assignments
        self.refresh().await?;

        Ok(())
    }

    /// Add a new node to the cluster view.
    async fn add_node(
        &self,
        node_id: String,
        address: String,
    ) -> Result<(), ClusterViewError> {
        let mut view = self.view.write();

        // Check if node already exists
        if view.nodes.iter().any(|n| n.id == node_id) {
            tracing::debug!("Node {} already exists in cluster view", node_id);
            return Ok(());
        }

        view.epoch += 1;
        view.nodes.push(ClusterNode {
            id: node_id,
            addr: address,
            role: "follower".to_string(),
        });

        tracing::info!("Added node to cluster view (epoch {})", view.epoch);

        Ok(())
    }

    /// Update a node's role in the cluster view.
    async fn update_node_role(
        &self,
        node_id: &str,
        role: &str,
    ) -> Result<(), ClusterViewError> {
        let mut view = self.view.write();

        // Find the node and check if update is needed
        let needs_update = view.nodes.iter()
            .find(|n| n.id == node_id)
            .map(|n| n.role != role)
            .unwrap_or(false);

        if needs_update {
            // Increment epoch first
            view.epoch += 1;
            let new_epoch = view.epoch;

            // Then update the role
            if let Some(node) = view.nodes.iter_mut().find(|n| n.id == node_id) {
                node.role = role.to_string();
                tracing::info!("Updated node {} role to {} (epoch {})", node_id, role, new_epoch);
            }
        }

        Ok(())
    }

    /// Remove a node from the cluster view.
    async fn remove_node(&self, node_id: &str) -> Result<(), ClusterViewError> {
        let mut view = self.view.write();

        let initial_len = view.nodes.len();
        view.nodes.retain(|n| n.id != node_id);

        if view.nodes.len() < initial_len {
            view.epoch += 1;
            tracing::info!("Removed node {} from cluster view (epoch {})", node_id, view.epoch);
        }

        Ok(())
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
