//! Admin service implementation.
//!
//! Handles administrative operations like TransferShard, SnapshotShard.

use crate::proto::{self, admin_server::Admin};
use std::sync::Arc;
use tonic::{Request, Response, Status};

/// Shard manager trait for admin operations.
///
/// Abstracts over ShardManager to avoid circular dependencies.
#[async_trait::async_trait]
pub trait ShardManagerOps: Send + Sync {
    /// Get a shard by ID.
    async fn get_shard(&self, shard_id: u32) -> Result<Arc<nori_raft::ReplicatedLSM>, String>;

    /// Get shard metadata (leader, replicas, etc.)
    async fn shard_info(&self, shard_id: u32) -> Result<ShardMetadata, String>;
}

/// Shard metadata for admin responses.
#[derive(Debug, Clone)]
pub struct ShardMetadata {
    pub shard_id: u32,
    pub is_leader: bool,
    pub leader: Option<nori_raft::NodeId>,
    pub term: u64,
    pub commit_index: u64,
}

/// Admin service implementation.
///
/// Provides administrative operations for shard management:
/// - SnapshotShard: Trigger a Raft snapshot for a specific shard
/// - TransferShard: Transfer shard leadership (future feature)
pub struct AdminService {
    shard_manager: Arc<dyn ShardManagerOps>,
}

impl AdminService {
    /// Create a new Admin service with a shard manager.
    pub fn new(shard_manager: Arc<dyn ShardManagerOps>) -> Self {
        Self { shard_manager }
    }
}

#[tonic::async_trait]
impl Admin for AdminService {
    async fn transfer_shard(
        &self,
        _request: Request<proto::ShardInfo>,
    ) -> Result<Response<proto::ShardInfo>, Status> {
        // Future Feature: Shard leadership transfer
        //
        // Implementation plan:
        // 1. Validate request (target node exists, shard exists)
        // 2. Call Raft leadership transfer API
        // 3. Wait for transfer to complete or timeout
        // 4. Return updated ShardInfo with new leader
        //
        // Complexity: Requires Raft leadership transfer support (not yet implemented)
        tracing::warn!("TransferShard called but not yet implemented");
        Err(Status::unimplemented(
            "TransferShard not yet implemented - requires Raft leadership transfer API"
        ))
    }

    async fn snapshot_shard(
        &self,
        request: Request<proto::ShardInfo>,
    ) -> Result<Response<proto::ShardInfo>, Status> {
        let req = request.into_inner();
        let shard_id = req.id;

        tracing::info!("SnapshotShard requested for shard {}", shard_id);

        // Get the shard
        let shard = self
            .shard_manager
            .get_shard(shard_id)
            .await
            .map_err(|e| Status::not_found(format!("Shard {} not found: {}", shard_id, e)))?;

        // Check if this node is the leader (snapshots typically triggered on leader)
        if !shard.is_leader() {
            let leader = shard.leader();
            return Err(Status::failed_precondition(format!(
                "Shard {} is not leader on this node. Leader: {:?}",
                shard_id, leader
            )));
        }

        // Trigger snapshot creation
        shard
            .raft()
            .create_snapshot()
            .await
            .map_err(|e| Status::internal(format!("Failed to create snapshot: {:?}", e)))?;

        tracing::info!("Snapshot created successfully for shard {}", shard_id);

        // Get updated shard metadata
        let metadata = self
            .shard_manager
            .shard_info(shard_id)
            .await
            .map_err(|e| Status::internal(format!("Failed to get shard info: {}", e)))?;

        // Build response
        let response = proto::ShardInfo {
            id: shard_id,
            replicas: vec![proto::ShardReplica {
                node_id: metadata.leader.map(|n| n.to_string()).unwrap_or_default(),
                leader: metadata.is_leader,
            }],
        };

        Ok(Response::new(response))
    }
}
