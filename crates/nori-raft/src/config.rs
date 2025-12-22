//! Raft configuration (timeouts, limits, tuning parameters).

use std::time::Duration;

/// Raft configuration.
///
/// Controls election timeouts, heartbeat intervals, snapshot triggers, and other tuning parameters.
/// Defaults are based on context/31_consensus.yaml specifications.
#[derive(Debug, Clone)]
pub struct RaftConfig {
    /// Shard ID for observability events (default: 0).
    /// Used to identify this Raft group in multi-shard deployments.
    pub shard_id: u32,

    /// Heartbeat interval (leader → followers).
    ///
    /// Leader sends AppendEntries (heartbeat or real entries) at this interval.
    /// Must be < election_timeout_min to prevent spurious elections.
    ///
    /// Default: 150ms
    pub heartbeat_interval: Duration,

    /// Minimum election timeout (follower → candidate).
    ///
    /// If follower doesn't hear from leader within this duration, it starts election.
    /// Randomized between [min, max] to prevent split votes.
    ///
    /// Default: 300ms
    pub election_timeout_min: Duration,

    /// Maximum election timeout.
    ///
    /// Upper bound for randomized election timeout.
    ///
    /// Default: 600ms
    pub election_timeout_max: Duration,

    /// Leader lease duration.
    ///
    /// Leader assumes it's leader for this duration after receiving quorum heartbeat acks.
    /// Enables fast linearizable reads without read-index quorum.
    ///
    /// Default: 2000ms
    pub lease_duration: Duration,

    /// Maximum number of entries per AppendEntries RPC.
    ///
    /// Limits message size and processing time per RPC.
    /// Larger = fewer RPCs, but more memory per message.
    ///
    /// Default: 1000 entries
    pub max_entries_per_append: usize,

    /// Snapshot trigger: log size threshold (bytes).
    ///
    /// Create snapshot when log exceeds this size.
    /// Reduces recovery time and disk usage.
    ///
    /// Default: 256 MiB
    pub snapshot_log_size_bytes: u64,

    /// Snapshot trigger: log entry count threshold.
    ///
    /// Create snapshot when (last_applied - last_snapshot_index) exceeds this.
    /// Alternative to size-based trigger.
    ///
    /// Default: 1,000,000 entries
    pub snapshot_entry_count: u64,

    /// InstallSnapshot chunk size (bytes).
    ///
    /// Split large snapshots into chunks for streaming.
    /// Smaller = less memory per RPC, more RPCs needed.
    ///
    /// Default: 1 MiB
    pub snapshot_chunk_size: usize,

    /// Maximum concurrent read-index requests.
    ///
    /// Limits memory usage for pending read-index operations.
    /// Read-index requires quorum, so can be slow under load.
    ///
    /// Default: 1000
    pub max_pending_read_index: usize,

    /// Apply batch size (commands applied to state machine in batches).
    ///
    /// Larger batches = more throughput, but higher latency variance.
    ///
    /// Default: 100 commands
    pub apply_batch_size: usize,

    /// Propose timeout (how long to wait for commit).
    ///
    /// After appending a log entry, propose() waits until commit_index
    /// reaches the entry's index. This timeout prevents indefinite waits.
    ///
    /// Default: 5000ms
    pub propose_timeout: Duration,
}

impl Default for RaftConfig {
    fn default() -> Self {
        Self {
            shard_id: 0,
            // Per context/31_consensus.yaml
            heartbeat_interval: Duration::from_millis(150),
            election_timeout_min: Duration::from_millis(300),
            election_timeout_max: Duration::from_millis(600),
            lease_duration: Duration::from_millis(2000),

            // Replication tuning
            max_entries_per_append: 1000,

            // Snapshot triggers (per context/31_consensus.yaml)
            snapshot_log_size_bytes: 256 * 1024 * 1024, // 256 MiB
            snapshot_entry_count: 1_000_000,

            // Snapshot streaming
            snapshot_chunk_size: 1024 * 1024, // 1 MiB

            // Read-index limits
            max_pending_read_index: 1000,

            // Apply batching
            apply_batch_size: 100,

            // Propose timeout
            propose_timeout: Duration::from_millis(5000),
        }
    }
}

impl RaftConfig {
    /// Validate configuration (ensure invariants hold).
    ///
    /// Returns an error if configuration is invalid.
    pub fn validate(&self) -> Result<(), String> {
        // Heartbeat must be less than election timeout min
        if self.heartbeat_interval >= self.election_timeout_min {
            return Err(format!(
                "heartbeat_interval ({:?}) must be < election_timeout_min ({:?})",
                self.heartbeat_interval, self.election_timeout_min
            ));
        }

        // Election timeout min must be < max
        if self.election_timeout_min >= self.election_timeout_max {
            return Err(format!(
                "election_timeout_min ({:?}) must be < election_timeout_max ({:?})",
                self.election_timeout_min, self.election_timeout_max
            ));
        }

        // Lease must be long enough to prevent spurious lease expiry
        if self.lease_duration < self.heartbeat_interval * 2 {
            return Err(format!(
                "lease_duration ({:?}) should be >= 2 * heartbeat_interval ({:?})",
                self.lease_duration, self.heartbeat_interval
            ));
        }

        // Positive limits
        if self.max_entries_per_append == 0 {
            return Err("max_entries_per_append must be > 0".to_string());
        }

        if self.snapshot_chunk_size == 0 {
            return Err("snapshot_chunk_size must be > 0".to_string());
        }

        Ok(())
    }

    /// Get randomized election timeout.
    ///
    /// Returns a random duration between [election_timeout_min, election_timeout_max].
    /// Each server gets a different timeout to prevent split votes.
    pub fn random_election_timeout(&self) -> Duration {
        use rand::Rng;
        let min_ms = self.election_timeout_min.as_millis() as u64;
        let max_ms = self.election_timeout_max.as_millis() as u64;
        let random_ms = rand::thread_rng().gen_range(min_ms..=max_ms);
        Duration::from_millis(random_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_valid() {
        let config = RaftConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_heartbeat_too_long() {
        let mut config = RaftConfig::default();
        config.heartbeat_interval = Duration::from_millis(400); // > election_timeout_min
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_election_timeout_range() {
        let mut config = RaftConfig::default();
        config.election_timeout_min = Duration::from_millis(700); // > max
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_random_election_timeout_in_range() {
        let config = RaftConfig::default();
        for _ in 0..100 {
            let timeout = config.random_election_timeout();
            assert!(timeout >= config.election_timeout_min);
            assert!(timeout <= config.election_timeout_max);
        }
    }
}
