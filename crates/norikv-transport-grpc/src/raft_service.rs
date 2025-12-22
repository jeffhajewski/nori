//! Raft RPC service implementation (server-side).
//!
//! Receives Raft RPCs from peer nodes over gRPC and forwards them to the
//! appropriate shard's Raft core via per-shard channels.

use crate::proto::{self, raft_server::Raft};
use bytes::Bytes;
use nori_raft::transport::RpcMessage;
use nori_raft::types::{
    AppendEntriesRequest, InstallSnapshotRequest, LogEntry, LogIndex, NodeId, ReadIndexRequest,
    RequestVoteRequest, Term,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, RwLock};
use tonic::{Request, Response, Status};

/// Extracts shard ID from a NodeId string.
///
/// NodeIds follow the pattern `"{base_node}-shard{shard_id}"`.
/// For example: "node1-shard5" → Some(5)
fn extract_shard_id(node_id: &str) -> Option<u32> {
    node_id.rsplit("-shard").next()?.parse().ok()
}

/// Raft service implementation with per-shard routing.
///
/// Receives RPCs from network and forwards to the appropriate shard's Raft core.
/// Routes based on shard ID extracted from the NodeId in each RPC.
pub struct RaftService {
    /// Per-shard channels: shard_id → Sender<RpcMessage>
    rpc_channels: Arc<RwLock<HashMap<u32, tokio::sync::mpsc::Sender<RpcMessage>>>>,
}

impl RaftService {
    /// Create a new Raft service with per-shard channel routing.
    ///
    /// # Arguments
    /// * `rpc_channels` - Map of shard_id to RPC channel for that shard
    pub fn new(rpc_channels: Arc<RwLock<HashMap<u32, tokio::sync::mpsc::Sender<RpcMessage>>>>) -> Self {
        Self { rpc_channels }
    }

    /// Get the RPC channel for a specific shard.
    async fn get_shard_channel(&self, shard_id: u32) -> Result<tokio::sync::mpsc::Sender<RpcMessage>, Status> {
        let channels = self.rpc_channels.read().await;
        channels
            .get(&shard_id)
            .cloned()
            .ok_or_else(|| Status::unavailable(format!("Shard {} not initialized", shard_id)))
    }

    /// Convert protobuf LogEntry to Rust type
    fn from_proto_entry(entry: &proto::LogEntry) -> LogEntry {
        LogEntry {
            term: Term(entry.term),
            index: LogIndex(entry.index),
            command: Bytes::from(entry.command.clone()),
        }
    }

    /// Convert Rust LogEntry to protobuf (for future AppendEntries handling)
    #[allow(dead_code)]
    fn to_proto_entry(entry: &LogEntry) -> proto::LogEntry {
        proto::LogEntry {
            term: entry.term.as_u64(),
            index: entry.index.as_u64(),
            command: entry.command.to_vec(),
        }
    }
}

