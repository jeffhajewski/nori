//! SWIM-like membership/failure detector.
//!
//! Provides cluster membership tracking with failure detection using a SWIM-like gossip protocol.
//!
//! # Architecture
//!
//! - Periodic probing of random members
//! - Indirect probes when direct probes fail
//! - Suspicion mechanism before declaring failures
//! - Gossip-based dissemination of membership changes
//!
//! # Modules
//!
//! - [`config`]: Protocol configuration (timeouts, fanout, etc.)
//! - [`message`]: Wire protocol messages (Ping, Ack, PingReq, etc.)
//! - [`transport`]: Transport abstraction for network I/O

pub mod config;
pub mod member_list;
pub mod message;
pub mod node;
pub mod probe;
pub mod suspicion;
pub mod timer;
pub mod transport;

pub use config::{ConfigError, SwimConfig};
pub use member_list::MemberList;
pub use message::{GossipEntry, MessageError, SwimMessage};
pub use node::SwimNode;
pub use probe::ProbeManager;
pub use suspicion::SuspicionManager;
pub use timer::{PeriodicTimer, Timeout, TimeoutResult};
pub use transport::{create_transport_mesh, InMemoryTransport, SwimTransport};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Membership trait for cluster membership management.
///
/// Provides methods to track cluster members, detect failures, and handle joins/leaves.
#[async_trait]
pub trait Membership: Send + Sync {
    /// Get the current cluster view.
    fn view(&self) -> ClusterView;

    /// Subscribe to membership events.
    fn events(&self) -> tokio::sync::broadcast::Receiver<MembershipEvent>;

    /// Join a cluster via a seed node.
    async fn join(&self, seed: SocketAddr) -> Result<(), MembershipError>;

    /// Leave the cluster gracefully.
    async fn leave(&self) -> Result<(), MembershipError>;
}

/// Cluster membership view.
///
/// Represents the current state of cluster members at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterView {
    /// Epoch/version of this view
    pub epoch: u64,

    /// List of cluster members
    pub members: Vec<Member>,
}

/// A member in the cluster.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Member {
    /// Member ID
    pub id: String,

    /// Network address
    pub addr: SocketAddr,

    /// Current member state
    pub state: MemberState,

    /// Incarnation number (for conflict resolution)
    pub incarnation: u64,
}

/// Member state in the SWIM protocol.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum MemberState {
    /// Member is alive and responding
    Alive,

    /// Member is suspected of failure
    Suspect,

    /// Member has been confirmed failed
    Failed,

    /// Member left gracefully
    Left,
}

/// Membership events emitted by the SWIM protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MembershipEvent {
    /// A member joined the cluster
    MemberJoined { id: String, addr: SocketAddr },

    /// A member became suspected
    MemberSuspect { id: String, incarnation: u64 },

    /// A member was confirmed failed
    MemberFailed { id: String },

    /// A member left gracefully
    MemberLeft { id: String },

    /// A member refuted a suspicion
    MemberAlive { id: String, incarnation: u64 },
}

/// Errors from membership operations.
#[derive(Debug, thiserror::Error)]
pub enum MembershipError {
    #[error("Failed to join cluster: {0}")]
    JoinFailed(String),

    #[error("Failed to leave cluster: {0}")]
    LeaveFailed(String),

    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// SWIM protocol errors.
#[derive(Debug, thiserror::Error)]
pub enum SwimError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Message error: {0}")]
    Message(#[from] MessageError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Timeout")]
    Timeout,

    #[error("Shutdown")]
    Shutdown,
}

/// Placeholder SWIM implementation.
///
/// Provides a minimal in-memory membership tracker.
/// Full SWIM protocol (probing, indirect probes, gossip) to be implemented.
pub struct SwimMembership {
    /// Local member ID
    local_id: String,

    /// Local address
    #[allow(dead_code)]
    local_addr: SocketAddr,

    /// Current cluster view
    view: Arc<RwLock<ClusterView>>,

