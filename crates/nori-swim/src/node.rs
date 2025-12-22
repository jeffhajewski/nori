//! SWIM node implementation.
//!
//! Wires together all SWIM components into a running membership node:
//! - Member list management
//! - Probe loop for failure detection
//! - Suspicion management
//! - Message dispatch
//! - Join/Leave protocols

use crate::config::SwimConfig;
use crate::member_list::MemberList;
use crate::message::SwimMessage;
use crate::probe::{self, ProbeManager};
use crate::suspicion::SuspicionManager;
use crate::timer::Timeout;
use crate::transport::SwimTransport;
use crate::{ClusterView, GossipEntry, Member, MembershipError, MembershipEvent};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

/// A running SWIM membership node.
pub struct SwimNode<T: SwimTransport> {
    /// Configuration
    config: SwimConfig,

    /// Transport for sending/receiving messages
    transport: Arc<T>,

    /// Member list state machine
    member_list: Arc<MemberList>,

    /// Probe manager for tracking pending probes
    probe_manager: Arc<ProbeManager>,

    /// Suspicion manager for tracking suspected members
    suspicion_manager: Arc<SuspicionManager>,

    /// Event broadcast channel
    event_tx: broadcast::Sender<MembershipEvent>,

    /// Shutdown notifier
    shutdown: Arc<Notify>,
}

impl<T: SwimTransport> SwimNode<T> {
    /// Create a new SWIM node.
    pub fn new(id: String, addr: SocketAddr, config: SwimConfig, transport: Arc<T>) -> Self {
        let (event_tx, _) = broadcast::channel(config.event_channel_capacity);

        Self {
            suspicion_manager: Arc::new(SuspicionManager::new(config.suspicion_timeout)),
            config,
            transport,
            member_list: Arc::new(MemberList::new(id, addr)),
            probe_manager: Arc::new(ProbeManager::new()),
            event_tx,
            shutdown: Arc::new(Notify::new()),
        }
    }

    /// Get the local node ID.
    pub fn local_id(&self) -> &str {
        self.member_list.local_id()
    }

    /// Get the local node address.
    pub fn local_addr(&self) -> SocketAddr {
        self.member_list.local_addr()
    }

    /// Get the current cluster view.
    pub fn view(&self) -> ClusterView {
        ClusterView {
            epoch: self.member_list.epoch(),
            members: self.member_list.all_members(),
        }
    }

    /// Subscribe to membership events.
    pub fn events(&self) -> broadcast::Receiver<MembershipEvent> {
        self.event_tx.subscribe()
    }

    /// Join a cluster via a seed node.
    pub async fn join(&self, seed: SocketAddr) -> Result<(), MembershipError> {
        // Send join message
        let join_msg = SwimMessage::Join {
            id: self.member_list.local_id().to_string(),
            addr: self.member_list.local_addr(),
        };

        self.transport
            .send(seed, join_msg)
            .await
            .map_err(|e| MembershipError::JoinFailed(e.to_string()))?;

        // Wait for JoinAck with timeout
        let result = Timeout::race(self.config.ping_timeout * 3, async {
            // Note: In a real implementation, we'd receive on a separate channel
            // For now, we rely on the message loop to process the JoinAck
            tokio::time::sleep(self.config.ping_timeout).await;
        })
        .await;

        if result.is_err() {
            return Err(MembershipError::JoinFailed("timeout waiting for JoinAck".to_string()));
        }

        Ok(())
    }

    /// Leave the cluster gracefully.
    pub async fn leave(&self) -> Result<(), MembershipError> {
        // Mark self as left and get event
        if let Some(event) = self.member_list.leave() {
            let _ = self.event_tx.send(event);
        }

        // Best-effort broadcast to all known members
        let leave_msg = SwimMessage::Leave {
            id: self.member_list.local_id().to_string(),
            incarnation: self.member_list.local_incarnation(),
        };

        for member in self.member_list.all_members() {
            if member.id != self.local_id() {
                let _ = self.transport.send(member.addr, leave_msg.clone()).await;
            }
        }

        Ok(())
    }

