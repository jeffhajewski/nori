//! Transport abstraction for Raft RPC communication.
//!
//! Defines the `RaftTransport` trait that allows pluggable transport implementations:
//! - gRPC transport for production (norikv-transport-grpc crate)
//! - In-memory channels for unit testing
//! - Mock transport for integration tests with simulated failures
//!
//! All RPC calls are async and return `Result<Response, RaftError>`.

use crate::error::Result;
use crate::types::*;
use async_trait::async_trait;

/// Transport abstraction for Raft RPC communication.
///
/// Implementations handle:
/// - Connection management (pooling, retries, backoff)
/// - Serialization/deserialization (bincode, protobuf, etc.)
/// - Network failures (timeouts, connection errors)
/// - Observability (latency metrics, error rates)
///
/// # Implementation Notes
///
/// - All methods are async and may take significant time (network I/O)
/// - Transport should handle retries internally (not Raft's concern)
/// - Timeouts should be configurable per RPC type
/// - Transient errors (network down) should be retried; permanent errors (node shutdown) should fail fast
/// - NodeId is an opaque identifier; transport resolves it to actual address (DNS, IP:port, etc.)
#[async_trait]
pub trait RaftTransport: Send + Sync {
    /// Send RequestVote RPC to a peer.
    ///
    /// Used during leader election. Candidate sends this to all peers to request votes.
    /// Returns the peer's response (vote granted or denied).
    ///
    /// Errors:
    /// - `Io`: Network error (unreachable, timeout, connection refused)
    /// - `Serialization`: Failed to encode request or decode response
    async fn request_vote(
        &self,
        target: &NodeId,
        request: RequestVoteRequest,
    ) -> Result<RequestVoteResponse>;

    /// Send AppendEntries RPC to a peer.
    ///
    /// Used for:
    /// - Heartbeats (empty entries list)
    /// - Log replication (non-empty entries)
    ///
    /// Leader sends this to all followers periodically.
    /// Returns the follower's response (success or conflict info).
    ///
    /// Errors:
    /// - `Io`: Network error
    /// - `Serialization`: Failed to encode/decode
    async fn append_entries(
        &self,
        target: &NodeId,
        request: AppendEntriesRequest,
    ) -> Result<AppendEntriesResponse>;

    /// Send InstallSnapshot RPC to a peer.
    ///
    /// Used when follower is too far behind (log compacted).
    /// Leader streams snapshot in chunks.
    ///
    /// Returns the follower's response (bytes stored so far).
    ///
    /// Errors:
    /// - `Io`: Network error
    /// - `Serialization`: Failed to encode/decode
    async fn install_snapshot(
        &self,
        target: &NodeId,
        request: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse>;

    /// Send ReadIndex RPC to a peer.
    ///
    /// Used for linearizable reads when leader lease expired.
    /// Leader sends this to quorum to confirm it's still leader.
    ///
    /// Returns the follower's acknowledgment.
    ///
    /// Errors:
    /// - `Io`: Network error
    /// - `Serialization`: Failed to encode/decode
    async fn read_index(
        &self,
        target: &NodeId,
        request: ReadIndexRequest,
    ) -> Result<ReadIndexResponse>;
}

/// In-memory transport for testing (local channels, no network).
///
/// Allows testing Raft logic without actual network I/O.
/// Useful for:
/// - Unit tests (single-threaded, deterministic)
/// - Integration tests (multi-node clusters in-process)
/// - Chaos tests (controlled network partitions, delays)
///
/// # Example
///
/// ```ignore
/// use std::collections::HashMap;
/// use std::sync::Arc;
/// use tokio::sync::mpsc;
///
/// // Create channels for each node
/// let mut channels = HashMap::new();
/// for node_id in &["n1", "n2", "n3"] {
///     let (tx, rx) = mpsc::channel(100);
///     channels.insert(NodeId::new(*node_id), (tx, rx));
/// }
///
/// // Create transport for node "n1"
/// let transport = Arc::new(InMemoryTransport::new(
///     NodeId::new("n1"),
///     channels.clone(),
/// ));
/// ```
pub struct InMemoryTransport {
    /// This node's ID
    local_id: NodeId,

    /// Channels to other nodes (NodeId → sender)
    /// Each node has a receiver for incoming RPCs
    peers: std::sync::Arc<parking_lot::RwLock<std::collections::HashMap<NodeId, RpcSender>>>,
}

/// RPC message envelope (tagged union of all RPC types)
#[derive(Debug)]
pub enum RpcMessage {
    RequestVote {
        request: RequestVoteRequest,
        response_tx: tokio::sync::oneshot::Sender<RequestVoteResponse>,
    },
    AppendEntries {
        request: AppendEntriesRequest,
        response_tx: tokio::sync::oneshot::Sender<AppendEntriesResponse>,
    },
    InstallSnapshot {
        request: InstallSnapshotRequest,
        response_tx: tokio::sync::oneshot::Sender<InstallSnapshotResponse>,
    },
    ReadIndex {
        request: ReadIndexRequest,
        response_tx: tokio::sync::oneshot::Sender<ReadIndexResponse>,
    },
}

pub type RpcSender = tokio::sync::mpsc::Sender<RpcMessage>;
pub type RpcReceiver = tokio::sync::mpsc::Receiver<RpcMessage>;

impl InMemoryTransport {
    /// Create a new in-memory transport.
    ///
    /// `local_id`: This node's ID
    /// `peers`: Map of peer IDs to their RPC message senders
    pub fn new(
        local_id: NodeId,
        peers: std::collections::HashMap<NodeId, RpcSender>,
    ) -> Self {
        Self {
            local_id,
            peers: std::sync::Arc::new(parking_lot::RwLock::new(peers)),
        }
    }

