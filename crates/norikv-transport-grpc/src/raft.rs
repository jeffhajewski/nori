//! Raft transport implementation over gRPC.
//!
//! Provides GrpcRaftTransport which implements the RaftTransport trait,
//! enabling Raft nodes to communicate over the network via gRPC.

use crate::proto::{self, raft_client::RaftClient};
use bytes::Bytes;
use nori_raft::error::{RaftError, Result};
use nori_raft::transport::RaftTransport;
use nori_raft::types::{
    AppendEntriesRequest, AppendEntriesResponse, InstallSnapshotRequest,
    InstallSnapshotResponse, LogEntry, LogIndex, NodeId, ReadIndexRequest, ReadIndexResponse,
    RequestVoteRequest, RequestVoteResponse, Term,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::{Channel, Endpoint};

/// gRPC-based Raft transport.
///
/// Maintains connections to peer nodes and translates between Rust types
/// and protobuf messages for RPC communication.
pub struct GrpcRaftTransport {
    /// Connection pool: NodeId -> gRPC client
    clients: Arc<RwLock<HashMap<String, RaftClient<Channel>>>>,

    /// Peer addresses: NodeId -> "host:port"
    peer_addrs: HashMap<String, String>,
}

impl GrpcRaftTransport {
    /// Create a new gRPC Raft transport.
    ///
    /// # Arguments
    /// * `peer_addrs` - Map of NodeId to gRPC endpoint addresses (e.g., "127.0.0.1:9001")
    pub fn new(peer_addrs: HashMap<String, String>) -> Self {
        Self {
            clients: Arc::new(RwLock::new(HashMap::new())),
            peer_addrs,
        }
    }

    /// Get or create a gRPC client for a peer node.
    async fn get_client(&self, target: &NodeId) -> Result<RaftClient<Channel>> {
        let target_id = target.as_str();

        // Check if we already have a connection
        {
            let clients = self.clients.read().await;
            if let Some(client) = clients.get(target_id) {
                return Ok(client.clone());
            }
        }

        // Create a new connection
        let addr = self
            .peer_addrs
            .get(target_id)
            .ok_or_else(|| RaftError::ConfigError {
                reason: format!("Unknown peer: {}", target_id),
            })?;

        let endpoint = Endpoint::from_shared(format!("http://{}", addr))
            .map_err(|e| RaftError::ConfigError {
                reason: format!("Invalid endpoint: {}", e),
            })?
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(10));

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    format!("Failed to connect to {}: {}", addr, e),
                ),
            })?;

        let client = RaftClient::new(channel);

        // Cache the connection
        {
            let mut clients = self.clients.write().await;
            clients.insert(target_id.to_string(), client.clone());
        }

        Ok(client)
    }

    /// Convert Rust LogEntry to protobuf
    fn to_proto_entry(entry: &LogEntry) -> proto::LogEntry {
        proto::LogEntry {
            term: entry.term.as_u64(),
            index: entry.index.as_u64(),
            command: entry.command.to_vec(),
        }
    }

    /// Convert protobuf LogEntry to Rust
    fn from_proto_entry(_entry: &proto::LogEntry) -> LogEntry {
        LogEntry {
            term: Term(_entry.term),
            index: LogIndex(_entry.index),
            command: Bytes::from(_entry.command.clone()),
        }
    }
}