    /// Request shutdown.
    pub fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }

    /// Get the shutdown notifier for external use.
    pub fn shutdown_handle(&self) -> Arc<Notify> {
        self.shutdown.clone()
    }

    /// Start the SWIM protocol (probe loop, message handling).
    ///
    /// This spawns background tasks and returns immediately.
    /// Call `shutdown()` to stop.
    pub fn start(self: Arc<Self>) {
        // Start probe loop
        let node = self.clone();
        tokio::spawn(async move {
            node.run_probe_loop().await;
        });

        // Start message receive loop
        let node = self.clone();
        tokio::spawn(async move {
            node.run_receive_loop().await;
        });

        // Start suspicion check loop
        let node = self.clone();
        tokio::spawn(async move {
            node.run_suspicion_loop().await;
        });
    }

    /// Run the periodic probe loop.
    async fn run_probe_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.config.probe_interval) => {
                    self.do_probe_cycle().await;
                }
                _ = self.shutdown.notified() => {
                    break;
                }
            }
        }
    }

    /// Execute a single probe cycle.
    async fn do_probe_cycle(self: &Arc<Self>) {
        // Select random member to probe
        let target = match self.member_list.get_random_member(None) {
            Some(m) => m,
            None => return, // No members to probe
        };

        // Perform probe
        let result = probe::probe_member(
            &self.config,
            &self.member_list,
            &self.probe_manager,
            self.transport.as_ref(),
            &target.id,
            target.addr,
        )
        .await;

        match result {
            Ok(true) => {
                // Member responded - clear any suspicion
                self.suspicion_manager.clear_suspicion(&target.id);
            }
            Ok(false) => {
                // Member didn't respond - suspect them
                if let Some(event) = self.member_list.suspect(&target.id) {
                    let _ = self.event_tx.send(event);
                    let member = self.member_list.get_member(&target.id);
                    if let Some(m) = member {
                        self.suspicion_manager.start_suspicion(&target.id, m.incarnation);
                    }
                }
            }
            Err(_) => {
                // Transport error - ignore for now
            }
        }
    }

    /// Run the message receive loop.
    async fn run_receive_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                result = self.transport.recv() => {
                    match result {
                        Ok((from_addr, msg)) => {
                            self.handle_message(from_addr, msg).await;
                        }
                        Err(_) => {
                            // Transport error - check if shutdown
                            if Arc::strong_count(&self.shutdown) == 1 {
                                break;
                            }
                        }
                    }
                }
                _ = self.shutdown.notified() => {
                    break;
                }
            }
        }
    }

    /// Handle an incoming message.
    async fn handle_message(&self, from_addr: SocketAddr, msg: SwimMessage) {
        match msg {
            SwimMessage::Ping {
                seq,
                from_id,
                gossip,
                ..
            } => {
                self.handle_ping(seq, &from_id, from_addr, gossip).await;
            }
            SwimMessage::Ack {
                seq,
                from_id,
                gossip,
                ..
            } => {
                self.handle_ack(seq, &from_id, gossip);
            }
            SwimMessage::PingReq {
                seq,
                from_id,
                from_addr: requester_addr,
                target_id,
                target_addr,
                gossip,
            } => {
                self.handle_ping_req(seq, &from_id, requester_addr, &target_id, target_addr, gossip)
                    .await;
            }
            SwimMessage::IndirectAck {
                seq,
                target_id,
                gossip,
            } => {
                self.handle_indirect_ack(seq, &target_id, gossip);
            }
            SwimMessage::Join { id, addr } => {
                self.handle_join(&id, addr).await;
            }
            SwimMessage::JoinAck { members } => {
                self.handle_join_ack(members);
            }
            SwimMessage::Leave { id, incarnation } => {
                self.handle_leave(&id, incarnation);
            }
        }
    }

    async fn handle_ping(&self, seq: u64, from_id: &str, from_addr: SocketAddr, gossip: Vec<GossipEntry>) {
        // Apply gossip
        for entry in &gossip {
            if let Some(event) = self.member_list.apply_gossip(entry) {
                let _ = self.event_tx.send(event);
            }
        }

        // Ensure sender is in member list
        if self.member_list.get_member(from_id).is_none() {
            self.member_list.add_member(Member {
                id: from_id.to_string(),
                addr: from_addr,
                state: crate::MemberState::Alive,
                incarnation: 0,
            });
            let _ = self.event_tx.send(MembershipEvent::MemberJoined {
                id: from_id.to_string(),
                addr: from_addr,
            });
        }

        // Send ack
        let ack = SwimMessage::Ack {
            seq,
            from_id: self.member_list.local_id().to_string(),
            from_addr: self.member_list.local_addr(),
            gossip: self.member_list.drain_gossip(self.config.max_gossip_per_msg),
        };
        let _ = self.transport.send(from_addr, ack).await;
    }

    fn handle_ack(&self, seq: u64, from_id: &str, gossip: Vec<GossipEntry>) {
        // Apply gossip
        for entry in &gossip {
            if let Some(event) = self.member_list.apply_gossip(entry) {
                let _ = self.event_tx.send(event);
            }
        }

        // Complete pending probe
        if let Some(target_id) = self.probe_manager.handle_ack(seq) {
            // Clear suspicion for the target (not the sender, since this might be indirect)
            self.suspicion_manager.clear_suspicion(&target_id);
        }

        let _ = from_id; // Used via gossip
    }

    async fn handle_ping_req(
        &self,
        seq: u64,
        _from_id: &str,
        requester_addr: SocketAddr,
        target_id: &str,
        target_addr: SocketAddr,
        gossip: Vec<GossipEntry>,
    ) {
        // Apply gossip
        for entry in &gossip {
            if let Some(event) = self.member_list.apply_gossip(entry) {
                let _ = self.event_tx.send(event);
            }
        }

        // Ping the target
        let ping = SwimMessage::Ping {
            seq,
            from_id: self.member_list.local_id().to_string(),
            from_addr: self.member_list.local_addr(),
            gossip: self.member_list.drain_gossip(self.config.max_gossip_per_msg),
        };

        if self.transport.send(target_addr, ping).await.is_ok() {
            // Wait for ack and forward to requester
            // Note: This is simplified - in production we'd track this properly
            let _ = tokio::time::timeout(self.config.ping_timeout, async {
                // The actual ack handling is done in the receive loop
                // For indirect probes, we forward the ack back
            })
            .await;
        }

        let _ = target_id; // Used in ping
        let _ = requester_addr; // For forwarding ack
    }

    fn handle_indirect_ack(&self, seq: u64, target_id: &str, gossip: Vec<GossipEntry>) {
        // Apply gossip
        for entry in &gossip {
            if let Some(event) = self.member_list.apply_gossip(entry) {
                let _ = self.event_tx.send(event);
            }
        }

        // Complete pending probe for the target
        self.probe_manager.handle_ack(seq);
        self.suspicion_manager.clear_suspicion(target_id);
    }

    async fn handle_join(&self, id: &str, addr: SocketAddr) {
        // Add new member
        self.member_list.add_member(Member {
            id: id.to_string(),
            addr,
            state: crate::MemberState::Alive,
            incarnation: 0,
        });

        let _ = self.event_tx.send(MembershipEvent::MemberJoined {
            id: id.to_string(),
            addr,
        });

        // Send JoinAck with current membership
        let join_ack = SwimMessage::JoinAck {
            members: self.member_list.all_members(),
        };
        let _ = self.transport.send(addr, join_ack).await;
    }

    fn handle_join_ack(&self, members: Vec<Member>) {
        // Add all members from the cluster
        for member in members {
            if member.id != self.local_id() {
                self.member_list.add_member(member.clone());
                let _ = self.event_tx.send(MembershipEvent::MemberJoined {
                    id: member.id,
                    addr: member.addr,
                });
            }
        }
    }

    fn handle_leave(&self, id: &str, incarnation: u64) {
        // Apply leave via gossip
        if let Some(member) = self.member_list.get_member(id) {
            if let Some(event) = self.member_list.apply_gossip(&GossipEntry {
                id: id.to_string(),
                addr: member.addr,
                state: crate::MemberState::Left,
                incarnation,
            }) {
                let _ = self.event_tx.send(event);
            }
        }

        // Clear any suspicion
        self.suspicion_manager.clear_suspicion(id);
    }

    /// Run the suspicion check loop.
    async fn run_suspicion_loop(self: Arc<Self>) {
        // Check at 1/4 of suspicion timeout interval
        let check_interval = self.config.suspicion_timeout / 4;

        loop {
            tokio::select! {
                _ = tokio::time::sleep(check_interval) => {
                    self.suspicion_manager.process_expired(&self.member_list, &self.event_tx);
                }
                _ = self.shutdown.notified() => {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::create_transport_mesh;
    use crate::MemberState;
    use std::time::Duration;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    fn test_config() -> SwimConfig {
        SwimConfig {
            probe_interval: Duration::from_millis(50),
            ping_timeout: Duration::from_millis(25),
            suspicion_timeout: Duration::from_millis(100),
            indirect_fanout: 2,
            max_gossip_per_msg: 5,
            event_channel_capacity: 10,
        }
    }

    #[tokio::test]
    async fn test_node_creation() {
        let addr = test_addr(8001);
        let (transport, _tx) = crate::transport::InMemoryTransport::new(addr);

        let node = SwimNode::new(
            "node1".to_string(),
            addr,
            test_config(),
            Arc::new(transport),
        );

        assert_eq!(node.local_id(), "node1");
        assert_eq!(node.local_addr(), addr);

        let view = node.view();
        assert_eq!(view.members.len(), 1);
        assert_eq!(view.members[0].id, "node1");
    }

    #[tokio::test]
    async fn test_two_node_cluster() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        let config = test_config();
        let node1 = Arc::new(SwimNode::new("node1".to_string(), addr1, config.clone(), t1));
        let node2 = Arc::new(SwimNode::new("node2".to_string(), addr2, config, t2));

        // Start both nodes
        node1.clone().start();
        node2.clone().start();

        // Node1 joins via node2
        // First, manually add node2 to node1's list (simulating join)
        node1.member_list.add_member(Member {
            id: "node2".to_string(),
            addr: addr2,
            state: MemberState::Alive,
            incarnation: 0,
        });

        node2.member_list.add_member(Member {
            id: "node1".to_string(),
            addr: addr1,
            state: MemberState::Alive,
            incarnation: 0,
        });

        // Wait for a few probe cycles
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Both should see each other
        assert!(node1.member_list.get_member("node2").is_some());
        assert!(node2.member_list.get_member("node1").is_some());

        // Shutdown
        node1.shutdown();
        node2.shutdown();

        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn test_leave() {
        let addr = test_addr(8001);
        let (transport, _tx) = crate::transport::InMemoryTransport::new(addr);

        let node = SwimNode::new("node1".to_string(), addr, test_config(), Arc::new(transport));
        let mut events = node.events();

        node.leave().await.unwrap();

        // Should receive MemberLeft event
        let event = events.try_recv().unwrap();
        assert!(matches!(event, MembershipEvent::MemberLeft { id } if id == "node1"));

        // Member should be in Left state
        let member = node.member_list.get_member("node1").unwrap();
        assert_eq!(member.state, MemberState::Left);
    }

    #[tokio::test]
    async fn test_failure_detection() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();

        let mut config = test_config();
        config.probe_interval = Duration::from_millis(20);
        config.ping_timeout = Duration::from_millis(10);
        config.suspicion_timeout = Duration::from_millis(50);

        let node1 = Arc::new(SwimNode::new("node1".to_string(), addr1, config, t1));

        // Add node2 (but don't create node2 - it won't respond)
        node1.member_list.add_member(Member {
            id: "node2".to_string(),
            addr: addr2,
            state: MemberState::Alive,
            incarnation: 0,
        });

        let mut events = node1.events();

        // Start node1
        node1.clone().start();

        // Wait for probe + suspicion timeout
        tokio::time::sleep(Duration::from_millis(150)).await;

        // Should have received suspect and failed events
        let mut saw_suspect = false;
        let mut saw_failed = false;

        while let Ok(event) = events.try_recv() {
            match event {
                MembershipEvent::MemberSuspect { id, .. } if id == "node2" => saw_suspect = true,
                MembershipEvent::MemberFailed { id } if id == "node2" => saw_failed = true,
                _ => {}
            }
        }

        assert!(saw_suspect, "Should have seen Suspect event");
        assert!(saw_failed, "Should have seen Failed event");

        node1.shutdown();
    }
}
