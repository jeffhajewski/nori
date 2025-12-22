//! Member list state machine with incarnation logic.
//!
//! Manages the membership view and applies gossip according to SWIM rules:
//! - Higher incarnation always wins
//! - Same incarnation: Suspect > Alive
//! - Self-refutation: bump incarnation and queue refutation

use crate::message::GossipEntry;
use crate::{Member, MemberState, MembershipEvent};
use parking_lot::RwLock;
use rand::seq::SliceRandom;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// Internal member entry with additional metadata.
#[derive(Debug, Clone)]
pub struct MemberEntry {
    pub id: String,
    pub addr: SocketAddr,
    pub state: MemberState,
    pub incarnation: u64,
}

impl MemberEntry {
    pub fn to_member(&self) -> Member {
        Member {
            id: self.id.clone(),
            addr: self.addr,
            state: self.state,
            incarnation: self.incarnation,
        }
    }

    pub fn to_gossip_entry(&self) -> GossipEntry {
        GossipEntry {
            id: self.id.clone(),
            addr: self.addr,
            state: self.state,
            incarnation: self.incarnation,
        }
    }
}

impl From<&Member> for MemberEntry {
    fn from(m: &Member) -> Self {
        Self {
            id: m.id.clone(),
            addr: m.addr,
            state: m.state,
            incarnation: m.incarnation,
        }
    }
}

impl From<&GossipEntry> for MemberEntry {
    fn from(g: &GossipEntry) -> Self {
        Self {
            id: g.id.clone(),
            addr: g.addr,
            state: g.state,
            incarnation: g.incarnation,
        }
    }
}

/// Thread-safe member list with gossip application logic.
pub struct MemberList {
    /// Local node's ID
    local_id: String,

    /// Local node's address
    local_addr: SocketAddr,

    /// Local incarnation number (for self-refutation)
    local_incarnation: AtomicU64,

    /// All known members (id → entry)
    members: RwLock<HashMap<String, MemberEntry>>,

    /// Gossip entries pending dissemination (FIFO queue)
    pending_gossip: RwLock<VecDeque<GossipEntry>>,

    /// Cluster view epoch (incremented on changes)
    epoch: AtomicU64,
}

impl MemberList {
    /// Create a new member list with local node as the only member.
    pub fn new(local_id: String, local_addr: SocketAddr) -> Self {
        let mut members = HashMap::new();
        members.insert(
            local_id.clone(),
            MemberEntry {
                id: local_id.clone(),
                addr: local_addr,
                state: MemberState::Alive,
                incarnation: 0,
            },
        );

        Self {
            local_id,
            local_addr,
            local_incarnation: AtomicU64::new(0),
            members: RwLock::new(members),
            pending_gossip: RwLock::new(VecDeque::new()),
            epoch: AtomicU64::new(0),
        }
    }

    /// Get the local node's ID.
    pub fn local_id(&self) -> &str {
        &self.local_id
    }

    /// Get the local node's address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Get the current incarnation number.
    pub fn local_incarnation(&self) -> u64 {
        self.local_incarnation.load(Ordering::SeqCst)
    }

    /// Get the current epoch.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    /// Get all members as a vector.
    pub fn all_members(&self) -> Vec<Member> {
        self.members
            .read()
            .values()
            .map(|e| e.to_member())
            .collect()
    }

    /// Get a member by ID.
    pub fn get_member(&self, id: &str) -> Option<Member> {
        self.members.read().get(id).map(|e| e.to_member())
    }

    /// Get count of alive/suspect members (excludes failed/left).
    pub fn active_member_count(&self) -> usize {
        self.members
            .read()
            .values()
            .filter(|m| matches!(m.state, MemberState::Alive | MemberState::Suspect))
            .count()
    }

