//! KV service implementation.
//!
//! Handles Put, Get, Delete operations by routing to ReplicatedLSM.

use crate::proto::{self, kv_server::Kv};
use nori_raft::ReplicatedLSM;
use std::sync::Arc;
use std::time::Duration;
use tonic::{Request, Response, Status};

/// KV service implementation.
///
/// Routes requests to the underlying ReplicatedLSM.
/// Returns NOT_LEADER errors with leader hint in metadata when not leader.
pub struct KvService {
    replicated_lsm: Arc<ReplicatedLSM>,
}

impl KvService {
    /// Create a new KV service.
    pub fn new(replicated_lsm: Arc<ReplicatedLSM>) -> Self {
        Self { replicated_lsm }
    }
}

#[tonic::async_trait]
impl Kv for KvService {
    async fn put(
        &self,
        request: Request<proto::PutRequest>,
    ) -> Result<Response<proto::PutResponse>, Status> {
        let req = request.into_inner();

        tracing::debug!("PUT request: key_len={}", req.key.len());

        // Convert TTL from milliseconds
        let ttl = if req.ttl_ms > 0 {
            Some(Duration::from_millis(req.ttl_ms))
        } else {
            None
        };

        // Attempt the put operation
        match self
            .replicated_lsm
            .replicated_put(bytes::Bytes::from(req.key), bytes::Bytes::from(req.value), ttl)
            .await
        {
            Ok(index) => {
                // Success - return version
                let version = Some(proto::Version {
                    term: 0, // TODO: Get actual term from Raft
                    index: index.0,
                });

                Ok(Response::new(proto::PutResponse {
                    version,
                    meta: std::collections::HashMap::new(),
                }))
            }
            Err(e) => {
                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
                    // Get leader hint if available
                    let leader_hint = self.replicated_lsm.leader()
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

        tracing::debug!("GET request: key_len={}", req.key.len());

        // Attempt the get operation
        match self.replicated_lsm.replicated_get(&req.key).await {
            Ok(Some(value)) => {
                Ok(Response::new(proto::GetResponse {
                    value: value.to_vec(),
                    version: Some(proto::Version {
                        term: 0, // TODO: Get actual term
                        index: 0, // TODO: Get actual index
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
                    let leader_hint = self.replicated_lsm.leader()
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

        tracing::debug!("DELETE request: key_len={}", req.key.len());

        // Attempt the delete operation
        match self
            .replicated_lsm
            .replicated_delete(bytes::Bytes::from(req.key))
            .await
        {
            Ok(_index) => {
                Ok(Response::new(proto::DeleteResponse {
                    tombstoned: true,
                }))
            }
            Err(e) => {
                // Check if it's a NotLeader error
                if format!("{:?}", e).contains("NotLeader") {
                    let leader_hint = self.replicated_lsm.leader()
                        .map(|id| id.to_string())
                        .unwrap_or_default();

                    let mut response = Response::new(proto::DeleteResponse {
                        tombstoned: false,
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
