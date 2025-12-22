//! Multi-shard backend for distributing keys across shards.
//!
//! Routes requests based on key hash using consistent hashing.

use crate::shard_manager::ShardManager;
use bytes::Bytes;
use nori_raft::{LogIndex, NodeId, RaftError, Term};
use norikv_placement::xxhash64;
use norikv_transport_grpc::KvBackend;
use std::sync::Arc;
use std::time::Duration;

/// Multi-shard backend that routes keys to shards based on consistent hashing.
///
/// Architecture:
/// 1. Hash key using xxhash64(seed=0)
/// 2. Map hash to shard using jump_consistent_hash (via ShardManager)
/// 3. Get or create shard lazily
/// 4. Forward operation to target shard's ReplicatedLSM
pub struct MultiShardBackend {
    /// Shard manager (manages 1024 Raft groups)
    shard_manager: Arc<ShardManager>,

    /// Total number of shards (typically 1024)
    total_shards: u32,
}

impl MultiShardBackend {
    /// Create a new multi-shard backend.
    pub fn new(shard_manager: Arc<ShardManager>, total_shards: u32) -> Self {
        Self {
            shard_manager,
            total_shards,
        }
    }

    /// Compute the shard ID for a key.
    fn get_shard_for_key(&self, key: &[u8]) -> u32 {
        let hash = xxhash64(key);
        norikv_placement::jump_consistent_hash(hash, self.total_shards)
    }
}

#[async_trait::async_trait]
impl KvBackend for MultiShardBackend {
    async fn put(
        &self,
        key: Bytes,
        value: Bytes,
        ttl: Option<Duration>,
    ) -> Result<LogIndex, RaftError> {
        // Determine target shard
        let shard_id = self.get_shard_for_key(&key);

        tracing::debug!("PUT key_len={} -> shard {}", key.len(), shard_id);

        // Get or create the shard
        let shard = self
            .shard_manager
            .get_or_create_shard(shard_id)
            .await
            .map_err(|_e| RaftError::NotLeader { leader: None })?;

        // Forward to shard
        shard.replicated_put(key, value, ttl).await
    }

    async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, RaftError> {
        // Determine target shard
        let shard_id = self.get_shard_for_key(key);

        tracing::debug!("GET key_len={} -> shard {}", key.len(), shard_id);

        // Get or create the shard
        let shard = self
            .shard_manager
            .get_or_create_shard(shard_id)
            .await
            .map_err(|_e| RaftError::NotLeader { leader: None })?;

        // Forward to shard - now returns (value, term, index)
        shard.replicated_get(key).await
    }

    async fn delete(&self, key: Bytes) -> Result<LogIndex, RaftError> {
        // Determine target shard
        let shard_id = self.get_shard_for_key(&key);

        tracing::debug!("DELETE key_len={} -> shard {}", key.len(), shard_id);

        // Get or create the shard
        let shard = self
            .shard_manager
            .get_or_create_shard(shard_id)
            .await
            .map_err(|_e| RaftError::NotLeader { leader: None })?;

        // Forward to shard
        shard.replicated_delete(key).await
    }

    fn is_leader(&self) -> bool {
        // Note: In multi-shard mode, leadership is per-shard, not global.
        // This method is effectively meaningless for a multi-shard backend.
        //
        // We return `true` to allow requests through - the actual leadership
        // check happens during the operation when we route to the target shard.
        // If the target shard is not leader, the operation will fail with
        // RaftError::NotLeader and the client will retry.
        //
        // For accurate cluster health, use HealthChecker which queries each
        // shard individually (see apps/norikv-server/src/health.rs).
        true
    }

    fn leader(&self) -> Option<NodeId> {
        // Note: In multi-shard mode, leadership is per-shard, not global.
        // Each of the 1024 shards may have a different leader.
        //
        // We return None to indicate "unknown global leader" - callers should
        // not rely on this method in multi-shard mode.
        //
        // For accurate leadership tracking per shard, use:
        // - shard_manager.shard_info(shard_id) to get leader for specific shard
        // - ClusterView to get aggregated cluster topology
        None
    }

    fn current_term(&self) -> Term {
        // Note: In multi-shard mode, each shard has its own independent term.
        // There is no meaningful "global term" across all 1024 shards.
        //
        // We return Term(0) as a placeholder. Callers should not rely on
        // this method in multi-shard mode.
        //
        // For per-shard term tracking, query individual shards via:
        //   shard_manager.get_shard(shard_id).raft().current_term()
        Term(0)
    }

