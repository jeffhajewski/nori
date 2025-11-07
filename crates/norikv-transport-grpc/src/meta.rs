//! Meta service implementation.
//!
//! Handles WatchCluster - streaming cluster membership updates.

use crate::proto::{self, meta_server::Meta};
use tonic::{Request, Response, Status};

/// Meta service implementation.
///
/// TODO: Integrate with SWIM membership for real cluster updates.
pub struct MetaService {}

impl MetaService {
    /// Create a new Meta service.
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for MetaService {
    fn default() -> Self {
        Self::new()
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
        // TODO: Implement cluster watch by subscribing to SWIM membership events
        // For now, return a stub stream
        tracing::warn!("WatchCluster called but not yet implemented");

        let (tx, rx) = tokio::sync::mpsc::channel(4);

        // Spawn a task that sends periodic updates
        tokio::spawn(async move {
            // Send initial empty cluster view
            let _ = tx
                .send(Ok(proto::ClusterView {
                    epoch: 0,
                    nodes: vec![],
                    shards: vec![],
                }))
                .await;
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(
            rx,
        )))
    }
}
