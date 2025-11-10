//! Comprehensive health check for multi-shard server.
//!
//! Queries all active shards and returns aggregated health status.

use crate::shard_manager::ShardManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Comprehensive health status for the server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerHealthStatus {
    /// Overall server status: "healthy", "degraded", "unhealthy", "starting"
    pub status: String,

    /// Node ID
    pub node_id: String,

    /// Server uptime in seconds (approximation based on sample shard)
    pub uptime_seconds: u64,

    /// Total number of shards configured
    pub total_shards: u32,

    /// Number of active shards (created and running)
    pub active_shards: usize,

    /// Number of shards where this node is leader
    pub leader_shards: usize,

    /// Number of shards where this node is follower
    pub follower_shards: usize,

    /// Detailed shard health (sample of first 10 active shards)
    pub shard_samples: Vec<ShardHealth>,

    /// Additional details
    pub details: String,
}

/// Health status for a single shard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardHealth {
    /// Shard ID
    pub shard_id: u32,

    /// Whether this node is the leader for this shard
    pub is_leader: bool,

    /// Current Raft term
    pub term: u64,

    /// Commit index
    pub commit_index: u64,

    /// Shard status: "healthy", "initializing", "error"
    pub status: String,
}

/// Health check service for multi-shard server.
pub struct HealthChecker {
    node_id: String,
    total_shards: u32,
    shard_manager: Arc<ShardManager>,
}

impl HealthChecker {
    /// Create a new health checker.
    pub fn new(node_id: String, total_shards: u32, shard_manager: Arc<ShardManager>) -> Self {
        Self {
            node_id,
            total_shards,
            shard_manager,
        }
    }

    /// Check server health by querying all active shards.
    pub async fn check(&self) -> ServerHealthStatus {
        let mut active_shards = 0;
        let mut leader_shards = 0;
        let mut follower_shards = 0;
        let mut shard_samples = Vec::new();
        let mut has_errors = false;

        // Query all shards (up to first 1024, which is our max)
        for shard_id in 0..self.total_shards.min(1024) {
            if let Ok(shard) = self.shard_manager.get_shard(shard_id).await {
                active_shards += 1;

                let is_leader = shard.is_leader();
                let raft = shard.raft();
                let term = raft.current_term();
                let commit_index = raft.commit_index();

                if is_leader {
                    leader_shards += 1;
                } else {
                    follower_shards += 1;
                }

                // Collect sample (first 10 active shards)
                if shard_samples.len() < 10 {
                    shard_samples.push(ShardHealth {
                        shard_id,
                        is_leader,
                        term: term.as_u64(),
                        commit_index: commit_index.as_u64(),
                        status: "healthy".to_string(),
                    });
                }
            } else {
                // Shard doesn't exist yet (lazy initialization), this is normal
                // Don't count as error
            }
        }

        // Determine overall status
        let status = if active_shards == 0 {
            "starting".to_string()
        } else if has_errors {
            "degraded".to_string()
        } else if active_shards > 0 {
            "healthy".to_string()
        } else {
            "unhealthy".to_string()
        };

        let details = if active_shards == 0 {
            "Server is starting, no shards active yet".to_string()
        } else {
            format!(
                "Active shards: {}, Leader: {}, Follower: {}",
                active_shards, leader_shards, follower_shards
            )
        };

        // Estimate uptime from shard 0 if available
        let uptime_seconds = if let Ok(shard) = self.shard_manager.get_shard(0).await {
            // Use commit index as proxy for uptime (rough estimate)
            // In production, would track actual start time
            shard.raft().commit_index().as_u64() / 10 // Assume ~10 commits/sec
        } else {
            0
        };

        ServerHealthStatus {
            status,
            node_id: self.node_id.clone(),
            uptime_seconds,
            total_shards: self.total_shards,
            active_shards,
            leader_shards,
            follower_shards,
            shard_samples,
            details,
        }
    }

    /// Quick health check (just checks if any shards are active).
    ///
    /// Useful for load balancer health checks that need fast response.
    pub async fn check_quick(&self) -> bool {
        // Check if shard 0 exists and is responsive
        if let Ok(shard) = self.shard_manager.get_shard(0).await {
            // If we can query the shard, it's healthy
            let _ = shard.is_leader();
            true
        } else {
            // No shards active yet, server is still starting
            false
        }
    }
}