    fn commit_index(&self) -> LogIndex {
        // Note: In multi-shard mode, each shard has its own independent commit index.
        // There is no meaningful "global commit index" across all 1024 shards.
        //
        // We return LogIndex(0) as a placeholder. Callers should not rely on
        // this method in multi-shard mode.
        //
        // For per-shard commit index tracking, query individual shards via:
        //   shard_manager.get_shard(shard_id).raft().commit_index()
        LogIndex(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ClusterConfig, ServerConfig, TelemetryConfig};
    use nori_lsm::ATLLConfig;
    use nori_raft::{ConfigEntry, RaftConfig};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_key_routing() {
        let temp_dir = tempfile::tempdir().unwrap();

        let config = ServerConfig {
            node_id: "test-node".to_string(),
            rpc_addr: "127.0.0.1:7447".to_string(),
            http_addr: "127.0.0.1:8447".to_string(),
            data_dir: temp_dir.path().to_path_buf(),
            cluster: ClusterConfig {
                seed_nodes: vec![],
                total_shards: 8,
                replication_factor: 3,
            },
            telemetry: TelemetryConfig::default(),
        };

        let raft_config = RaftConfig::default();
        let lsm_config = ATLLConfig::default();

        let node_id = NodeId::new("test-node");
        let transport = Arc::new(nori_raft::transport::InMemoryTransport::new(
            node_id.clone(),
            HashMap::new(),
        ));
        let initial_config = ConfigEntry::Single(vec![node_id]);

        let shard_manager = Arc::new(ShardManager::new(
            config.clone(),
            raft_config,
            lsm_config,
            transport,
            initial_config,
        ));

        let backend = MultiShardBackend::new(shard_manager, 8);

        // Test that same key always routes to same shard
        let shard1 = backend.get_shard_for_key(b"test-key");
        let shard2 = backend.get_shard_for_key(b"test-key");
        assert_eq!(shard1, shard2);

        // Test that shard is in valid range
        assert!(shard1 < 8);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_multi_shard_put_get() {
        let temp_dir = tempfile::tempdir().unwrap();

        let config = ServerConfig {
            node_id: "test-node".to_string(),
            rpc_addr: "127.0.0.1:7447".to_string(),
            http_addr: "127.0.0.1:8447".to_string(),
            data_dir: temp_dir.path().to_path_buf(),
            cluster: ClusterConfig {
                seed_nodes: vec![],
                total_shards: 4,
                replication_factor: 3,
            },
            telemetry: TelemetryConfig::default(),
        };

        let raft_config = RaftConfig::default();
        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = temp_dir.path().to_path_buf();

        let node_id = NodeId::new("test-node");
        let transport = Arc::new(nori_raft::transport::InMemoryTransport::new(
            node_id.clone(),
            HashMap::new(),
        ));
        let initial_config = ConfigEntry::Single(vec![node_id]);

        let shard_manager = Arc::new(ShardManager::new(
            config.clone(),
            raft_config,
            lsm_config,
            transport,
            initial_config,
        ));

        let backend = MultiShardBackend::new(shard_manager.clone(), 4);

        // Determine which shard the test key will go to and pre-create it
        let key = Bytes::from("test-key");
        let value = Bytes::from("test-value");
        let shard_id = backend.get_shard_for_key(&key);

        // Create and start the shard
        let _shard = shard_manager.get_or_create_shard(shard_id).await.unwrap();

        // Wait for leader election with retry
        use std::time::Duration;
        tokio::time::sleep(Duration::from_millis(1500)).await;

        // Put with retry on NotLeader errors
        let mut retries = 0;
        loop {
            match backend.put(key.clone(), value.clone(), None).await {
                Ok(_) => break,
                Err(e) if retries < 5 => {
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                Err(e) => panic!("PUT failed after {} retries: {:?}", retries, e),
            }
        }

        // Wait a bit for the write to apply
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Get with retry on NotLeader errors
        let mut get_retries = 0;
        let retrieved = loop {
            match backend.get(&key).await {
                Ok(v) => break v,
                Err(e) if get_retries < 3 => {
                    get_retries += 1;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
                Err(e) => panic!("GET failed after {} retries: {:?}", get_retries, e),
            }
        };
        assert_eq!(retrieved.map(|(v, _, _)| v), Some(value));
    }
}
