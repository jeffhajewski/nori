//! Raft RPC service implementation (server-side).
//!
//! Receives Raft RPCs from peer nodes over gRPC and forwards them to the
//! local Raft node via a channel for processing.

use crate::proto::{self, raft_server::Raft};
use bytes::Bytes;
use nori_raft::transport::RpcMessage;
use nori_raft::types::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
    InstallSnapshotResponse, LogEntry, LogIndex, NodeId, ReadIndexRequest, ReadIndexResponse,
    RequestVoteRequest, RequestVoteResponse, Term,
};
use tokio::sync::oneshot;
use tonic::{Request, Response, Status};

/// Raft service implementation.
///
/// Receives RPCs from network and forwards to Raft core via channel.
/// This decouples network I/O from Raft logic and enables testing.
pub struct RaftService {
    /// Channel to send RPC requests to Raft core
    rpc_tx: tokio::sync::mpsc::Sender<RpcMessage>,
}

impl RaftService {
    /// Create a new Raft service.
    ///
    /// # Arguments
    /// * `rpc_tx` - Channel to forward RPC requests to Raft core
    pub fn new(rpc_tx: tokio::sync::mpsc::Sender<RpcMessage>) -> Self {
        Self { rpc_tx }
    }

    /// Convert protobuf LogEntry to Rust type
    fn from_proto_entry(entry: &proto::LogEntry) -> LogEntry {
        LogEntry {
            term: Term(entry.term),
            index: LogIndex(entry.index),
            command: Bytes::from(entry.command.clone()),
        }
    }

    /// Convert Rust LogEntry to protobuf
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

        tracing::debug!(
            "RequestVote RPC: term={}, candidate={}",
            req.term,
            req.candidate_id
        );

        // Convert protobuf to Rust types
        let raft_req = RequestVoteRequest {
            term: Term(req.term),
            candidate_id: NodeId::new(req.candidate_id),
            last_log_index: LogIndex(req.last_log_index),
            last_log_term: Term(req.last_log_term),
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        self.rpc_tx
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

        tracing::debug!(
            "AppendEntries RPC: term={}, leader={}, entries={}",
            req.term,
            req.leader_id,
            req.entries.len()
        );

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
        self.rpc_tx
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

        tracing::debug!(
            "InstallSnapshot RPC: term={}, leader={}, offset={}, done={}",
            req.term,
            req.leader_id,
            req.offset,
            req.done
        );

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
        self.rpc_tx
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

        tracing::debug!(
            "ReadIndex RPC: term={}, read_id={}, commit_index={}",
            req.term,
            req.read_id,
            req.commit_index
        );

        // Convert protobuf to Rust types
        let raft_req = ReadIndexRequest {
            term: Term(req.term),
            read_id: req.read_id,
            commit_index: LogIndex(req.commit_index),
        };

        // Forward to Raft core
        let (response_tx, response_rx) = oneshot::channel();
        self.rpc_tx
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
        let (tx, _rx) = tokio::sync::mpsc::channel(100);
        let _service = RaftService::new(tx);
        // Service should be created successfully
    }
}