#[tonic::async_trait]
impl Raft for RaftService {
    async fn request_vote(
        &self,
        request: Request<proto::RequestVoteRequest>,
    ) -> Result<Response<proto::RequestVoteResponse>, Status> {
        let req = request.into_inner();

        // Extract shard ID from candidate_id
        let shard_id = extract_shard_id(&req.candidate_id)
            .ok_or_else(|| Status::invalid_argument(format!(
                "Invalid candidate_id format: {}. Expected 'node-shardN'", req.candidate_id
            )))?;

        tracing::debug!(
            "RequestVote RPC: term={}, candidate={}, shard={}",
            req.term,
            req.candidate_id,
            shard_id
        );

        // Get the channel for this shard
        let rpc_tx = self.get_shard_channel(shard_id).await?;

        // Convert protobuf to Rust types
        let raft_req = RequestVoteRequest {
            term: Term(req.term),
            candidate_id: NodeId::new(req.candidate_id),
            last_log_index: LogIndex(req.last_log_index),
            last_log_term: Term(req.last_log_term),
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        rpc_tx
            .send(RpcMessage::RequestVote {
                request: raft_req,
                response_tx,
            })
            .await
            .map_err(|_| Status::internal("Raft core unavailable"))?;

        // Wait for response from Raft core
        let response = response_rx
            .await
            .map_err(|_| Status::internal("Failed to receive response from Raft"))?;

        Ok(Response::new(proto::RequestVoteResponse {
            term: response.term.as_u64(),
            vote_granted: response.vote_granted,
        }))
    }

    async fn append_entries(
        &self,
        request: Request<proto::AppendEntriesRequest>,
    ) -> Result<Response<proto::AppendEntriesResponse>, Status> {
        let req = request.into_inner();

        // Extract shard ID from leader_id
        let shard_id = extract_shard_id(&req.leader_id)
            .ok_or_else(|| Status::invalid_argument(format!(
                "Invalid leader_id format: {}. Expected 'node-shardN'", req.leader_id
            )))?;

        tracing::debug!(
            "AppendEntries RPC: term={}, leader={}, shard={}, entries={}",
            req.term,
            req.leader_id,
            shard_id,
            req.entries.len()
        );

        // Get the channel for this shard
        let rpc_tx = self.get_shard_channel(shard_id).await?;

        // Convert protobuf to Rust types
        let raft_req = AppendEntriesRequest {
            term: Term(req.term),
            leader_id: NodeId::new(req.leader_id),
            prev_log_index: LogIndex(req.prev_log_index),
            prev_log_term: Term(req.prev_log_term),
            entries: req.entries.iter().map(Self::from_proto_entry).collect(),
            leader_commit: LogIndex(req.leader_commit),
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        rpc_tx
            .send(RpcMessage::AppendEntries {
                request: raft_req,
                response_tx,
            })
            .await
            .map_err(|_| Status::internal("Raft core unavailable"))?;

        // Wait for response
        let response = response_rx
            .await
            .map_err(|_| Status::internal("Failed to receive response from Raft"))?;

        Ok(Response::new(proto::AppendEntriesResponse {
            term: response.term.as_u64(),
            success: response.success,
            conflict_index: response.conflict_index.map(|i| i.as_u64()).unwrap_or(0),
            last_log_index: response.last_log_index.as_u64(),
        }))
    }

    async fn install_snapshot(
        &self,
        request: Request<proto::InstallSnapshotRequest>,
    ) -> Result<Response<proto::InstallSnapshotResponse>, Status> {
        let req = request.into_inner();

        // Extract shard ID from leader_id
        let shard_id = extract_shard_id(&req.leader_id)
            .ok_or_else(|| Status::invalid_argument(format!(
                "Invalid leader_id format: {}. Expected 'node-shardN'", req.leader_id
            )))?;

        tracing::debug!(
            "InstallSnapshot RPC: term={}, leader={}, shard={}, offset={}, done={}",
            req.term,
            req.leader_id,
            shard_id,
            req.offset,
            req.done
        );

        // Get the channel for this shard
        let rpc_tx = self.get_shard_channel(shard_id).await?;

        // Convert protobuf to Rust types
        let raft_req = InstallSnapshotRequest {
            term: Term(req.term),
            leader_id: NodeId::new(req.leader_id),
            last_included_index: LogIndex(req.last_included_index),
            last_included_term: Term(req.last_included_term),
            offset: req.offset,
            data: Bytes::from(req.data),
            done: req.done,
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        rpc_tx
            .send(RpcMessage::InstallSnapshot {
                request: raft_req,
                response_tx,
            })
            .await
            .map_err(|_| Status::internal("Raft core unavailable"))?;

        // Wait for response
        let response = response_rx
            .await
            .map_err(|_| Status::internal("Failed to receive response from Raft"))?;

        Ok(Response::new(proto::InstallSnapshotResponse {
            term: response.term.as_u64(),
            bytes_stored: response.bytes_stored,
        }))
    }

    async fn read_index(
        &self,
        request: Request<proto::ReadIndexRequest>,
    ) -> Result<Response<proto::ReadIndexResponse>, Status> {
        let req = request.into_inner();

        // Extract shard ID from leader_id
        let shard_id = extract_shard_id(&req.leader_id)
            .ok_or_else(|| Status::invalid_argument(format!(
                "Invalid leader_id format: {}. Expected 'node-shardN'", req.leader_id
            )))?;

        tracing::debug!(
            "ReadIndex RPC: term={}, shard={}, read_id={}, commit_index={}",
            req.term,
            shard_id,
            req.read_id,
            req.commit_index
        );

        // Get the channel for this shard
        let rpc_tx = self.get_shard_channel(shard_id).await?;

        // Convert protobuf to Rust types
        let raft_req = ReadIndexRequest {
            term: Term(req.term),
            read_id: req.read_id,
            commit_index: LogIndex(req.commit_index),
            leader_id: NodeId::new(req.leader_id),
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        rpc_tx
            .send(RpcMessage::ReadIndex {
                request: raft_req,
                response_tx,
            })
            .await
            .map_err(|_| Status::internal("Raft core unavailable"))?;

        // Wait for response
        let response = response_rx
            .await
            .map_err(|_| Status::internal("Failed to receive response from Raft"))?;

        Ok(Response::new(proto::ReadIndexResponse {
            term: response.term.as_u64(),
            ack: response.ack,
            read_id: response.read_id,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_shard_id() {
        assert_eq!(extract_shard_id("node1-shard0"), Some(0));
        assert_eq!(extract_shard_id("node1-shard5"), Some(5));
        assert_eq!(extract_shard_id("node1-shard1023"), Some(1023));
        assert_eq!(extract_shard_id("my-node-shard42"), Some(42));

        // Invalid formats
        assert_eq!(extract_shard_id("node1"), None);
        assert_eq!(extract_shard_id("node1-shardX"), None);
        assert_eq!(extract_shard_id(""), None);
    }

    #[test]
    fn test_entry_conversion() {
        let proto = proto::LogEntry {
            term: 5,
            index: 42,
            command: b"test_command".to_vec(),
        };

        let rust_entry = RaftService::from_proto_entry(&proto);
        assert_eq!(rust_entry.term, Term(5));
        assert_eq!(rust_entry.index, LogIndex(42));
        assert_eq!(rust_entry.command, Bytes::from("test_command"));

        let back_to_proto = RaftService::to_proto_entry(&rust_entry);
        assert_eq!(back_to_proto.term, proto.term);
        assert_eq!(back_to_proto.index, proto.index);
        assert_eq!(back_to_proto.command, proto.command);
    }

    #[tokio::test]
    async fn test_service_creation() {
        let channels = Arc::new(RwLock::new(HashMap::new()));
        let _service = RaftService::new(channels);
        // Service should be created successfully
    }

    #[tokio::test]
    async fn test_get_shard_channel_not_found() {
        let channels = Arc::new(RwLock::new(HashMap::new()));
        let service = RaftService::new(channels);

        let result = service.get_shard_channel(5).await;
        assert!(result.is_err());

        let status = result.unwrap_err();
        assert_eq!(status.code(), tonic::Code::Unavailable);
        assert!(status.message().contains("Shard 5 not initialized"));
    }

    #[tokio::test]
    async fn test_get_shard_channel_found() {
        let (tx, _rx) = tokio::sync::mpsc::channel(100);
        let mut map = HashMap::new();
        map.insert(5u32, tx);

        let channels = Arc::new(RwLock::new(map));
        let service = RaftService::new(channels);

        let result = service.get_shard_channel(5).await;
        assert!(result.is_ok());
    }
}
