//! Leader leases for fast linearizable reads.
//!
//! Per Raft paper §8 (Read-only queries):
//! - Leaders can serve reads without contacting other servers if they have a valid lease
//! - A lease is acquired when the leader receives heartbeat acknowledgments from a majority
//! - Lease duration must be less than election timeout to prevent stale reads
//! - If lease expires, fall back to read-index protocol (quorum check)
//!
//! # Safety
//!
//! The lease mechanism is safe because:
//! 1. Lease duration < election timeout minimum
//! 2. Leader cannot be deposed while lease is valid (no election can complete)
//! 3. If network partitions, lease expires before new leader elected
//!
//! # Design
//!
//! ```text
//! Leader sends heartbeat → Receives majority ACKs → Grants lease for duration D
//!
//! During lease:
//!   - Serve reads locally (no quorum)
//!   - High throughput, low latency
//!
//! After lease expires:
//!   - Fall back to read-index (quorum check)
//!   - Or wait for next heartbeat round to renew lease
//! ```

use crate::config::RaftConfig;
use crate::types::*;
use std::time::{Duration, Instant};

/// Lease state for a Raft leader.
///
/// Tracks when the leader has a valid lease and can serve reads without quorum.
#[derive(Debug, Clone)]
pub struct LeaseState {
    /// When the lease expires
    pub expiry: Option<Instant>,

    /// Lease duration (derived from config)
    duration: Duration,
}

impl LeaseState {
    /// Create a new lease state.
    ///
    /// Lease duration is set to 90% of the minimum election timeout to provide
    /// a safety margin and account for clock skew.
    pub fn new(config: &RaftConfig) -> Self {
        let duration = config.election_timeout_min.mul_f32(0.9);

        Self {
            expiry: None,
            duration,
        }
    }

    /// Check if the lease is currently valid.
    ///
    /// Returns true if the lease exists and has not expired.
    pub fn is_valid(&self) -> bool {
        if let Some(expiry) = self.expiry {
            Instant::now() < expiry
        } else {
            false
        }
    }

    /// Grant or renew the lease.
    ///
    /// Called when the leader receives heartbeat acknowledgments from a majority.
    /// Sets the lease expiry to now + duration.
    pub fn grant(&mut self) {
        self.expiry = Some(Instant::now() + self.duration);
    }

    /// Revoke the lease.
    ///
    /// Called when:
    /// - Leader steps down
    /// - Leader discovers higher term
    /// - Node is no longer leader
    pub fn revoke(&mut self) {
        self.expiry = None;
    }

    /// Get remaining lease time.
    ///
    /// Returns Some(duration) if lease is valid, None otherwise.
    pub fn remaining(&self) -> Option<Duration> {
        if let Some(expiry) = self.expiry {
            let now = Instant::now();
            if now < expiry {
                Some(expiry - now)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Get the lease duration.
    pub fn duration(&self) -> Duration {
        self.duration
    }
}

/// Check if leader can serve a read with its current lease.
///
/// Returns true if:
/// - Node is the leader
/// - Lease is valid (not expired)
///
/// If this returns false, caller should fall back to read-index protocol.
pub fn can_read_with_lease(role: Role, lease: Option<&LeaseState>) -> bool {
    if role != Role::Leader {
        return false;
    }

    if let Some(lease_state) = lease {
        lease_state.is_valid()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_lease_state_new() {
        let config = RaftConfig::default();
        let lease = LeaseState::new(&config);

        assert_eq!(lease.expiry, None);
        assert!(!lease.is_valid());
        assert_eq!(lease.remaining(), None);
    }

    #[test]
    fn test_lease_grant() {
        let config = RaftConfig::default();
        let mut lease = LeaseState::new(&config);

        // Grant lease
        lease.grant();

        assert!(lease.is_valid());
        assert!(lease.remaining().is_some());

        let remaining = lease.remaining().unwrap();
        assert!(remaining > Duration::from_millis(0));
        assert!(remaining <= lease.duration());
    }

    #[test]
    fn test_lease_expiry() {
        let mut config = RaftConfig::default();
        config.election_timeout_min = Duration::from_millis(10);

        let mut lease = LeaseState::new(&config);
        lease.grant();

        assert!(lease.is_valid());

        // Wait for lease to expire
        std::thread::sleep(Duration::from_millis(20));

        assert!(!lease.is_valid());
        assert_eq!(lease.remaining(), None);
    }

    #[test]
    fn test_lease_revoke() {
        let config = RaftConfig::default();
        let mut lease = LeaseState::new(&config);

        lease.grant();
        assert!(lease.is_valid());

        lease.revoke();
        assert!(!lease.is_valid());
        assert_eq!(lease.remaining(), None);
    }

    #[test]
    fn test_can_read_with_lease() {
        let config = RaftConfig::default();
        let mut lease = LeaseState::new(&config);

        // Not leader - cannot read
        assert!(!can_read_with_lease(Role::Follower, Some(&lease)));
        assert!(!can_read_with_lease(Role::Candidate, Some(&lease)));

        // Leader but no lease - cannot read
        assert!(!can_read_with_lease(Role::Leader, None));

        // Leader with invalid lease - cannot read
        assert!(!can_read_with_lease(Role::Leader, Some(&lease)));

        // Leader with valid lease - can read
        lease.grant();
        assert!(can_read_with_lease(Role::Leader, Some(&lease)));

        // Lease expires - cannot read
        lease.revoke();
        assert!(!can_read_with_lease(Role::Leader, Some(&lease)));
    }

    #[test]
    fn test_lease_duration_safety_margin() {
        let config = RaftConfig::default();
        let lease = LeaseState::new(&config);

        // Lease duration should be 90% of min election timeout
        let expected = config.election_timeout_min.mul_f32(0.9);
        assert_eq!(lease.duration(), expected);

        // Verify safety margin: lease < min election timeout
        assert!(lease.duration() < config.election_timeout_min);
    }
}