    /// Select a random member for probing (excludes self, failed, and left).
    pub fn get_random_member(&self, exclude: Option<&str>) -> Option<Member> {
        let members = self.members.read();
        let candidates: Vec<_> = members
            .values()
            .filter(|m| {
                m.id != self.local_id
                    && exclude.map_or(true, |e| m.id != e)
                    && matches!(m.state, MemberState::Alive | MemberState::Suspect)
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let mut rng = rand::thread_rng();
        candidates.choose(&mut rng).map(|e| e.to_member())
    }

    /// Select k random members for indirect probing (excludes self, target, failed, left).
    pub fn get_random_members(&self, k: usize, exclude: &[&str]) -> Vec<Member> {
        let members = self.members.read();
        let mut candidates: Vec<_> = members
            .values()
            .filter(|m| {
                m.id != self.local_id
                    && !exclude.contains(&m.id.as_str())
                    && matches!(m.state, MemberState::Alive | MemberState::Suspect)
            })
            .collect();

        let mut rng = rand::thread_rng();
        candidates.shuffle(&mut rng);
        candidates.into_iter().take(k).map(|e| e.to_member()).collect()
    }

    /// Apply a gossip entry according to SWIM rules.
    ///
    /// Returns the membership event if state changed, None otherwise.
    pub fn apply_gossip(&self, entry: &GossipEntry) -> Option<MembershipEvent> {
        // Handle self-gossip (refutation)
        if entry.id == self.local_id {
            return self.handle_self_gossip(entry);
        }

        let mut members = self.members.write();

        if let Some(current) = members.get_mut(&entry.id) {
            // Existing member - apply incarnation rules
            self.apply_to_existing(current, entry)
        } else {
            // New member
            let event = MembershipEvent::MemberJoined {
                id: entry.id.clone(),
                addr: entry.addr,
            };
            members.insert(entry.id.clone(), MemberEntry::from(entry));
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.queue_gossip(entry.clone());
            Some(event)
        }
    }

    /// Handle gossip about self (for refutation).
    fn handle_self_gossip(&self, entry: &GossipEntry) -> Option<MembershipEvent> {
        // If someone says we're Suspect or Failed, refute it
        if matches!(entry.state, MemberState::Suspect | MemberState::Failed) {
            let new_incarnation = self.local_incarnation.fetch_add(1, Ordering::SeqCst) + 1;

            // Update our own entry
            let mut members = self.members.write();
            if let Some(me) = members.get_mut(&self.local_id) {
                me.incarnation = new_incarnation;
                me.state = MemberState::Alive;
            }

            // Queue refutation gossip
            self.queue_gossip(GossipEntry {
                id: self.local_id.clone(),
                addr: self.local_addr,
                state: MemberState::Alive,
                incarnation: new_incarnation,
            });

            self.epoch.fetch_add(1, Ordering::SeqCst);

            return Some(MembershipEvent::MemberAlive {
                id: self.local_id.clone(),
                incarnation: new_incarnation,
            });
        }

        None
    }

    /// Apply gossip to an existing member entry.
    fn apply_to_existing(
        &self,
        current: &mut MemberEntry,
        entry: &GossipEntry,
    ) -> Option<MembershipEvent> {
        // Rule 1: Higher incarnation always wins
        if entry.incarnation > current.incarnation {
            let event = self.make_event(entry);
            current.incarnation = entry.incarnation;
            current.state = entry.state;
            current.addr = entry.addr;
            self.epoch.fetch_add(1, Ordering::SeqCst);
            self.queue_gossip(entry.clone());
            return event;
        }

        // Rule 2: Same incarnation - state ordering
        if entry.incarnation == current.incarnation {
            // Suspect > Alive (at same incarnation)
            if entry.state == MemberState::Suspect && current.state == MemberState::Alive {
                current.state = MemberState::Suspect;
                self.epoch.fetch_add(1, Ordering::SeqCst);
                self.queue_gossip(entry.clone());
                return Some(MembershipEvent::MemberSuspect {
                    id: entry.id.clone(),
                    incarnation: entry.incarnation,
                });
            }

            // Failed > Suspect (at same incarnation)
            if entry.state == MemberState::Failed && current.state == MemberState::Suspect {
                current.state = MemberState::Failed;
                self.epoch.fetch_add(1, Ordering::SeqCst);
                self.queue_gossip(entry.clone());
                return Some(MembershipEvent::MemberFailed {
                    id: entry.id.clone(),
                });
            }

            // Left is final
            if entry.state == MemberState::Left && current.state != MemberState::Left {
                current.state = MemberState::Left;
                self.epoch.fetch_add(1, Ordering::SeqCst);
                self.queue_gossip(entry.clone());
                return Some(MembershipEvent::MemberLeft {
                    id: entry.id.clone(),
                });
            }
        }

        // No change
        None
    }

    /// Create a membership event from a gossip entry.
    fn make_event(&self, entry: &GossipEntry) -> Option<MembershipEvent> {
        match entry.state {
            MemberState::Alive => Some(MembershipEvent::MemberAlive {
                id: entry.id.clone(),
                incarnation: entry.incarnation,
            }),
            MemberState::Suspect => Some(MembershipEvent::MemberSuspect {
                id: entry.id.clone(),
                incarnation: entry.incarnation,
            }),
            MemberState::Failed => Some(MembershipEvent::MemberFailed {
                id: entry.id.clone(),
            }),
            MemberState::Left => Some(MembershipEvent::MemberLeft {
                id: entry.id.clone(),
            }),
        }
    }

    /// Mark a member as suspect (called when probe fails).
    pub fn suspect(&self, id: &str) -> Option<MembershipEvent> {
        if id == self.local_id {
            return None; // Can't suspect self
        }

        let mut members = self.members.write();
        if let Some(member) = members.get_mut(id) {
            if member.state == MemberState::Alive {
                member.state = MemberState::Suspect;
                self.epoch.fetch_add(1, Ordering::SeqCst);
                self.queue_gossip(member.to_gossip_entry());
                return Some(MembershipEvent::MemberSuspect {
                    id: id.to_string(),
                    incarnation: member.incarnation,
                });
            }
        }
        None
    }

    /// Mark a member as failed (called when suspicion timeout expires).
    pub fn confirm_failed(&self, id: &str) -> Option<MembershipEvent> {
        if id == self.local_id {
            return None; // Can't fail self
        }

        let mut members = self.members.write();
        if let Some(member) = members.get_mut(id) {
            if member.state == MemberState::Suspect {
                member.state = MemberState::Failed;
                self.epoch.fetch_add(1, Ordering::SeqCst);
                self.queue_gossip(member.to_gossip_entry());
                return Some(MembershipEvent::MemberFailed {
                    id: id.to_string(),
                });
            }
        }
        None
    }

    /// Mark local node as left.
    pub fn leave(&self) -> Option<MembershipEvent> {
        let new_incarnation = self.local_incarnation.fetch_add(1, Ordering::SeqCst) + 1;

        let mut members = self.members.write();
        if let Some(me) = members.get_mut(&self.local_id) {
            me.state = MemberState::Left;
            me.incarnation = new_incarnation;
        }

        self.epoch.fetch_add(1, Ordering::SeqCst);

        self.queue_gossip(GossipEntry {
            id: self.local_id.clone(),
            addr: self.local_addr,
            state: MemberState::Left,
            incarnation: new_incarnation,
        });

        Some(MembershipEvent::MemberLeft {
            id: self.local_id.clone(),
        })
    }

    /// Add a member directly (used during join).
    pub fn add_member(&self, member: Member) {
        let mut members = self.members.write();
        if !members.contains_key(&member.id) {
            members.insert(member.id.clone(), MemberEntry::from(&member));
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Queue a gossip entry for dissemination.
    fn queue_gossip(&self, entry: GossipEntry) {
        let mut queue = self.pending_gossip.write();
        // Avoid duplicates - remove old entry for same member if exists
        queue.retain(|e| e.id != entry.id);
        queue.push_back(entry);
    }

    /// Drain up to `max` gossip entries for piggybacking on a message.
    pub fn drain_gossip(&self, max: usize) -> Vec<GossipEntry> {
        let mut queue = self.pending_gossip.write();
        let count = max.min(queue.len());
        queue.drain(..count).collect()
    }

    /// Get the current gossip queue length.
    pub fn pending_gossip_count(&self) -> usize {
        self.pending_gossip.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    #[test]
    fn test_new_member_list() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        assert_eq!(list.local_id(), "node1");
        assert_eq!(list.local_incarnation(), 0);
        assert_eq!(list.epoch(), 0);
        assert_eq!(list.all_members().len(), 1);

        let member = list.get_member("node1").unwrap();
        assert_eq!(member.id, "node1");
        assert_eq!(member.state, MemberState::Alive);
    }

    #[test]
    fn test_apply_gossip_new_member() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        let entry = GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        };

        let event = list.apply_gossip(&entry);
        assert!(matches!(event, Some(MembershipEvent::MemberJoined { id, .. }) if id == "node2"));
        assert_eq!(list.all_members().len(), 2);
        assert_eq!(list.epoch(), 1);
    }

    #[test]
    fn test_apply_gossip_higher_incarnation_wins() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add node2
        list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        });

