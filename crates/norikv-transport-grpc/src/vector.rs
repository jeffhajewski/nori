//! Vector service implementation.
//!
//! Handles vector operations by routing to a VectorBackend.

use crate::proto::{self, vector_server::Vector};
use crate::vector_backend::{self, VectorBackend};
use nori_observe::Meter;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime};
use tonic::{Request, Response, Status};

/// Request size limits
const MAX_NAMESPACE_SIZE: usize = 256;
const MAX_ID_SIZE: usize = 512;
const MAX_VECTOR_DIMENSIONS: usize = 4096;

/// Idempotency cache entry
#[derive(Clone, Debug)]
struct IdempotencyEntry {
    success: bool,
    /// Timestamp for future TTL-based eviction
    #[allow(dead_code)]
    timestamp: SystemTime,
}

/// Vector service implementation.
///
/// Routes requests to the underlying VectorBackend.
/// Returns NOT_LEADER errors with leader hint in metadata when not leader.
pub struct VectorService {
    backend: Arc<dyn VectorBackend>,
    /// Idempotency cache: idempotency_key -> (success, timestamp)
    idempotency_cache: Arc<RwLock<HashMap<String, IdempotencyEntry>>>,
    /// Metrics meter for observability
    meter: Arc<dyn Meter>,
}

impl VectorService {
    /// Create a new Vector service.
    pub fn new(backend: Arc<dyn VectorBackend>) -> Self {
        Self::with_meter(backend, Arc::new(nori_observe::NoopMeter))
    }

    /// Create a new Vector service with metrics.
    pub fn with_meter(backend: Arc<dyn VectorBackend>, meter: Arc<dyn Meter>) -> Self {
        Self {
            backend,
            idempotency_cache: Arc::new(RwLock::new(HashMap::new())),
            meter,
        }
    }

    /// Validate namespace.
    #[allow(clippy::result_large_err)]
    fn validate_namespace(&self, namespace: &str) -> Result<(), Status> {
        if namespace.is_empty() {
            return Err(Status::invalid_argument("Namespace cannot be empty"));
        }
        if namespace.len() > MAX_NAMESPACE_SIZE {
            return Err(Status::invalid_argument(format!(
                "Namespace size {} exceeds limit {}",
                namespace.len(),
                MAX_NAMESPACE_SIZE
            )));
        }
        Ok(())
    }

    /// Validate vector ID.
    #[allow(clippy::result_large_err)]
    fn validate_id(&self, id: &str) -> Result<(), Status> {
        if id.is_empty() {
            return Err(Status::invalid_argument("Vector ID cannot be empty"));
        }
        if id.len() > MAX_ID_SIZE {
            return Err(Status::invalid_argument(format!(
                "Vector ID size {} exceeds limit {}",
                id.len(),
                MAX_ID_SIZE
            )));
        }
        Ok(())
    }

    /// Validate vector dimensions.
    #[allow(clippy::result_large_err)]
    fn validate_vector(&self, vector: &[f32], expected_dims: Option<usize>) -> Result<(), Status> {
        if vector.is_empty() {
            return Err(Status::invalid_argument("Vector cannot be empty"));
        }
        if vector.len() > MAX_VECTOR_DIMENSIONS {
            return Err(Status::invalid_argument(format!(
                "Vector dimensions {} exceeds limit {}",
                vector.len(),
                MAX_VECTOR_DIMENSIONS
            )));
        }
        if let Some(dims) = expected_dims {
            if vector.len() != dims {
                return Err(Status::invalid_argument(format!(
                    "Vector dimensions {} does not match expected {}",
                    vector.len(),
                    dims
                )));
            }
        }
        Ok(())
    }

    /// Check idempotency cache for duplicate request.
    fn check_idempotency(&self, key: &str) -> Option<bool> {
        if key.is_empty() {
            return None;
        }
        self.idempotency_cache
            .read()
            .get(key)
            .map(|entry| entry.success)
    }

    /// Store result in idempotency cache.
    fn store_idempotency(&self, key: &str, success: bool) {
        if key.is_empty() {
            return;
        }
        self.idempotency_cache.write().insert(
            key.to_string(),
            IdempotencyEntry {
                success,
                timestamp: SystemTime::now(),
            },
        );
    }