    /// Add a peer to the transport.
    ///
    /// Used for dynamic membership changes (though RaftConfig handles this at higher level).
    pub fn add_peer(&self, peer_id: NodeId, sender: RpcSender) {
        self.peers.write().insert(peer_id, sender);
    }

    /// Remove a peer from the transport.
    pub fn remove_peer(&self, peer_id: &NodeId) {
        self.peers.write().remove(peer_id);
    }

    /// Get peer sender (for sending RPCs).
    fn get_peer(&self, peer_id: &NodeId) -> Option<RpcSender> {
        self.peers.read().get(peer_id).cloned()
    }
}

#[async_trait]
impl RaftTransport for InMemoryTransport {
    async fn request_vote(
        &self,
        target: &NodeId,
        request: RequestVoteRequest,
    ) -> Result<RequestVoteResponse> {
        let peer = self
            .get_peer(target)
            .ok_or_else(|| crate::error::RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("peer not found: {}", target),
                ),
            })?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        peer.send(RpcMessage::RequestVote {
            request,
            response_tx,
        })
        .await
        .map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })?;

        response_rx.await.map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })
    }

    async fn append_entries(
        &self,
        target: &NodeId,
        request: AppendEntriesRequest,
    ) -> Result<AppendEntriesResponse> {
        let peer = self
            .get_peer(target)
            .ok_or_else(|| crate::error::RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("peer not found: {}", target),
                ),
            })?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        peer.send(RpcMessage::AppendEntries {
            request,
            response_tx,
        })
        .await
        .map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })?;

        response_rx.await.map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })
    }

    async fn install_snapshot(
        &self,
        target: &NodeId,
        request: InstallSnapshotRequest,
    ) -> Result<InstallSnapshotResponse> {
        let peer = self
            .get_peer(target)
            .ok_or_else(|| crate::error::RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("peer not found: {}", target),
                ),
            })?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        peer.send(RpcMessage::InstallSnapshot {
            request,
            response_tx,
        })
        .await
        .map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })?;

        response_rx.await.map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })
    }

    async fn read_index(
        &self,
        target: &NodeId,
        request: ReadIndexRequest,
    ) -> Result<ReadIndexResponse> {
        let peer = self
            .get_peer(target)
            .ok_or_else(|| crate::error::RaftError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("peer not found: {}", target),
                ),
            })?;

        let (response_tx, response_rx) = tokio::sync::oneshot::channel();
        peer.send(RpcMessage::ReadIndex {
            request,
            response_tx,
        })
        .await
        .map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })?;

        response_rx.await.map_err(|e| crate::error::RaftError::Io {
            source: std::io::Error::new(std::io::ErrorKind::BrokenPipe, e.to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_in_memory_transport_request_vote() {
        let (tx1, mut rx1) = tokio::sync::mpsc::channel(10);
        let (tx2, _rx2) = tokio::sync::mpsc::channel(10);

        let mut peers = HashMap::new();
        peers.insert(NodeId::new("n1"), tx1);
        peers.insert(NodeId::new("n2"), tx2.clone());

        let transport = InMemoryTransport::new(NodeId::new("n2"), peers);

        // Spawn a task to respond to the RPC
        tokio::spawn(async move {
            if let Some(RpcMessage::RequestVote {
                request: _,
                response_tx,
            }) = rx1.recv().await
            {
                let _ = response_tx.send(RequestVoteResponse {
                    term: Term(5),
                    vote_granted: true,
                });
            }
        });

        let request = RequestVoteRequest {
            term: Term(5),
            candidate_id: NodeId::new("n2"),
            last_log_index: LogIndex(10),
            last_log_term: Term(4),
        };

        let response = transport.request_vote(&NodeId::new("n1"), request).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        assert_eq!(response.term, Term(5));
        assert!(response.vote_granted);
    }

    #[tokio::test]
    async fn test_in_memory_transport_peer_not_found() {
        let peers = HashMap::new();
        let transport = InMemoryTransport::new(NodeId::new("n1"), peers);

        let request = RequestVoteRequest {
            term: Term(5),
            candidate_id: NodeId::new("n1"),
            last_log_index: LogIndex(10),
            last_log_term: Term(4),
        };

        let response = transport
            .request_vote(&NodeId::new("unknown"), request)
            .await;
        assert!(response.is_err());
        assert!(matches!(response.unwrap_err(), crate::error::RaftError::Io { .. }));
    }
}
