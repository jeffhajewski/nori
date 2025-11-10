//! KV backend trait for abstracting over single-shard and multi-shard routing.

use bytes::Bytes;
use nori_raft::{LogIndex, NodeId, RaftError, Term};
use std::time::Duration;

/// Backend interface for KV operations.
///
/// This trait abstracts over:
/// - Single-shard: Direct ReplicatedLSM
/// - Multi-shard: Router that distributes keys across shards
#[async_trait::async_trait]
pub trait KvBackend: Send + Sync {
    /// Put a key-value pair with optional TTL.
    async fn put(
        &self,
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    ) -> Result<LogIndex, RaftError>;

    /// Get a value by key.
    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, RaftError>;

    /// Delete a key.
    async fn delete(&self, key: Bytes) -> Result<LogIndex, RaftError>;

    /// Check if this backend is currently leader (for the relevant shard).
    fn is_leader(&self) -> bool;

    /// Get the leader node ID (if known).
    fn leader(&self) -> Option<NodeId>;

    /// Get the current Raft term.
    fn current_term(&self) -> Term;

    /// Get the commit index.
    fn commit_index(&self) -> LogIndex;
}

/// Adapter to make ReplicatedLSM implement KvBackend.
pub struct SingleShardBackend {
    lsm: std::sync::Arc<nori_raft::ReplicatedLSM>,
}

impl SingleShardBackend {
    /// Create a new single-shard backend.
    pub fn new(lsm: std::sync::Arc<nori_raft::ReplicatedLSM>) -> Self {
        Self { lsm }
    }
}

#[async_trait::async_trait]
impl KvBackend for SingleShardBackend {
    async fn put(
        &self,
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    ) -> Result<LogIndex, RaftError> {
        self.lsm.replicated_put(key, value, ttl).await
    }

    async fn get(&self, key: &[u8]) -> Result<Option<Bytes>, RaftError> {
        self.lsm.replicated_get(key).await
    }

    async fn delete(&self, key: Bytes) -> Result<LogIndex, RaftError> {
        self.lsm.replicated_delete(key).await
    }

    fn is_leader(&self) -> bool {
        self.lsm.is_leader()
    }

    fn leader(&self) -> Option<NodeId> {
        self.lsm.leader()
    }

    fn current_term(&self) -> Term {
        self.lsm.raft().current_term()
    }

    fn commit_index(&self) -> LogIndex {
        self.lsm.raft().commit_index()
    }
}