    /// Check if backend is leader, return error with hint if not.
    #[allow(clippy::result_large_err)]
    fn check_leader(&self) -> Result<(), Status> {
        if !self.backend.is_leader() {
            let mut status = Status::failed_precondition("Not leader");
            if let Some(leader) = self.backend.leader() {
                // Add leader hint to metadata
                let metadata = status.metadata_mut();
                metadata.insert("x-norikv-leader", leader.as_str().parse().unwrap());
            }
            return Err(status);
        }
        Ok(())
    }

    /// Check consistency level and enforce leader requirement for strong consistency.
    ///
    /// Supported levels:
    /// - "strong" (default): Must read from leader
    /// - "eventual": Can read from any node
    /// - "bounded_staleness": Can read from any node (same as eventual for now)
    #[allow(clippy::result_large_err)]
    fn check_consistency(&self, consistency: &str) -> Result<(), Status> {
        let level = if consistency.is_empty() {
            "strong"
        } else {
            consistency
        };

        match level {
            "strong" => {
                // Strong consistency: must read from leader
                if !self.backend.is_leader() {
                    let leader_hint = self.backend.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    if !leader_hint.is_empty() {
                        let mut status = Status::unavailable(
                            "NOT_LEADER: strong consistency requires leader"
                        );
                        let metadata = status.metadata_mut();
                        metadata.insert("x-norikv-leader", leader_hint.parse().unwrap());
                        return Err(status);
                    }

                    return Err(Status::unavailable(
                        "NOT_LEADER: strong consistency requires leader"
                    ));
                }
            }
            "eventual" | "bounded_staleness" => {
                // Eventual/bounded: can read from follower
                tracing::debug!("Vector read with {} consistency", level);
            }
            _ => {
                return Err(Status::invalid_argument(format!(
                    "Unknown consistency level: {}",
                    level
                )));
            }
        }
        Ok(())
    }

    /// Convert proto distance function to backend type.
    #[allow(clippy::result_large_err)]
    fn convert_distance(
        proto_dist: proto::DistanceFunction,
    ) -> Result<vector_backend::DistanceFunction, Status> {
        match proto_dist {
            proto::DistanceFunction::Euclidean => Ok(vector_backend::DistanceFunction::Euclidean),
            proto::DistanceFunction::Cosine => Ok(vector_backend::DistanceFunction::Cosine),
            proto::DistanceFunction::InnerProduct => {
                Ok(vector_backend::DistanceFunction::InnerProduct)
            }
            proto::DistanceFunction::Unspecified => Err(Status::invalid_argument(
                "Distance function must be specified",
            )),
        }
    }

    /// Convert proto index type to backend type.
    #[allow(clippy::result_large_err)]
    fn convert_index_type(
        proto_type: proto::VectorIndexType,
    ) -> Result<vector_backend::VectorIndexType, Status> {
        match proto_type {
            proto::VectorIndexType::BruteForce => Ok(vector_backend::VectorIndexType::BruteForce),
            proto::VectorIndexType::Hnsw => Ok(vector_backend::VectorIndexType::Hnsw),
            proto::VectorIndexType::Unspecified => {
                // Default to HNSW
                Ok(vector_backend::VectorIndexType::Hnsw)
            }
        }
    }
}

#[tonic::async_trait]
impl Vector for VectorService {
    async fn create_index(
        &self,
        request: Request<proto::CreateVectorIndexRequest>,
    ) -> Result<Response<proto::CreateVectorIndexResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;
        if req.dimensions == 0 || req.dimensions > MAX_VECTOR_DIMENSIONS as u32 {
            return Err(Status::invalid_argument(format!(
                "Dimensions must be between 1 and {}",
                MAX_VECTOR_DIMENSIONS
            )));
        }

        // Check idempotency
        if let Some(success) = self.check_idempotency(&req.idempotency_key) {
            self.meter.counter("vector_idempotency_hits", &[("operation", "create_index")]).inc(1);
            return Ok(Response::new(proto::CreateVectorIndexResponse {
                created: success,
                meta: HashMap::new(),
            }));
        }

        // Check leader
        self.check_leader()?;

        // Convert types
        let distance = Self::convert_distance(proto::DistanceFunction::try_from(req.distance).unwrap_or(proto::DistanceFunction::Unspecified))?;
        let index_type = Self::convert_index_type(proto::VectorIndexType::try_from(req.index_type).unwrap_or(proto::VectorIndexType::Unspecified))?;

        // Execute
        let result = self
            .backend
            .create_index(
                req.namespace,
                req.dimensions as usize,
                distance,
                index_type,
            )
            .await;

        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