    /// Event broadcast channel
    event_tx: tokio::sync::broadcast::Sender<MembershipEvent>,
}

impl SwimMembership {
    /// Create a new SWIM membership instance.
    ///
    /// # Arguments
    /// - `id`: This node's member ID
    /// - `addr`: This node's network address
    pub fn new(id: String, addr: SocketAddr) -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(100);

        let local_member = Member {
            id: id.clone(),
            addr,
            state: MemberState::Alive,
            incarnation: 0,
        };

        let view = ClusterView {
            epoch: 0,
            members: vec![local_member],
        };

        Self {
            local_id: id,
            local_addr: addr,
            view: Arc::new(RwLock::new(view)),
            event_tx,
        }
    }

    /// Start SWIM background tasks.
    ///
    /// Placeholder for future implementation of:
    /// - Periodic probing
    /// - Suspicion timers
    /// - Gossip dissemination
    pub async fn start(&self) -> Result<(), MembershipError> {
        tracing::info!("SWIM membership started (placeholder - probing/gossip not yet implemented)");
        Ok(())
    }

    /// Shutdown SWIM gracefully.
    pub async fn shutdown(&self) -> Result<(), MembershipError> {
        tracing::info!("SWIM membership shutting down");
        self.leave().await?;
        Ok(())
    }
}

#[async_trait]
impl Membership for SwimMembership {
    fn view(&self) -> ClusterView {
        // Use try_read to avoid blocking in async context
        let view = self.view.try_read().expect("Failed to acquire read lock");
        view.clone()
    }

    fn events(&self) -> tokio::sync::broadcast::Receiver<MembershipEvent> {
        self.event_tx.subscribe()
    }

    async fn join(&self, seed: SocketAddr) -> Result<(), MembershipError> {
        tracing::info!(
            "SWIM join requested to seed {} (placeholder - actual join protocol not implemented)",
            seed
        );

        // TODO: Implement actual SWIM join protocol:
        // 1. Send JOIN message to seed
        // 2. Receive cluster membership view
        // 3. Start probing cluster members
        // 4. Announce self as alive via gossip

        // For now, just log
        Ok(())
    }

    async fn leave(&self) -> Result<(), MembershipError> {
        tracing::info!(
            "SWIM leave requested for {} (placeholder - actual leave broadcast not implemented)",
            self.local_id
        );

        // TODO: Implement actual SWIM leave protocol:
        // 1. Broadcast LEAVE message to all members
        // 2. Update local state to Left
        // 3. Stop probing
        // 4. Emit MemberLeft event

        let mut view = self.view.write().await;
        view.epoch += 1;

        // Update self to Left state
        if let Some(member) = view.members.iter_mut().find(|m| m.id == self.local_id) {
            member.state = MemberState::Left;
        }

        // Emit event
        let _ = self.event_tx.send(MembershipEvent::MemberLeft {
            id: self.local_id.clone(),
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_swim_creation() {
        let swim = SwimMembership::new("node1".to_string(), "127.0.0.1:8000".parse().unwrap());
        let view = swim.view();

        assert_eq!(view.members.len(), 1);
        assert_eq!(view.members[0].id, "node1");
        assert_eq!(view.members[0].state, MemberState::Alive);
    }

    #[tokio::test]
    async fn test_swim_leave() {
        let swim = SwimMembership::new("node1".to_string(), "127.0.0.1:8000".parse().unwrap());

        swim.leave().await.unwrap();

        let view = swim.view();
        assert_eq!(view.members[0].state, MemberState::Left);
    }

    #[tokio::test]
    async fn test_membership_events() {
        let swim = SwimMembership::new("node1".to_string(), "127.0.0.1:8000".parse().unwrap());
        let mut rx = swim.events();

        swim.leave().await.unwrap();

        // Should receive MemberLeft event
        let event = rx.recv().await.unwrap();
        match event {
            MembershipEvent::MemberLeft { id } => {
                assert_eq!(id, "node1");
            }
            _ => panic!("Expected MemberLeft event"),
        }
    }
}
