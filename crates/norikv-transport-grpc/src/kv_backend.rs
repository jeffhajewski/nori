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

    /// Get a value by key, returning (value, term, log_index) if found.
    ///
    /// # Returns
    /// - `Ok(Some((value, term, index)))` if key exists and is not deleted
    /// - `Ok(None)` if key does not exist or is deleted
    /// - `Err` on errors
    async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, RaftError>;

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

    async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, RaftError> {
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

#[cfg(test)]
pub struct MockKvBackend;

#[cfg(test)]
impl MockKvBackend {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl KvBackend for MockKvBackend {
    async fn put(
        &self,
        _key: Bytes,
        _value: Bytes,
        _ttl: Option<Duration>,
    ) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(1))
    }

    async fn get(&self, _key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, RaftError> {
        Ok(Some((Bytes::from("mock_value"), Term(1), LogIndex(1))))
    }

    async fn delete(&self, _key: Bytes) -> Result<LogIndex, RaftError> {
        Ok(LogIndex(1))
    }

    fn is_leader(&self) -> bool {
        true
    }

    fn leader(&self) -> Option<NodeId> {
        Some(NodeId::new("mock-leader"))
    }

    fn current_term(&self) -> Term {
        Term(1)
    }

    fn commit_index(&self) -> LogIndex {
        LogIndex(1)
    }
}