        match result {
            Ok(created) => {
                self.store_idempotency(&req.idempotency_key, created);
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "create_index"), ("status", "success")])
                    .observe(duration_ms);
                Ok(Response::new(proto::CreateVectorIndexResponse {
                    created,
                    meta: HashMap::new(),
                }))
            }
            Err(e) => {
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "create_index"), ("status", "error")])
                    .observe(duration_ms);
                Err(Status::internal(format!("Failed to create index: {}", e)))
            }
        }
    }

    async fn drop_index(
        &self,
        request: Request<proto::DropVectorIndexRequest>,
    ) -> Result<Response<proto::DropVectorIndexResponse>, Status> {
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;

        // Check idempotency
        if let Some(success) = self.check_idempotency(&req.idempotency_key) {
            return Ok(Response::new(proto::DropVectorIndexResponse {
                dropped: success,
            }));
        }

        // Check leader
        self.check_leader()?;

        // Execute
        let result = self.backend.drop_index(req.namespace).await;

        match result {
            Ok(dropped) => {
                self.store_idempotency(&req.idempotency_key, dropped);
                Ok(Response::new(proto::DropVectorIndexResponse { dropped }))
            }
            Err(e) => Err(Status::internal(format!("Failed to drop index: {}", e))),
        }
    }

    async fn insert(
        &self,
        request: Request<proto::VectorInsertRequest>,
    ) -> Result<Response<proto::VectorInsertResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;
        self.validate_id(&req.id)?;
        self.validate_vector(&req.vector, None)?;

        // Check idempotency
        if let Some(success) = self.check_idempotency(&req.idempotency_key) {
            self.meter.counter("vector_idempotency_hits", &[("operation", "insert")]).inc(1);
            return Ok(Response::new(proto::VectorInsertResponse {
                inserted: success,
                version: Some(proto::Version {
                    term: self.backend.current_term().0,
                    index: self.backend.commit_index().0,
                }),
            }));
        }

        // Check leader
        self.check_leader()?;

        // Execute
        let result = self
            .backend
            .insert(req.namespace, req.id, req.vector)
            .await;

        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

        match result {
            Ok(index) => {
                self.store_idempotency(&req.idempotency_key, true);
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "insert"), ("status", "success")])
                    .observe(duration_ms);
                Ok(Response::new(proto::VectorInsertResponse {
                    inserted: true,
                    version: Some(proto::Version {
                        term: self.backend.current_term().0,
                        index: index.0,
                    }),
                }))
            }
            Err(e) => {
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "insert"), ("status", "error")])
                    .observe(duration_ms);
                Err(Status::internal(format!("Failed to insert vector: {}", e)))
            }
        }
    }

    async fn delete(
        &self,
        request: Request<proto::VectorDeleteRequest>,
    ) -> Result<Response<proto::VectorDeleteResponse>, Status> {
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;
        self.validate_id(&req.id)?;

        // Check idempotency
        if let Some(success) = self.check_idempotency(&req.idempotency_key) {
            return Ok(Response::new(proto::VectorDeleteResponse {
                deleted: success,
            }));
        }

        // Check leader
        self.check_leader()?;

        // Execute
        let result = self.backend.delete(req.namespace, req.id).await;

        match result {
            Ok(deleted) => {
                self.store_idempotency(&req.idempotency_key, deleted);
                Ok(Response::new(proto::VectorDeleteResponse { deleted }))
            }
            Err(e) => Err(Status::internal(format!("Failed to delete vector: {}", e))),
        }
    }

    async fn search(
        &self,
        request: Request<proto::VectorSearchRequest>,
    ) -> Result<Response<proto::VectorSearchResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;
        self.validate_vector(&req.query, None)?;
        if req.k == 0 {
            return Err(Status::invalid_argument("k must be greater than 0"));
        }

        // Check consistency level (default: strong)
        self.check_consistency(&req.consistency)?;

        // Execute
        let result = self
            .backend
            .search(&req.namespace, &req.query, req.k as usize, req.include_vectors)
            .await;

        let duration_us = start.elapsed().as_micros() as u64;
        let duration_ms = duration_us as f64 / 1000.0;

        match result {
            Ok(matches) => {
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "search"), ("status", "success")])
                    .observe(duration_ms);

                let proto_matches: Vec<proto::VectorMatch> = matches
                    .into_iter()
                    .map(|m| proto::VectorMatch {
                        id: m.id,
                        distance: m.distance,
                        vector: m.vector.unwrap_or_default(),
                    })
                    .collect();

                Ok(Response::new(proto::VectorSearchResponse {
                    matches: proto_matches,
                    search_time_us: duration_us,
                }))
            }
            Err(e) => {
                self.meter.histo("vector_request_duration_ms", &[], &[("operation", "search"), ("status", "error")])
                    .observe(duration_ms);
                Err(Status::internal(format!("Failed to search: {}", e)))
            }
        }
    }

    async fn get(
        &self,
        request: Request<proto::VectorGetRequest>,
    ) -> Result<Response<proto::VectorGetResponse>, Status> {
        let req = request.into_inner();

        // Validate
        self.validate_namespace(&req.namespace)?;
        self.validate_id(&req.id)?;

        // Check consistency level (default: strong)
        self.check_consistency(&req.consistency)?;

        // Execute
        let result = self.backend.get(&req.namespace, &req.id).await;

        match result {
            Ok(Some(vector)) => Ok(Response::new(proto::VectorGetResponse {
                vector,
                found: true,
            })),
            Ok(None) => Ok(Response::new(proto::VectorGetResponse {
                vector: Vec::new(),
                found: false,
            })),
            Err(e) => Err(Status::internal(format!("Failed to get vector: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_backend::MockVectorBackend;

    #[tokio::test]
    async fn test_create_index() {
        let backend = Arc::new(MockVectorBackend::new());
        let service = VectorService::new(backend);

        let request = Request::new(proto::CreateVectorIndexRequest {
            namespace: "test".to_string(),
            dimensions: 128,
            distance: proto::DistanceFunction::Cosine as i32,
            index_type: proto::VectorIndexType::Hnsw as i32,
            idempotency_key: "".to_string(),
        });

        let response = service.create_index(request).await.unwrap();
        assert!(response.into_inner().created);
    }

    #[tokio::test]
    async fn test_insert_and_search() {
        let backend = Arc::new(MockVectorBackend::new());
        let service = VectorService::new(backend);

        // Insert
        let insert_req = Request::new(proto::VectorInsertRequest {
            namespace: "test".to_string(),
            id: "vec1".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            idempotency_key: "".to_string(),
        });

        let response = service.insert(insert_req).await.unwrap();
        assert!(response.into_inner().inserted);

        // Search
        let search_req = Request::new(proto::VectorSearchRequest {
            namespace: "test".to_string(),
            query: vec![0.1, 0.2, 0.3],
            k: 5,
            include_vectors: false,
            consistency: "".to_string(), // Default to strong
        });

        let response = service.search(search_req).await.unwrap();
        let inner = response.into_inner();
        assert!(!inner.matches.is_empty());
        assert!(inner.search_time_us > 0);
    }

    #[tokio::test]
    async fn test_validation_errors() {
        let backend = Arc::new(MockVectorBackend::new());
        let service = VectorService::new(backend);

        // Empty namespace
        let request = Request::new(proto::VectorInsertRequest {
            namespace: "".to_string(),
            id: "vec1".to_string(),
            vector: vec![0.1],
            idempotency_key: "".to_string(),
        });
        assert!(service.insert(request).await.is_err());

        // Empty ID
        let request = Request::new(proto::VectorInsertRequest {
            namespace: "test".to_string(),
            id: "".to_string(),
            vector: vec![0.1],
            idempotency_key: "".to_string(),
        });
        assert!(service.insert(request).await.is_err());

        // Empty vector
        let request = Request::new(proto::VectorInsertRequest {
            namespace: "test".to_string(),
            id: "vec1".to_string(),
            vector: vec![],
            idempotency_key: "".to_string(),
        });
        assert!(service.insert(request).await.is_err());
    }

    #[tokio::test]
    async fn test_idempotency() {
        let backend = Arc::new(MockVectorBackend::new());
        let service = VectorService::new(backend);

        let idempotency_key = "unique-key-123".to_string();

        // First request
        let request = Request::new(proto::VectorInsertRequest {
            namespace: "test".to_string(),
            id: "vec1".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            idempotency_key: idempotency_key.clone(),
        });
        let response1 = service.insert(request).await.unwrap();
        assert!(response1.into_inner().inserted);

        // Second request with same idempotency key should return cached result
        let request = Request::new(proto::VectorInsertRequest {
            namespace: "test".to_string(),
            id: "vec1".to_string(),
            vector: vec![0.1, 0.2, 0.3],
            idempotency_key,
        });
        let response2 = service.insert(request).await.unwrap();
        assert!(response2.into_inner().inserted);
    }
}
