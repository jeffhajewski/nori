//! Probing logic for failure detection.
//!
//! Implements the SWIM probe cycle:
//! 1. Select random member
//! 2. Send direct Ping, wait for Ack
//! 3. If no Ack, send indirect PingReq to k random members
//! 4. If still no Ack, mark target as Suspect

use crate::config::SwimConfig;
use crate::member_list::MemberList;
use crate::message::{GossipEntry, SwimMessage};
use crate::timer::Timeout;
use crate::transport::SwimTransport;
use crate::SwimError;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Tracks pending probe state.
#[derive(Debug)]
pub struct PendingProbe {
    /// Target member ID
    pub target_id: String,
    /// Target address
    pub target_addr: SocketAddr,
    /// Whether indirect probes have been sent
    pub indirect_sent: bool,
    /// Sender to notify when ack received
    pub ack_tx: mpsc::Sender<()>,
}

/// Manages probe lifecycle and sequence numbers.
pub struct ProbeManager {
    /// Sequence number counter
    seq_counter: AtomicU64,

    /// Pending probes (seq → probe state)
    pending: RwLock<HashMap<u64, PendingProbe>>,
}

impl ProbeManager {
    /// Create a new probe manager.
    pub fn new() -> Self {
        Self {
            seq_counter: AtomicU64::new(1),
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Generate the next sequence number.
    pub fn next_seq(&self) -> u64 {
        self.seq_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Register a pending probe.
    pub fn register(&self, seq: u64, probe: PendingProbe) {
        self.pending.write().insert(seq, probe);
    }

    /// Mark that indirect probes have been sent for a sequence.
    pub fn mark_indirect_sent(&self, seq: u64) {
        if let Some(probe) = self.pending.write().get_mut(&seq) {
            probe.indirect_sent = true;
        }
    }

    /// Handle an ack for a sequence number.
    ///
    /// Returns the target ID if this was a valid pending probe.
    pub fn handle_ack(&self, seq: u64) -> Option<String> {
        if let Some(probe) = self.pending.write().remove(&seq) {
            // Notify waiters
            let _ = probe.ack_tx.try_send(());
            Some(probe.target_id)
        } else {
            None
        }
    }

    /// Remove a pending probe (e.g., after timeout).
    pub fn remove(&self, seq: u64) -> Option<PendingProbe> {
        self.pending.write().remove(&seq)
    }

    /// Check if a sequence is pending.
    pub fn is_pending(&self, seq: u64) -> bool {
        self.pending.read().contains_key(&seq)
    }

    /// Get count of pending probes.
    pub fn pending_count(&self) -> usize {
        self.pending.read().len()
    }
}

impl Default for ProbeManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Execute a single probe cycle.
///
/// Returns `Ok(true)` if the target responded, `Ok(false)` if it should be suspected.
pub async fn probe_member<T: SwimTransport>(
    config: &SwimConfig,
    member_list: &MemberList,
    probe_manager: &ProbeManager,
    transport: &T,
    target_id: &str,
    target_addr: SocketAddr,
) -> Result<bool, SwimError> {
    let seq = probe_manager.next_seq();
    let (ack_tx, mut ack_rx) = mpsc::channel(1);

    // Register probe
    probe_manager.register(
        seq,
        PendingProbe {
            target_id: target_id.to_string(),
            target_addr,
            indirect_sent: false,
            ack_tx,
        },
    );

    // Build ping message with gossip
    let gossip = member_list.drain_gossip(config.max_gossip_per_msg);
    let ping = SwimMessage::Ping {
        seq,
        from_id: member_list.local_id().to_string(),
        from_addr: member_list.local_addr(),
        gossip,
    };

    // Send direct ping
    transport.send(target_addr, ping).await?;

    // Wait for direct ack
    let direct_result = Timeout::race(config.ping_timeout, ack_rx.recv()).await;

    if direct_result.is_ok() {
        // Got direct ack
        probe_manager.remove(seq);
        return Ok(true);
    }

    // Direct ping failed - try indirect probes
    probe_manager.mark_indirect_sent(seq);

    // Select k random members for indirect probe
    let indirect_targets = member_list.get_random_members(
        config.indirect_fanout,
        &[target_id, member_list.local_id()],
    );

    if indirect_targets.is_empty() {
        // No other members to ask
        probe_manager.remove(seq);
        return Ok(false);
    }

    // Send indirect probe requests
    let gossip = member_list.drain_gossip(config.max_gossip_per_msg);
    for helper in &indirect_targets {
        let ping_req = SwimMessage::PingReq {
            seq,
            from_id: member_list.local_id().to_string(),
            from_addr: member_list.local_addr(),
            target_id: target_id.to_string(),
            target_addr,
            gossip: gossip.clone(),
        };
        // Best effort - don't fail on send errors
        let _ = transport.send(helper.addr, ping_req).await;
    }

    // Wait for indirect ack
    let indirect_result = Timeout::race(config.ping_timeout, ack_rx.recv()).await;

    probe_manager.remove(seq);

    Ok(indirect_result.is_ok())
}

/// Handle incoming ping request (indirect probe).
pub async fn handle_ping_req<T: SwimTransport>(
    member_list: &MemberList,
    transport: &T,
    seq: u64,
    from_addr: SocketAddr,
    target_id: &str,
    target_addr: SocketAddr,
    gossip: Vec<GossipEntry>,
    max_gossip: usize,
) -> Result<(), SwimError> {
    // Apply received gossip
    for entry in gossip {
        member_list.apply_gossip(&entry);
    }

    // Send ping to target
    let ping = SwimMessage::Ping {
        seq,
        from_id: member_list.local_id().to_string(),
        from_addr: member_list.local_addr(),
        gossip: member_list.drain_gossip(max_gossip),
    };
    transport.send(target_addr, ping).await?;

    // Wait briefly for ack
    // Note: In a real implementation, we'd track this properly
    // For now, we assume the original sender will handle the indirect ack
    let _ = target_id; // Silence unused warning
    let _ = from_addr;

    Ok(())
}

/// Handle incoming ping.
pub async fn handle_ping<T: SwimTransport>(
    member_list: &MemberList,
    transport: &T,
    seq: u64,
    from_id: &str,
    from_addr: SocketAddr,
    gossip: Vec<GossipEntry>,
    max_gossip: usize,
) -> Result<(), SwimError> {
    // Apply received gossip
    for entry in gossip {
        member_list.apply_gossip(&entry);
    }

    // Update sender in member list if new
    if member_list.get_member(from_id).is_none() {
        member_list.apply_gossip(&GossipEntry {
            id: from_id.to_string(),
            addr: from_addr,
            state: crate::MemberState::Alive,
            incarnation: 0,
        });
    }

    // Send ack
    let ack = SwimMessage::Ack {
        seq,
        from_id: member_list.local_id().to_string(),
        from_addr: member_list.local_addr(),
        gossip: member_list.drain_gossip(max_gossip),
    };
    transport.send(from_addr, ack).await?;

    Ok(())
}

/// Handle incoming ack.
pub fn handle_ack(
    member_list: &MemberList,
    probe_manager: &ProbeManager,
    seq: u64,
    from_id: &str,
    gossip: Vec<GossipEntry>,
) {
    // Apply received gossip
    for entry in gossip {
        member_list.apply_gossip(&entry);
    }

    // Complete the pending probe
    probe_manager.handle_ack(seq);

    // Update sender if new
    let _ = from_id; // Used implicitly via gossip
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{create_transport_mesh, InMemoryTransport};
    use crate::MemberState;
    use std::sync::Arc;
    use std::time::Duration;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    fn test_config() -> SwimConfig {
        SwimConfig {
            probe_interval: Duration::from_millis(100),
            ping_timeout: Duration::from_millis(50),
            suspicion_timeout: Duration::from_millis(200),
            indirect_fanout: 2,
            max_gossip_per_msg: 5,
            event_channel_capacity: 10,
        }
    }

    #[test]
    fn test_probe_manager_sequence() {
        let pm = ProbeManager::new();
        assert_eq!(pm.next_seq(), 1);
        assert_eq!(pm.next_seq(), 2);
        assert_eq!(pm.next_seq(), 3);
    }

    #[test]
    fn test_probe_manager_register_and_ack() {
        let pm = ProbeManager::new();
        let (tx, _rx) = mpsc::channel(1);

        pm.register(1, PendingProbe {
            target_id: "node2".to_string(),
            target_addr: test_addr(8002),
            indirect_sent: false,
            ack_tx: tx,
        });

        assert!(pm.is_pending(1));
        assert_eq!(pm.pending_count(), 1);

        let target = pm.handle_ack(1);
        assert_eq!(target, Some("node2".to_string()));
        assert!(!pm.is_pending(1));
        assert_eq!(pm.pending_count(), 0);
    }

    #[tokio::test]
    async fn test_probe_success() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        let config = test_config();
        let member_list1 = Arc::new(MemberList::new("node1".to_string(), addr1));
        let member_list2 = Arc::new(MemberList::new("node2".to_string(), addr2));

        // Add node2 to node1's list
        member_list1.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: addr2,
            state: MemberState::Alive,
            incarnation: 0,
        });

        let probe_manager = Arc::new(ProbeManager::new());

        // Spawn responder on node2 (handles pings)
        let t2_clone = t2.clone();
        let ml2 = member_list2.clone();
        let responder = tokio::spawn(async move {
            loop {
                match t2_clone.recv().await {
                    Ok((from_addr, msg)) => {
                        if let SwimMessage::Ping { seq, from_id, gossip, .. } = msg {
                            handle_ping(
                                &ml2,
                                t2_clone.as_ref(),
                                seq,
                                &from_id,
                                from_addr,
                                gossip,
                                5,
                            ).await.unwrap();
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn receiver on node1 (handles acks)
        let t1_recv = t1.clone();
        let ml1 = member_list1.clone();
        let pm1 = probe_manager.clone();
        let receiver = tokio::spawn(async move {
            loop {
                match t1_recv.recv().await {
                    Ok((_from_addr, msg)) => {
                        if let SwimMessage::Ack { seq, from_id, gossip, .. } = msg {
                            handle_ack(&ml1, &pm1, seq, &from_id, gossip);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Give responders time to start
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Probe node2 from node1
        let result = probe_member(
            &config,
            &member_list1,
            &probe_manager,
            t1.as_ref(),
            "node2",
            addr2,
        ).await.unwrap();

        assert!(result, "Probe should succeed");

        // Clean up
        responder.abort();
        receiver.abort();
    }

    #[tokio::test]
    async fn test_probe_timeout() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        // Only create transport for node1 (node2 won't respond)
        let (t1, _tx1) = InMemoryTransport::new(addr1);

        // Add a non-responding peer
        let (tx2, _rx2) = mpsc::channel(100);
        t1.add_peer(addr2, tx2);

        let config = test_config();
        let member_list = MemberList::new("node1".to_string(), addr1);
        member_list.apply_gossip(&GossipEntry {
            id: "node2".to_string(),
            addr: addr2,
            state: MemberState::Alive,
            incarnation: 0,
        });

        let probe_manager = ProbeManager::new();

        // Probe should fail (no response)
        let result = probe_member(
            &config,
            &member_list,
            &probe_manager,
            &t1,
            "node2",
            addr2,
        ).await.unwrap();

        assert!(!result, "Probe should fail due to timeout");
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        let member_list2 = MemberList::new("node2".to_string(), addr2);

        // Handle ping from node1
        handle_ping(
            &member_list2,
            t2.as_ref(),
            42,
            "node1",
            addr1,
            vec![],
            5,
        ).await.unwrap();

        // Node1 should receive ack
        let (from, msg) = t1.recv().await.unwrap();
        assert_eq!(from, addr2);
        assert!(matches!(msg, SwimMessage::Ack { seq: 42, .. }));

        // Node1 should be added to node2's member list
        assert!(member_list2.get_member("node1").is_some());
    }
}
