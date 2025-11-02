//! Health check service for monitoring and load balancers.
//!
//! Provides a simple health check utility for the server.

use std::sync::Arc;
use nori_raft::ReplicatedLSM;

/// Health check response
#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthStatus {
    /// Node status: "healthy", "unhealthy", "starting"
    pub status: String,
    /// Whether this node is the Raft leader
    pub is_leader: bool,
    /// Current Raft term (if available)
    pub term: u64,
    /// Additional details
    pub details: String,
}

/// Health check service
pub struct HealthService {
    replicated_lsm: Arc<ReplicatedLSM>,
}

impl HealthService {
    /// Create a new health service
    pub fn new(replicated_lsm: Arc<ReplicatedLSM>) -> Self {
        Self { replicated_lsm }
    }

    /// Check health status
    pub fn check_health(&self) -> HealthStatus {
        let is_leader = self.replicated_lsm.is_leader();

        HealthStatus {
            status: "healthy".to_string(),
            is_leader,
            term: 0, // TODO: Get actual term when available
            details: format!(
                "Node is {}",
                if is_leader { "leader" } else { "follower" }
            ),
        }
    }
}
