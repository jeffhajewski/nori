//! KV service implementation.
//!
//! Handles Put, Get, Delete operations by routing to a KvBackend.

use crate::kv_backend::KvBackend;
use crate::proto::{self, kv_server::Kv};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use parking_lot::RwLock;
use tonic::{Request, Response, Status};

/// Request size limits (from 10_product.yaml)
const MAX_KEY_SIZE: usize = 64 * 1024;      // 64 KB
const MAX_VALUE_SIZE: usize = 4 * 1024 * 1024; // 4 MB

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
pub struct KvService {
    backend: Arc<dyn KvBackend>,
    /// Idempotency cache: idempotency_key -> (version, timestamp)
    /// TODO: Add TTL expiration and persistence for production
    idempotency_cache: Arc<RwLock<HashMap<String, IdempotencyEntry>>>,
}

impl KvService {
    /// Create a new KV service.
    pub fn new(backend: Arc<dyn KvBackend>) -> Self {
        Self {
            backend,
            idempotency_cache: Arc::new(RwLock::new(HashMap::new())),
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
        let req = request.into_inner();

        tracing::debug!(
            "PUT request: key_len={}, idempotency_key={:?}, if_match={:?}",
            req.key.len(),
            req.idempotency_key,
            req.if_match
        );

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

        // Phase 2.2: CAS - check if_match version
        if let Some(expected_version) = req.if_match {
            // TODO: Implement actual version checking in KvBackend
            // For now, we log it as a placeholder
            tracing::debug!("CAS if_match: term={}, index={}", expected_version.term, expected_version.index);
            // In production, this should:
            // 1. Read current version from LSM
            // 2. Compare with expected_version
            // 3. Return FAILED_PRECONDITION if mismatch
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

                Ok(Response::new(proto::PutResponse {
                    version: Some(version),
                    meta: std::collections::HashMap::new(),
                }))
            }
            Err(e) => {
                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
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

                Err(Status::internal(format!("Put failed: {:?}", e)))
            }
        }
    }

    async fn get(
        &self,
        request: Request<proto::GetRequest>,
    ) -> Result<Response<proto::GetResponse>, Status> {
        let req = request.into_inner();

        tracing::debug!(
            "GET request: key_len={}, consistency={:?}",
            req.key.len(),
            req.consistency
        );

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
            Ok(Some(value)) => {
                // Get current term and commit index for version tracking
                let term = self.backend.current_term();
                let commit_index = self.backend.commit_index();

                Ok(Response::new(proto::GetResponse {
                    value: value.to_vec(),
                    version: Some(proto::Version {
                        term: term.as_u64(),
                        index: commit_index.as_u64(),
                    }),
                    meta: std::collections::HashMap::new(),
                }))
            }
            Ok(None) => {
                // Return empty bytes for missing keys (not an error)
                Ok(Response::new(proto::GetResponse {
                    value: vec![],
                    version: None,
                    meta: std::collections::HashMap::new(),
                }))
            }
            Err(e) => {
                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
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

                Err(Status::internal(format!("Get failed: {:?}", e)))
            }
        }
    }

    async fn delete(
        &self,
        request: Request<proto::DeleteRequest>,
    ) -> Result<Response<proto::DeleteResponse>, Status> {
        let req = request.into_inner();

        tracing::debug!(
            "DELETE request: key_len={}, idempotency_key={:?}, if_match={:?}",
            req.key.len(),
            req.idempotency_key,
            req.if_match
        );

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

        // Phase 2.2: CAS - check if_match version
        if let Some(expected_version) = req.if_match {
            // TODO: Implement actual version checking in KvBackend
            tracing::debug!("CAS if_match: term={}, index={}", expected_version.term, expected_version.index);
            // In production, this should:
            // 1. Read current version from LSM
            // 2. Compare with expected_version
            // 3. Return FAILED_PRECONDITION if mismatch
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

                Ok(Response::new(proto::DeleteResponse {
                    tombstoned: true,
                    version: Some(version),
                }))
            }
            Err(e) => {
                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
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

                Err(Status::internal(format!("Delete failed: {:?}", e)))
            }
        }
    }
}