        // Update to Suspect with same incarnation
        list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Suspect,
            incarnation: 0,
        });

        let member = list.get_member("node2").unwrap();
        assert_eq!(member.state, MemberState::Suspect);

        // Refute with higher incarnation
        let event = list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 1,
        });

        assert!(matches!(event, Some(MembershipEvent::MemberAlive { incarnation: 1, .. })));
        let member = list.get_member("node2").unwrap();
        assert_eq!(member.state, MemberState::Alive);
        assert_eq!(member.incarnation, 1);
    }

    #[test]
    fn test_self_refutation() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Someone says we're suspect
        let event = list.apply_gossip(&GossipEntry {
            id: "node1".to_string(),
            addr: test_addr(8001),
            state: MemberState::Suspect,
            incarnation: 0,
        });

        // We should refute by bumping incarnation
        assert!(matches!(event, Some(MembershipEvent::MemberAlive { id, incarnation: 1 }) if id == "node1"));
        assert_eq!(list.local_incarnation(), 1);

        let member = list.get_member("node1").unwrap();
        assert_eq!(member.state, MemberState::Alive);
        assert_eq!(member.incarnation, 1);
    }

    #[test]
    fn test_suspect_then_failed() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add and suspect node2
        list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        });

        let event = list.suspect("node2");
        assert!(matches!(event, Some(MembershipEvent::MemberSuspect { .. })));

        // Confirm failed
        let event = list.confirm_failed("node2");
        assert!(matches!(event, Some(MembershipEvent::MemberFailed { .. })));

        let member = list.get_member("node2").unwrap();
        assert_eq!(member.state, MemberState::Failed);
    }

    #[test]
    fn test_random_member_selection() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add some members
        for i in 2..=5 {
            list.apply_gossip(&GossipEntry {
                id: format!("node{}", i),
                addr: test_addr(8000 + i),
                state: MemberState::Alive,
                incarnation: 0,
            });
        }

        // Random selection should not return self
        for _ in 0..10 {
            let member = list.get_random_member(None).unwrap();
            assert_ne!(member.id, "node1");
        }

        // With exclusion
        for _ in 0..10 {
            let member = list.get_random_member(Some("node2")).unwrap();
            assert_ne!(member.id, "node1");
            assert_ne!(member.id, "node2");
        }
    }

    #[test]
    fn test_random_members_k() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add 5 members
        for i in 2..=6 {
            list.apply_gossip(&GossipEntry {
                id: format!("node{}", i),
                addr: test_addr(8000 + i),
                state: MemberState::Alive,
                incarnation: 0,
            });
        }

        // Get k=3 random members excluding node2
        let members = list.get_random_members(3, &["node2"]);
        assert_eq!(members.len(), 3);
        for m in &members {
            assert_ne!(m.id, "node1");
            assert_ne!(m.id, "node2");
        }
    }

    #[test]
    fn test_gossip_queue() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add members (each should queue gossip)
        for i in 2..=5 {
            list.apply_gossip(&GossipEntry {
                id: format!("node{}", i),
                addr: test_addr(8000 + i),
                state: MemberState::Alive,
                incarnation: 0,
            });
        }

        assert_eq!(list.pending_gossip_count(), 4);

        // Drain some
        let drained = list.drain_gossip(2);
        assert_eq!(drained.len(), 2);
        assert_eq!(list.pending_gossip_count(), 2);

        // Drain rest
        let drained = list.drain_gossip(10);
        assert_eq!(drained.len(), 2);
        assert_eq!(list.pending_gossip_count(), 0);
    }

    #[test]
    fn test_leave() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        let event = list.leave();
        assert!(matches!(event, Some(MembershipEvent::MemberLeft { id }) if id == "node1"));

        let member = list.get_member("node1").unwrap();
        assert_eq!(member.state, MemberState::Left);
        assert_eq!(member.incarnation, 1);
    }

    #[test]
    fn test_active_member_count() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add members
        list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        });
        list.apply_gossip(&GossipEntry {
            id: "node3".to_string(),
            addr: test_addr(8003),
            state: MemberState::Suspect,
            incarnation: 0,
        });
        list.apply_gossip(&GossipEntry {
            id: "node4".to_string(),
            addr: test_addr(8004),
            state: MemberState::Failed,
            incarnation: 0,
        });

        // Only alive and suspect count as active
        assert_eq!(list.active_member_count(), 3); // node1, node2, node3
    }

    #[test]
    fn test_gossip_dedup() {
        let list = MemberList::new("node1".to_string(), test_addr(8001));

        // Add member
        list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(8002),
            state: MemberState::Alive,
            incarnation: 0,
        });

        // Suspect same member (should replace in queue)
        list.suspect("node2");

        // Queue should have 1 entry (the newer suspect state)
        assert_eq!(list.pending_gossip_count(), 1);

        let drained = list.drain_gossip(10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].state, MemberState::Suspect);
    }
}
