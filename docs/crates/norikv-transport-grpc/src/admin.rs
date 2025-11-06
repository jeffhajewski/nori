//! Admin service implementation.
//!
//! Handles administrative operations like TransferShard, SnapshotShard.

use crate::proto::{self, admin_server::Admin};
use tonic::{Request, Response, Status};

/// Admin service implementation.
///
/// TODO: Implement shard management operations.
pub struct AdminService {}

impl AdminService {
    /// Create a new Admin service.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for AdminService {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl Admin for AdminService {
    async fn transfer_shard(
        &self,
        _request: Request<proto::ShardInfo>,
    ) -> Result<Response<proto::ShardInfo>, Status> {
        // TODO: Implement shard transfer
        tracing::warn!("TransferShard called but not yet implemented");
        Err(Status::unimplemented("TransferShard not yet implemented"))
    }

    async fn snapshot_shard(
        &self,
        _request: Request<proto::ShardInfo>,
    ) -> Result<Response<proto::ShardInfo>, Status> {
        // TODO: Implement shard snapshot
        tracing::warn!("SnapshotShard called but not yet implemented");
        Err(Status::unimplemented("SnapshotShard not yet implemented"))
    }
}
