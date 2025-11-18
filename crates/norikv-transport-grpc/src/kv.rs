//! KV service implementation.
//!
//! Handles Put, Get, Delete operations by routing to a KvBackend.

use crate::kv_backend::KvBackend;
use crate::proto::{self, kv_server::Kv};
use nori_observe::Meter;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use parking_lot::RwLock;
use tonic::{Request, Response, Status};

/// Request size limits (from 10_product.yaml)
const MAX_KEY_SIZE: usize = 1024;           // 1 KB
const MAX_VALUE_SIZE: usize = 1024 * 1024;  // 1 MB hard limit

/// Idempotency cache entry
#[derive(Clone, Debug)]
struct IdempotencyEntry {
    version: proto::Version,
    timestamp: SystemTime,
}

/// KV service implementation.
///
/// Routes requests to the underlying KvBackend (single-shard or multi-shard).
/// Returns NOT_LEADER errors with leader hint in metadata when not leader.
///
/// Features:
/// - Idempotency: Deduplicates retries using idempotency_key
/// - CAS: Compare-and-swap using if_match version
/// - Consistency: Supports "strong", "eventual", "bounded_staleness" reads
/// - Validation: Enforces key/value size limits
/// - Metrics: Tracks operation latency and request counts
pub struct KvService {
    backend: Arc<dyn KvBackend>,
    /// Idempotency cache: idempotency_key -> (version, timestamp)
    ///
    /// Future Enhancement: For production, consider:
    /// - TTL-based eviction (expire entries after N minutes)
    /// - Persistence to disk (survive restarts)
    /// - Size-based eviction (LRU when cache full)
    /// - Per-key expiration based on timestamp field
    ///
    /// Current implementation: Unbounded in-memory HashMap.
    /// Acceptable for development and low-volume production.
    idempotency_cache: Arc<RwLock<HashMap<String, IdempotencyEntry>>>,
    /// Metrics meter for observability
    meter: Arc<dyn Meter>,
}

impl KvService {
    /// Create a new KV service.
    pub fn new(backend: Arc<dyn KvBackend>) -> Self {
        Self::with_meter(backend, Arc::new(nori_observe::NoopMeter))
    }

    /// Create a new KV service with metrics.
    pub fn with_meter(backend: Arc<dyn KvBackend>, meter: Arc<dyn Meter>) -> Self {
        Self {
            backend,
            idempotency_cache: Arc::new(RwLock::new(HashMap::new())),
            meter,
        }
    }

    /// Validate request size limits.
    fn validate_size(&self, key: &[u8], value: Option<&[u8]>) -> Result<(), Status> {
        if key.len() > MAX_KEY_SIZE {
            return Err(Status::invalid_argument(format!(
                "Key size {} exceeds limit {}",
                key.len(),
                MAX_KEY_SIZE
            )));
        }
        if let Some(v) = value {
            if v.len() > MAX_VALUE_SIZE {
                return Err(Status::invalid_argument(format!(
                    "Value size {} exceeds limit {}",
                    v.len(),
                    MAX_VALUE_SIZE
                )));
            }
        }
        Ok(())
    }

    /// Check idempotency cache for duplicate request.
    fn check_idempotency(&self, key: &str) -> Option<proto::Version> {
        if key.is_empty() {
            return None;
        }
        self.idempotency_cache
            .read()
            .get(key)
            .map(|entry| entry.version.clone())
    }

    /// Store result in idempotency cache.
    fn store_idempotency(&self, key: &str, version: proto::Version) {
        if !key.is_empty() {
            self.idempotency_cache.write().insert(
                key.to_string(),
                IdempotencyEntry {
                    version,
                    timestamp: SystemTime::now(),
                },
            );
        }
    }
}

