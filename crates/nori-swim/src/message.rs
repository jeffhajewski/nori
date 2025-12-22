//! SWIM protocol messages.
//!
//! Wire format for all SWIM communication: pings, acks, indirect probes,
//! join/leave, and piggybacked gossip.

use crate::{Member, MemberState};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// A gossip entry piggybacked on SWIM messages.
///
/// Contains member state information for dissemination.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GossipEntry {
    /// Member ID
    pub id: String,
    /// Member's network address
    pub addr: SocketAddr,
    /// Member's current state
    pub state: MemberState,
    /// Member's incarnation number
    pub incarnation: u64,
}

impl From<&Member> for GossipEntry {
    fn from(member: &Member) -> Self {
        Self {
            id: member.id.clone(),
            addr: member.addr,
            state: member.state,
            incarnation: member.incarnation,
        }
    }
}

impl From<Member> for GossipEntry {
    fn from(member: Member) -> Self {
        Self {
            id: member.id,
            addr: member.addr,
            state: member.state,
            incarnation: member.incarnation,
        }
    }
}

/// SWIM protocol messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SwimMessage {
    /// Direct ping to check if a member is alive.
    Ping {
        /// Sequence number for matching acks
        seq: u64,
        /// Sender's member ID
        from_id: String,
        /// Sender's address
        from_addr: SocketAddr,
        /// Piggybacked gossip entries
        gossip: Vec<GossipEntry>,
    },

    /// Acknowledgment of a ping.
    Ack {
        /// Sequence number being acknowledged
        seq: u64,
        /// Responder's member ID
        from_id: String,
        /// Responder's address
        from_addr: SocketAddr,
        /// Piggybacked gossip entries
        gossip: Vec<GossipEntry>,
    },

    /// Request for indirect ping (sent to k random members).
    PingReq {
        /// Sequence number for the original ping
        seq: u64,
        /// Requester's member ID
        from_id: String,
        /// Requester's address (for routing ack back)
        from_addr: SocketAddr,
        /// Target member to ping
        target_id: String,
        /// Target's address
        target_addr: SocketAddr,
        /// Piggybacked gossip entries
        gossip: Vec<GossipEntry>,
    },

    /// Indirect ack (forwarded from indirect probe target).
    IndirectAck {
        /// Original sequence number
        seq: u64,
        /// Target member ID (the one that responded)
        target_id: String,
        /// Piggybacked gossip entries
        gossip: Vec<GossipEntry>,
    },

    /// Join request from a new member.
    Join {
        /// New member's ID
        id: String,
        /// New member's address
        addr: SocketAddr,
    },

    /// Join acknowledgment with current membership.
    JoinAck {
        /// Current cluster members
        members: Vec<Member>,
    },

    /// Leave notification.
    Leave {
        /// Leaving member's ID
        id: String,
        /// Incarnation number (to order with other updates)
        incarnation: u64,
    },
}

impl SwimMessage {
    /// Encode the message to bytes using bincode.
    pub fn encode(&self) -> Result<Vec<u8>, MessageError> {
        bincode::serialize(self).map_err(|e| MessageError::Serialization(e.to_string()))
    }

    /// Decode a message from bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, MessageError> {
        bincode::deserialize(bytes).map_err(|e| MessageError::Deserialization(e.to_string()))
    }

    /// Get the gossip entries from this message, if any.
    pub fn gossip(&self) -> &[GossipEntry] {
        match self {
            SwimMessage::Ping { gossip, .. } => gossip,
            SwimMessage::Ack { gossip, .. } => gossip,
            SwimMessage::PingReq { gossip, .. } => gossip,
            SwimMessage::IndirectAck { gossip, .. } => gossip,
            SwimMessage::Join { .. } => &[],
            SwimMessage::JoinAck { .. } => &[],
            SwimMessage::Leave { .. } => &[],
        }
    }

    /// Get the sequence number from this message, if applicable.
    pub fn seq(&self) -> Option<u64> {
        match self {
            SwimMessage::Ping { seq, .. } => Some(*seq),
            SwimMessage::Ack { seq, .. } => Some(*seq),
            SwimMessage::PingReq { seq, .. } => Some(*seq),
            SwimMessage::IndirectAck { seq, .. } => Some(*seq),
            _ => None,
        }
    }
}

/// Message encoding/decoding errors.
#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr() -> SocketAddr {
        "127.0.0.1:8000".parse().unwrap()
    }

    #[test]
    fn test_ping_roundtrip() {
        let msg = SwimMessage::Ping {
            seq: 42,
            from_id: "node1".to_string(),
            from_addr: test_addr(),
            gossip: vec![],
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_ack_roundtrip() {
        let msg = SwimMessage::Ack {
            seq: 42,
            from_id: "node1".to_string(),
            from_addr: test_addr(),
            gossip: vec![GossipEntry {
                id: "node2".to_string(),
                addr: "127.0.0.1:8001".parse().unwrap(),
                state: MemberState::Alive,
                incarnation: 5,
            }],
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_ping_req_roundtrip() {
        let msg = SwimMessage::PingReq {
            seq: 100,
            from_id: "node1".to_string(),
            from_addr: test_addr(),
            target_id: "node3".to_string(),
            target_addr: "127.0.0.1:8002".parse().unwrap(),
            gossip: vec![],
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_join_roundtrip() {
        let msg = SwimMessage::Join {
            id: "new_node".to_string(),
            addr: test_addr(),
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_join_ack_roundtrip() {
        let msg = SwimMessage::JoinAck {
            members: vec![
                Member {
                    id: "node1".to_string(),
                    addr: test_addr(),
                    state: MemberState::Alive,
                    incarnation: 0,
                },
                Member {
                    id: "node2".to_string(),
                    addr: "127.0.0.1:8001".parse().unwrap(),
                    state: MemberState::Suspect,
                    incarnation: 3,
                },
            ],
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_leave_roundtrip() {
        let msg = SwimMessage::Leave {
            id: "leaving_node".to_string(),
            incarnation: 10,
        };

        let bytes = msg.encode().unwrap();
        let decoded = SwimMessage::decode(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn test_gossip_entry_from_member() {
        let member = Member {
            id: "node1".to_string(),
            addr: test_addr(),
            state: MemberState::Alive,
            incarnation: 5,
        };

        let entry: GossipEntry = (&member).into();
        assert_eq!(entry.id, member.id);
        assert_eq!(entry.addr, member.addr);
        assert_eq!(entry.state, member.state);
        assert_eq!(entry.incarnation, member.incarnation);
    }

    #[test]
    fn test_seq_extraction() {
        let ping = SwimMessage::Ping {
            seq: 42,
            from_id: "node1".to_string(),
            from_addr: test_addr(),
            gossip: vec![],
        };
        assert_eq!(ping.seq(), Some(42));

        let join = SwimMessage::Join {
            id: "node1".to_string(),
            addr: test_addr(),
        };
        assert_eq!(join.seq(), None);
    }

    #[test]
    fn test_gossip_extraction() {
        let gossip = vec![GossipEntry {
            id: "node2".to_string(),
            addr: test_addr(),
            state: MemberState::Alive,
            incarnation: 1,
        }];

        let ping = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: test_addr(),
            gossip: gossip.clone(),
        };
        assert_eq!(ping.gossip(), &gossip);

        let join = SwimMessage::Join {
            id: "node1".to_string(),
            addr: test_addr(),
        };
        assert!(join.gossip().is_empty());
    }
}
