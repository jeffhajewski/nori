//! Suspicion timer management.
//!
//! Tracks members in the Suspect state and transitions them to Failed
//! after the suspicion timeout expires.

use crate::member_list::MemberList;
use crate::MembershipEvent;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Entry for a suspected member.
#[derive(Debug, Clone)]
struct SuspicionEntry {
    /// When the member was first suspected
    suspected_at: Instant,
    /// Member's incarnation when suspected
    incarnation: u64,
}

/// Manages suspicion timers for suspected members.
pub struct SuspicionManager {
    /// Suspicion timeout duration
    timeout: Duration,

    /// Tracked suspects (member_id → entry)
    suspects: RwLock<HashMap<String, SuspicionEntry>>,
}

impl SuspicionManager {
    /// Create a new suspicion manager.
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            suspects: RwLock::new(HashMap::new()),
        }
    }

    /// Start tracking a suspected member.
    ///
    /// If already tracking, this is a no-op.
    pub fn start_suspicion(&self, id: &str, incarnation: u64) {
        let mut suspects = self.suspects.write();
        suspects.entry(id.to_string()).or_insert(SuspicionEntry {
            suspected_at: Instant::now(),
            incarnation,
        });
    }

    /// Clear suspicion for a member (e.g., refutation or confirmed alive).
    pub fn clear_suspicion(&self, id: &str) {
        self.suspects.write().remove(id);
    }

    /// Check if a member is being tracked as suspect.
    pub fn is_suspected(&self, id: &str) -> bool {
        self.suspects.read().contains_key(id)
    }

    /// Get count of suspected members.
    pub fn suspect_count(&self) -> usize {
        self.suspects.read().len()
    }

    /// Get time remaining until a member is confirmed failed.
    ///
    /// Returns None if not suspected, or Some(Duration) remaining.
    pub fn time_remaining(&self, id: &str) -> Option<Duration> {
        let suspects = self.suspects.read();
        suspects.get(id).map(|entry| {
            let elapsed = entry.suspected_at.elapsed();
            self.timeout.saturating_sub(elapsed)
        })
    }

    /// Check for expired suspicions and transition to Failed.
    ///
    /// Returns a list of member IDs that should be marked as Failed.
    pub fn check_expired(&self) -> Vec<String> {
        let now = Instant::now();
        let suspects = self.suspects.read();

        suspects
            .iter()
            .filter_map(|(id, entry)| {
                if now.duration_since(entry.suspected_at) >= self.timeout {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Process expired suspicions, updating member list and emitting events.
    ///
    /// Returns the events that were emitted.
    pub fn process_expired(
        &self,
        member_list: &MemberList,
        event_tx: &broadcast::Sender<MembershipEvent>,
    ) -> Vec<MembershipEvent> {
        let expired = self.check_expired();
        let mut events = Vec::new();

        for id in expired {
            // Remove from suspicion tracking
            self.clear_suspicion(&id);

            // Mark as failed in member list
            if let Some(event) = member_list.confirm_failed(&id) {
                let _ = event_tx.send(event.clone());
                events.push(event);
            }
        }

        events
    }

    /// Run a check loop that periodically processes expired suspicions.
    ///
    /// This is typically spawned as a background task.
    pub async fn run_check_loop(
        self: std::sync::Arc<Self>,
        member_list: std::sync::Arc<MemberList>,
        event_tx: broadcast::Sender<MembershipEvent>,
        shutdown: std::sync::Arc<tokio::sync::Notify>,
        check_interval: Duration,
    ) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(check_interval) => {
                    self.process_expired(&member_list, &event_tx);
                }
                _ = shutdown.notified() => {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GossipEntry, MemberState};
    use std::net::SocketAddr;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    #[test]
    fn test_start_suspicion() {
        let sm = SuspicionManager::new(Duration::from_secs(5));

        sm.start_suspicion("node2", 0);
        assert!(sm.is_suspected("node2"));
        assert_eq!(sm.suspect_count(), 1);
    }

    #[test]
    fn test_clear_suspicion() {
        let sm = SuspicionManager::new(Duration::from_secs(5));

        sm.start_suspicion("node2", 0);
        assert!(sm.is_suspected("node2"));

        sm.clear_suspicion("node2");
        assert!(!sm.is_suspected("node2"));
        assert_eq!(sm.suspect_count(), 0);
    }

    #[test]
    fn test_time_remaining() {
        let sm = SuspicionManager::new(Duration::from_millis(100));

        sm.start_suspicion("node2", 0);

        let remaining = sm.time_remaining("node2").unwrap();
        assert!(remaining <= Duration::from_millis(100));
        assert!(remaining > Duration::ZERO);

        // Non-existent member
        assert!(sm.time_remaining("unknown").is_none());
    }

    #[test]
    fn test_check_expired_not_expired() {
        let sm = SuspicionManager::new(Duration::from_secs(5));

        sm.start_suspicion("node2", 0);

        // Should not be expired yet
        let expired = sm.check_expired();
        assert!(expired.is_empty());
    }

    #[tokio::test]
    async fn test_check_expired_after_timeout() {
        let sm = SuspicionManager::new(Duration::from_millis(10));

        sm.start_suspicion("node2", 0);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        let expired = sm.check_expired();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], "node2");
    }

    #[tokio::test]
    async fn test_process_expired() {
        let sm = SuspicionManager::new(Duration::from_millis(10));
        let member_list = MemberList::new("node1".to_string(), test_addr(8001));
        let (event_tx, mut event_rx) = broadcast::channel(10);

        // Add node2 and suspect it
        member_list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        });
        member_list.suspect("node2");
        sm.start_suspicion("node2", 0);

        // Wait for timeout
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Process expired
        let events = sm.process_expired(&member_list, &event_tx);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], MembershipEvent::MemberFailed { ref id } if id == "node2"));

        // Suspicion should be cleared
        assert!(!sm.is_suspected("node2"));

        // Member should be failed
        let member = member_list.get_member("node2").unwrap();
        assert_eq!(member.state, MemberState::Failed);

        // Event should be broadcast
        let event = event_rx.try_recv().unwrap();
        assert!(matches!(event, MembershipEvent::MemberFailed { id } if id == "node2"));
    }

    #[test]
    fn test_suspicion_dedup() {
        let sm = SuspicionManager::new(Duration::from_secs(5));

        sm.start_suspicion("node2", 0);
        let time1 = sm.time_remaining("node2").unwrap();

        // Re-starting should not reset the timer
        std::thread::sleep(std::time::Duration::from_millis(10));
        sm.start_suspicion("node2", 1);
        let time2 = sm.time_remaining("node2").unwrap();

        assert!(time2 < time1);
    }
}