#[tonic::async_trait]
impl Kv for KvService {
    async fn put(
        &self,
        request: Request<proto::PutRequest>,
    ) -> Result<Response<proto::PutResponse>, Status> {
        let start_time = Instant::now();
        let req = request.into_inner();

        tracing::debug!(
            "PUT request: key_len={}, idempotency_key={:?}, if_match={:?}",
            req.key.len(),
            req.idempotency_key,
            req.if_match
        );

        // Record request count
        self.meter.counter("kv_requests_total", &[("operation", "put")]).inc(1);

        // Phase 2.4: Validate request size
        self.validate_size(&req.key, Some(&req.value))?;

        // Phase 2.1: Check idempotency cache
        if let Some(cached_version) = self.check_idempotency(&req.idempotency_key) {
            tracing::debug!("Idempotent retry detected, returning cached result");
            return Ok(Response::new(proto::PutResponse {
                version: Some(cached_version),
                meta: std::collections::HashMap::new(),
            }));
        }

        // Phase 2.2: CAS (Compare-And-Swap) - check if_match version
        //
        // Implements optimistic concurrency control by checking that the current
        // version matches the expected version before applying the write.
        //
        // Note: This uses commit_index as a version proxy. For production use,
        // consider storing per-key version metadata in the LSM layer.
        if let Some(expected_version) = &req.if_match {
            tracing::debug!(
                "CAS if_match provided: term={}, index={}",
                expected_version.term,
                expected_version.index
            );

            // Read current value to check if key exists
            match self.backend.get(&req.key).await {
                Ok(Some(_)) => {
                    // Key exists - verify version matches
                    let current_term = self.backend.current_term();
                    let current_index = self.backend.commit_index();

                    if current_term.as_u64() != expected_version.term
                        || current_index.0 != expected_version.index
                    {
                        tracing::warn!(
                            "CAS version mismatch: expected (term={}, index={}), got (term={}, index={})",
                            expected_version.term,
                            expected_version.index,
                            current_term.as_u64(),
                            current_index.0
                        );

                        self.meter
                            .counter("kv_requests_total", &[("operation", "put"), ("status", "cas_failed")])
                            .inc(1);

                        return Err(Status::failed_precondition(format!(
                            "Version mismatch: expected term={} index={}, got term={} index={}",
                            expected_version.term,
                            expected_version.index,
                            current_term.as_u64(),
                            current_index.0
                        )));
                    }
                }
                Ok(None) => {
                    // Key doesn't exist - CAS with if_match should fail
                    tracing::warn!("CAS attempted on non-existent key");

                    self.meter
                        .counter("kv_requests_total", &[("operation", "put"), ("status", "cas_not_found")])
                        .inc(1);

                    return Err(Status::failed_precondition(
                        "CAS failed: key does not exist",
                    ));
                }
                Err(e) => {
                    tracing::error!("CAS version check failed: {:?}", e);
                    return Err(Status::internal(format!(
                        "Failed to check current version: {:?}",
                        e
                    )));
                }
            }
        }

        // Convert TTL from milliseconds
        let ttl = if req.ttl_ms > 0 {
            Some(Duration::from_millis(req.ttl_ms))
        } else {
            None
        };

        // Attempt the put operation
        match self
            .backend
            .put(bytes::Bytes::from(req.key), bytes::Bytes::from(req.value), ttl)
            .await
        {
            Ok(index) => {
                // Success - return version with actual term and index
                let term = self.backend.current_term();
                let version = proto::Version {
                    term: term.as_u64(),
                    index: index.0,
                };

                // Phase 2.1: Store in idempotency cache
                self.store_idempotency(&req.idempotency_key, version.clone());

                // Record success metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "put"), ("status", "success")])
                    .observe(latency_ms);

                Ok(Response::new(proto::PutResponse {
                    version: Some(version),
                    meta: std::collections::HashMap::new(),
                }))
            }
            Err(e) => {
                // Record error metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;

                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
                    self.meter.histo("kv_request_duration_ms", &[], &[("operation", "put"), ("status", "not_leader")])
                        .observe(latency_ms);

                    // Get leader hint if available
                    let leader_hint = self.backend.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut response = Response::new(proto::PutResponse {
                        version: None,
                        meta: std::collections::HashMap::new(),
                    });

                    // Add leader hint to metadata
                    if !leader_hint.is_empty() {
                        response.metadata_mut().insert(
                            "leader-hint",
                            leader_hint.parse().map_err(|_| {
                                Status::internal("Failed to set leader hint")
                            })?,
                        );
                    }

                    return Err(Status::unavailable("NOT_LEADER"));
                }

                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "put"), ("status", "error")])
                    .observe(latency_ms);

                Err(Status::internal(format!("Put failed: {:?}", e)))
            }
        }
    }

    async fn get(
        &self,
        request: Request<proto::GetRequest>,
    ) -> Result<Response<proto::GetResponse>, Status> {
        let start_time = Instant::now();
        let req = request.into_inner();

        tracing::debug!(
            "GET request: key_len={}, consistency={:?}",
            req.key.len(),
            req.consistency
        );

        // Record request count
        self.meter.counter("kv_requests_total", &[("operation", "get")]).inc(1);

        // Phase 2.4: Validate request size
        self.validate_size(&req.key, None)?;

        // Phase 2.3: Handle consistency level
        // Supported: "strong" (default), "eventual", "bounded_staleness"
        let consistency = if req.consistency.is_empty() {
            "strong"
        } else {
            req.consistency.as_str()
        };

        match consistency {
            "strong" => {
                // Strong consistency: must read from leader
                if !self.backend.is_leader() {
                    let leader_hint = self.backend.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut response = Response::new(proto::GetResponse {
                        value: vec![],
                        version: None,
                        meta: std::collections::HashMap::new(),
                    });

                    if !leader_hint.is_empty() {
                        response.metadata_mut().insert(
                            "leader-hint",
                            leader_hint.parse().map_err(|_| {
                                Status::internal("Failed to set leader hint")
                            })?,
                        );
                    }

                    return Err(Status::unavailable("NOT_LEADER: strong consistency requires leader"));
                }
            }
            "eventual" | "bounded_staleness" => {
                // Eventual/bounded: can read from follower (already supported)
                tracing::debug!("Reading with {} consistency", consistency);
            }
            _ => {
                return Err(Status::invalid_argument(format!(
                    "Unknown consistency level: {}",
                    consistency
                )));
            }
        }

        // Attempt the get operation
        match self.backend.get(&req.key).await {
            Ok(Some((value, term, log_index))) => {
                // Use per-key version from LSM layer (not global backend state)
                // This fixes the multi-shard CAS bug where all keys returned Term(0), LogIndex(0)

                // Record success metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "get"), ("status", "success"), ("result", "found")])
                    .observe(latency_ms);

                Ok(Response::new(proto::GetResponse {
                    value: value.to_vec(),
                    version: Some(proto::Version {
                        term: term.as_u64(),
                        index: log_index.as_u64(),
                    }),
                    meta: std::collections::HashMap::new(),
                }))
            }
            Ok(None) => {
                // Return empty bytes for missing keys (not an error)
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "get"), ("status", "success"), ("result", "not_found")])
                    .observe(latency_ms);

                Ok(Response::new(proto::GetResponse {
                    value: vec![],
                    version: None,
                    meta: std::collections::HashMap::new(),
                }))
            }
            Err(e) => {
                // Record error metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;

                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
                    self.meter.histo("kv_request_duration_ms", &[], &[("operation", "get"), ("status", "not_leader")])
                        .observe(latency_ms);

                    let leader_hint = self.backend.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut response = Response::new(proto::GetResponse {
                        value: vec![],
                        version: None,
                        meta: std::collections::HashMap::new(),
                    });

                    if !leader_hint.is_empty() {
                        response.metadata_mut().insert(
                            "leader-hint",
                            leader_hint.parse().map_err(|_| {
                                Status::internal("Failed to set leader hint")
                            })?,
                        );
                    }

                    return Err(Status::unavailable("NOT_LEADER"));
                }

                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "get"), ("status", "error")])
                    .observe(latency_ms);

                Err(Status::internal(format!("Get failed: {:?}", e)))
            }
        }
    }

    async fn delete(
        &self,
        request: Request<proto::DeleteRequest>,
    ) -> Result<Response<proto::DeleteResponse>, Status> {
        let start_time = Instant::now();
        let req = request.into_inner();

        tracing::debug!(
            "DELETE request: key_len={}, idempotency_key={:?}, if_match={:?}",
            req.key.len(),
            req.idempotency_key,
            req.if_match
        );

        // Record request count
        self.meter.counter("kv_requests_total", &[("operation", "delete")]).inc(1);

        // Phase 2.4: Validate request size
        self.validate_size(&req.key, None)?;

        // Phase 2.1: Check idempotency cache
        if let Some(cached_version) = self.check_idempotency(&req.idempotency_key) {
            tracing::debug!("Idempotent retry detected, returning cached result");
            return Ok(Response::new(proto::DeleteResponse {
                tombstoned: true,
                version: Some(cached_version),
            }));
        }

        // Phase 2.2: CAS (Compare-And-Swap) - check if_match version
        //
        // Future Feature: When if_match is provided, verify the current value's
        // version matches before applying the delete. This enables optimistic
        // concurrency control.
        //
        // Implementation requires:
        // 1. Add get_with_version() method to KvBackend
        // 2. Read current (term, index) from LSM before delete
        // 3. Compare with expected_version
        // 4. Return FAILED_PRECONDITION if mismatch
        //
        // For now, we log the request but don't enforce the check.
        if let Some(expected_version) = req.if_match {
            tracing::warn!(
                "CAS if_match provided (term={}, index={}) but version checking not yet implemented",
                expected_version.term,
                expected_version.index
            );
        }

        // Attempt the delete operation
        match self
            .backend
            .delete(bytes::Bytes::from(req.key))
            .await
        {
            Ok(index) => {
                // Success - return version with actual term and index
                let term = self.backend.current_term();
                let version = proto::Version {
                    term: term.as_u64(),
                    index: index.0,
                };

                // Phase 2.1: Store in idempotency cache
                self.store_idempotency(&req.idempotency_key, version.clone());

                // Record success metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;
                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "delete"), ("status", "success")])
                    .observe(latency_ms);

                Ok(Response::new(proto::DeleteResponse {
                    tombstoned: true,
                    version: Some(version),
                }))
            }
            Err(e) => {
                // Record error metrics
                let latency_ms = start_time.elapsed().as_secs_f64() * 1000.0;

                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
                    self.meter.histo("kv_request_duration_ms", &[], &[("operation", "delete"), ("status", "not_leader")])
                        .observe(latency_ms);

                    let leader_hint = self.backend.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut response = Response::new(proto::DeleteResponse {
                        tombstoned: false,
                        version: None,
                    });

                    if !leader_hint.is_empty() {
                        response.metadata_mut().insert(
                            "leader-hint",
                            leader_hint.parse().map_err(|_| {
                                Status::internal("Failed to set leader hint")
                            })?,
                        );
                    }

                    return Err(Status::unavailable("NOT_LEADER"));
                }

                self.meter.histo("kv_request_duration_ms", &[], &[("operation", "delete"), ("status", "error")])
                    .observe(latency_ms);

                Err(Status::internal(format!("Delete failed: {:?}", e)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use nori_raft::{LogIndex, Term};

    #[test]
    fn test_validate_size_key_too_large() {
        let backend = Arc::new(crate::kv_backend::MockKvBackend::new());
        let service = KvService::new(backend);

        // Create key larger than 1KB limit
        let large_key = vec![0u8; MAX_KEY_SIZE + 1];
        let value = vec![1u8; 100];

        let result = service.validate_size(&large_key, Some(&value));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("Key size"));
        assert!(err.message().contains("exceeds limit"));
    }

    #[test]
    fn test_validate_size_value_too_large() {
        let backend = Arc::new(crate::kv_backend::MockKvBackend::new());
        let service = KvService::new(backend);

        let key = vec![0u8; 100];
        // Create value larger than 1MB limit
        let large_value = vec![1u8; MAX_VALUE_SIZE + 1];

        let result = service.validate_size(&key, Some(&large_value));
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("Value size"));
        assert!(err.message().contains("exceeds limit"));
    }

    #[test]
    fn test_validate_size_at_limits() {
        let backend = Arc::new(crate::kv_backend::MockKvBackend::new());
        let service = KvService::new(backend);

        // Exactly at limits should succeed
        let max_key = vec![0u8; MAX_KEY_SIZE];
        let max_value = vec![1u8; MAX_VALUE_SIZE];

        let result = service.validate_size(&max_key, Some(&max_value));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_size_under_limits() {
        let backend = Arc::new(crate::kv_backend::MockKvBackend::new());
        let service = KvService::new(backend);

        let key = vec![0u8; 100];
        let value = vec![1u8; 1000];

        let result = service.validate_size(&key, Some(&value));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_size_no_value() {
        let backend = Arc::new(crate::kv_backend::MockKvBackend::new());
        let service = KvService::new(backend);

        let key = vec![0u8; 100];

        // For DELETE operations (no value)
        let result = service.validate_size(&key, None);
        assert!(result.is_ok());
    }

    // Stateful mock backend for CAS testing
    struct StatefulMockBackend {
        term: std::sync::RwLock<Term>,
        commit_index: std::sync::RwLock<LogIndex>,
        store: std::sync::RwLock<std::collections::HashMap<Vec<u8>, Bytes>>,
    }

    impl StatefulMockBackend {
        fn new() -> Self {
            Self {
                term: std::sync::RwLock::new(Term(1)),
                commit_index: std::sync::RwLock::new(LogIndex(1)),
                store: std::sync::RwLock::new(std::collections::HashMap::new()),
            }
        }

        fn set_version(&self, term: u64, index: u64) {
            *self.term.write().unwrap() = Term(term);
            *self.commit_index.write().unwrap() = LogIndex(index);
        }
    }

    #[async_trait::async_trait]
    impl crate::kv_backend::KvBackend for StatefulMockBackend {
        async fn put(
            &self,
            key: Bytes,
            value: Bytes,
            _ttl: Option<Duration>,
        ) -> Result<LogIndex, nori_raft::RaftError> {
            self.store.write().unwrap().insert(key.to_vec(), value);
            // Increment commit index on write
            let mut index = self.commit_index.write().unwrap();
            *index = LogIndex(index.0 + 1);
            Ok(*index)
        }

        async fn get(&self, key: &[u8]) -> Result<Option<(Bytes, Term, LogIndex)>, nori_raft::RaftError> {
            Ok(self.store.read().unwrap().get(key).cloned().map(|value| {
                // Return current term and commit index as version for mock
                (value, *self.term.read().unwrap(), *self.commit_index.read().unwrap())
            }))
        }

        async fn delete(&self, key: Bytes) -> Result<LogIndex, nori_raft::RaftError> {
            self.store.write().unwrap().remove(&key.to_vec());
            let mut index = self.commit_index.write().unwrap();
            *index = LogIndex(index.0 + 1);
            Ok(*index)
        }

        fn is_leader(&self) -> bool {
            true
        }

        fn leader(&self) -> Option<nori_raft::NodeId> {
            Some(nori_raft::NodeId::new("test-leader"))
        }

        fn current_term(&self) -> Term {
            *self.term.read().unwrap()
        }

        fn commit_index(&self) -> LogIndex {
            *self.commit_index.read().unwrap()
        }
    }

    #[tokio::test]
    async fn test_cas_success_when_version_matches() {
        let backend = Arc::new(StatefulMockBackend::new());
        let service = KvService::new(backend.clone());

        // First, put a key-value pair
        let key = b"test-key".to_vec();
        let value1 = b"value1".to_vec();

        let put_req = proto::PutRequest {
            key: key.clone(),
            value: value1,
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: None,
        };

        let response = service.put(Request::new(put_req)).await.unwrap();
        let version = response.into_inner().version.unwrap();

        // Now do a CAS with matching version
        let value2 = b"value2".to_vec();
        let cas_req = proto::PutRequest {
            key: key.clone(),
            value: value2.clone(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: Some(proto::Version {
                term: version.term,
                index: version.index,
            }),
        };

        let result = service.put(Request::new(cas_req)).await;
        assert!(result.is_ok(), "CAS should succeed when version matches");

        // Verify the value was updated
        let stored_value = backend.get(&key).await.unwrap().unwrap();
        assert_eq!(stored_value, Bytes::from(value2));
    }

    #[tokio::test]
    async fn test_cas_fail_when_version_mismatches() {
        let backend = Arc::new(StatefulMockBackend::new());
        let service = KvService::new(backend.clone());

        // First, put a key-value pair
        let key = b"test-key".to_vec();
        let value1 = b"value1".to_vec();

        let put_req = proto::PutRequest {
            key: key.clone(),
            value: value1.clone(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: None,
        };

        service.put(Request::new(put_req)).await.unwrap();

        // Simulate version change (another write happened)
        backend.set_version(1, 100);

        // Now try CAS with old version
        let value2 = b"value2".to_vec();
        let cas_req = proto::PutRequest {
            key: key.clone(),
            value: value2.clone(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: Some(proto::Version {
                term: 1,
                index: 2, // Old version, current is 100
            }),
        };

        let result = service.put(Request::new(cas_req)).await;
        assert!(result.is_err(), "CAS should fail when version doesn't match");

        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("Version mismatch"));

        // Verify the value was NOT updated
        let stored_value = backend.get(&key).await.unwrap().unwrap();
        assert_eq!(stored_value, Bytes::from(value1));
    }

    #[tokio::test]
    async fn test_cas_fail_when_key_not_exists() {
        let backend = Arc::new(StatefulMockBackend::new());
        let service = KvService::new(backend);

        // Try CAS on non-existent key
        let key = b"nonexistent-key".to_vec();
        let value = b"value".to_vec();

        let cas_req = proto::PutRequest {
            key,
            value,
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: Some(proto::Version {
                term: 1,
                index: 1,
            }),
        };

        let result = service.put(Request::new(cas_req)).await;
        assert!(result.is_err(), "CAS should fail when key doesn't exist");

        let err = result.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("key does not exist"));
    }

    #[tokio::test]
    async fn test_put_without_cas_on_existing_key() {
        let backend = Arc::new(StatefulMockBackend::new());
        let service = KvService::new(backend.clone());

        // First, put a key-value pair
        let key = b"test-key".to_vec();
        let value1 = b"value1".to_vec();

        let put_req = proto::PutRequest {
            key: key.clone(),
            value: value1,
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: None,
        };

        service.put(Request::new(put_req)).await.unwrap();

        // Update without CAS (if_match=None) should succeed
        let value2 = b"value2".to_vec();
        let update_req = proto::PutRequest {
            key: key.clone(),
            value: value2.clone(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: None,
        };

        let result = service.put(Request::new(update_req)).await;
        assert!(result.is_ok(), "PUT without CAS should succeed on existing key");

        // Verify the value was updated
        let stored_value = backend.get(&key).await.unwrap().unwrap();
        assert_eq!(stored_value, Bytes::from(value2));
    }
}