#[tonic::async_trait]
impl RaftTransport for GrpcRaftTransport {
    async fn request_vote(
        &self,
        target: &NodeId,
        request: RequestVoteRequest,
    ) -> Result<RequestVoteResponse> {
        let mut client = self.get_client(target).await?;

        let proto_req = proto::RequestVoteRequest {
            term: request.term.as_u64(),
            candidate_id: request.candidate_id.as_str().to_string(),
            last_log_index: request.last_log_index.as_u64(),
            last_log_term: request.last_log_term.as_u64(),
        };

        let response = client
            .request_vote(proto_req)
            .await
            .map_err(|e| RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("RequestVote RPC failed: {}", e),
                ),
            })?
            .into_inner();

        Ok(RequestVoteResponse {
            term: Term(response.term),
            vote_granted: response.vote_granted,
        })
    }

    async fn append_entries(
        &self,
        target: &NodeId,
        request: AppendEntriesRequest,
    ) -> Result<AppendEntriesResponse> {
        let mut client = self.get_client(target).await?;

        let proto_req = proto::AppendEntriesRequest {
            term: request.term.as_u64(),
            leader_id: request.leader_id.as_str().to_string(),
            prev_log_index: request.prev_log_index.as_u64(),
            prev_log_term: request.prev_log_term.as_u64(),
            entries: request.entries.iter().map(Self::to_proto_entry).collect(),
            leader_commit: request.leader_commit.as_u64(),
        };

        let response = client
            .append_entries(proto_req)
            .await
            .map_err(|e| RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("AppendEntries RPC failed: {}", e),
                ),
            })?
            .into_inner();

        Ok(AppendEntriesResponse {
            term: Term(response.term),
            success: response.success,
            conflict_index: if response.conflict_index > 0 {
                Some(LogIndex(response.conflict_index))
            } else {
                None
            },
            last_log_index: LogIndex(response.last_log_index),
        })
    }

    async fn install_snapshot(
        &self,
        target: &NodeId,
        request: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse> {
        let mut client = self.get_client(target).await?;

        let proto_req = proto::InstallSnapshotRequest {
            term: request.term.as_u64(),
            leader_id: request.leader_id.as_str().to_string(),
            last_included_index: request.last_included_index.as_u64(),
            last_included_term: request.last_included_term.as_u64(),
            offset: request.offset,
            data: request.data.to_vec(),
            done: request.done,
        };

        let response = client
            .install_snapshot(proto_req)
            .await
            .map_err(|e| RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("InstallSnapshot RPC failed: {}", e),
                ),
            })?
            .into_inner();

        Ok(InstallSnapshotResponse {
            term: Term(response.term),
            bytes_stored: response.bytes_stored,
        })
    }

    async fn read_index(
        &self,
        target: &NodeId,
        request: ReadIndexRequest,
    ) -> Result<ReadIndexResponse> {
        let mut client = self.get_client(target).await?;

        let proto_req = proto::ReadIndexRequest {
            term: request.term.as_u64(),
            read_id: request.read_id,
            commit_index: request.commit_index.as_u64(),
        };

        let response = client
            .read_index(proto_req)
            .await
            .map_err(|e| RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("ReadIndex RPC failed: {}", e),
                ),
            })?
            .into_inner();

        Ok(ReadIndexResponse {
            term: Term(response.term),
            ack: response.ack,
            read_id: response.read_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entry_conversion() {
        let entry = LogEntry {
            term: Term(5),
            index: LogIndex(42),
            command: Bytes::from("test_command"),
        };

        let proto = GrpcRaftTransport::to_proto_entry(&entry);
        assert_eq!(proto.term, 5);
        assert_eq!(proto.index, 42);
        assert_eq!(proto.command, b"test_command");

        let back = GrpcRaftTransport::from_proto_entry(&proto);
        assert_eq!(back.term, entry.term);
        assert_eq!(back.index, entry.index);
        assert_eq!(back.command, entry.command);
    }

    #[test]
    fn test_transport_creation() {
        let mut peers = HashMap::new();
        peers.insert("node1".to_string(), "127.0.0.1:9001".to_string());
        peers.insert("node2".to_string(), "127.0.0.1:9002".to_string());

        let transport = GrpcRaftTransport::new(peers.clone());
        assert_eq!(transport.peer_addrs.len(), 2);
        assert_eq!(
            transport.peer_addrs.get("node1"),
            Some(&"127.0.0.1:9001".to_string())
        );
    }
}
