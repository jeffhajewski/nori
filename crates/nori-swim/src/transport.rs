//! Transport abstraction for SWIM protocol communication.
//!
//! Defines the `SwimTransport` trait for pluggable transport implementations:
//! - In-memory channels for unit testing
//! - UDP transport for production
//!
//! All operations are async and return `Result<T, SwimError>`.

use crate::message::SwimMessage;
use crate::SwimError;
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Transport abstraction for SWIM protocol communication.
///
/// Implementations handle:
/// - Message serialization/deserialization
/// - Network I/O (UDP, in-memory channels, etc.)
/// - Error handling for network failures
#[async_trait]
pub trait SwimTransport: Send + Sync + 'static {
    /// Send a message to a target address.
    async fn send(&self, target: SocketAddr, msg: SwimMessage) -> Result<(), SwimError>;

    /// Receive a message (blocking until one arrives).
    async fn recv(&self) -> Result<(SocketAddr, SwimMessage), SwimError>;

    /// Get the local address this transport is bound to.
    fn local_addr(&self) -> SocketAddr;
}

/// In-memory transport for testing.
///
/// Uses tokio channels to simulate network communication without actual I/O.
/// Useful for:
/// - Unit tests (deterministic, no network)
/// - Integration tests (multi-node clusters in-process)
/// - Chaos tests (controlled message loss/delay)
pub struct InMemoryTransport {
    /// This node's address
    local_addr: SocketAddr,

    /// Channels to other nodes (address → sender)
    peers: Arc<parking_lot::RwLock<HashMap<SocketAddr, mpsc::Sender<(SocketAddr, SwimMessage)>>>>,

    /// Receiver for incoming messages
    rx: Arc<tokio::sync::Mutex<mpsc::Receiver<(SocketAddr, SwimMessage)>>>,
}

impl InMemoryTransport {
    /// Create a new in-memory transport.
    ///
    /// Returns the transport and a sender that can be shared with other transports.
    pub fn new(local_addr: SocketAddr) -> (Self, mpsc::Sender<(SocketAddr, SwimMessage)>) {
        let (tx, rx) = mpsc::channel(100);

        let transport = Self {
            local_addr,
            peers: Arc::new(parking_lot::RwLock::new(HashMap::new())),
            rx: Arc::new(tokio::sync::Mutex::new(rx)),
        };

        (transport, tx)
    }

    /// Add a peer's sender to this transport.
    pub fn add_peer(&self, addr: SocketAddr, sender: mpsc::Sender<(SocketAddr, SwimMessage)>) {
        self.peers.write().insert(addr, sender);
    }

    /// Remove a peer from this transport.
    pub fn remove_peer(&self, addr: &SocketAddr) {
        self.peers.write().remove(addr);
    }

    /// Check if a peer is connected.
    pub fn has_peer(&self, addr: &SocketAddr) -> bool {
        self.peers.read().contains_key(addr)
    }

    /// Get the number of connected peers.
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }
}

#[async_trait]
impl SwimTransport for InMemoryTransport {
    async fn send(&self, target: SocketAddr, msg: SwimMessage) -> Result<(), SwimError> {
        let sender = {
            let peers = self.peers.read();
            peers.get(&target).cloned()
        };

        match sender {
            Some(tx) => {
                tx.send((self.local_addr, msg))
                    .await
                    .map_err(|_| SwimError::Transport("peer channel closed".to_string()))?;
                Ok(())
            }
            None => Err(SwimError::Transport(format!(
                "peer not found: {}",
                target
            ))),
        }
    }

    async fn recv(&self) -> Result<(SocketAddr, SwimMessage), SwimError> {
        let mut rx = self.rx.lock().await;
        rx.recv()
            .await
            .ok_or_else(|| SwimError::Transport("receive channel closed".to_string()))
    }

    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// Create a mesh of connected in-memory transports.
///
/// Returns a map of address → transport, where each transport can communicate
/// with all others.
pub fn create_transport_mesh(
    addrs: Vec<SocketAddr>,
) -> HashMap<SocketAddr, Arc<InMemoryTransport>> {
    // First, create all transports and collect their senders
    let mut transports = HashMap::new();
    let mut senders = HashMap::new();

    for addr in &addrs {
        let (transport, sender) = InMemoryTransport::new(*addr);
        transports.insert(*addr, Arc::new(transport));
        senders.insert(*addr, sender);
    }

    // Connect all transports to each other
    for addr in &addrs {
        let transport = transports.get(addr).unwrap();
        for (peer_addr, sender) in &senders {
            if peer_addr != addr {
                transport.add_peer(*peer_addr, sender.clone());
            }
        }
    }

    transports
}

// ============================================================================
// UDP Transport (Production)
// ============================================================================

/// Maximum UDP datagram size for SWIM messages.
///
/// SWIM messages should be small (< 1400 bytes to avoid fragmentation).
/// We use 65535 as the buffer size to handle any valid UDP datagram.
const MAX_UDP_PACKET_SIZE: usize = 65535;

/// UDP-based transport for production use.
///
/// Uses a single UDP socket for both sending and receiving.
/// Messages are serialized using bincode.
///
/// # Example
///
/// ```ignore
/// let transport = UdpTransport::bind("0.0.0.0:7946").await?;
/// transport.send(peer_addr, message).await?;
/// let (from, msg) = transport.recv().await?;
/// ```
pub struct UdpTransport {
    /// The UDP socket
    socket: Arc<UdpSocket>,

    /// Local address
    local_addr: SocketAddr,
}

impl UdpTransport {
    /// Bind to a local address and create a new UDP transport.
    pub async fn bind(addr: SocketAddr) -> Result<Self, SwimError> {
        let socket = UdpSocket::bind(addr)
            .await
            .map_err(|e| SwimError::Transport(format!("failed to bind UDP socket: {}", e)))?;

        let local_addr = socket
            .local_addr()
            .map_err(|e| SwimError::Transport(format!("failed to get local addr: {}", e)))?;

        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
        })
    }

    /// Create from an existing socket.
    pub fn from_socket(socket: UdpSocket) -> Result<Self, SwimError> {
        let local_addr = socket
            .local_addr()
            .map_err(|e| SwimError::Transport(format!("failed to get local addr: {}", e)))?;

        Ok(Self {
            socket: Arc::new(socket),
            local_addr,
        })
    }

    /// Get a clone of the underlying socket (for advanced use).
    pub fn socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }
}

#[async_trait]
impl SwimTransport for UdpTransport {
    async fn send(&self, target: SocketAddr, msg: SwimMessage) -> Result<(), SwimError> {
        // Serialize the message
        let bytes = msg.encode()?;

        // Check message size
        if bytes.len() > MAX_UDP_PACKET_SIZE {
            return Err(SwimError::Transport(format!(
                "message too large: {} bytes (max {})",
                bytes.len(),
                MAX_UDP_PACKET_SIZE
            )));
        }

        // Send via UDP
        self.socket
            .send_to(&bytes, target)
            .await
            .map_err(|e| SwimError::Transport(format!("UDP send failed: {}", e)))?;

        Ok(())
    }

    async fn recv(&self) -> Result<(SocketAddr, SwimMessage), SwimError> {
        let mut buf = vec![0u8; MAX_UDP_PACKET_SIZE];

        // Receive datagram
        let (len, from) = self
            .socket
            .recv_from(&mut buf)
            .await
            .map_err(|e| SwimError::Transport(format!("UDP recv failed: {}", e)))?;

        // Deserialize the message
        let msg = SwimMessage::decode(&buf[..len])?;

        Ok((from, msg))
    }

    fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

/// Configuration for UDP transport.
#[derive(Debug, Clone)]
pub struct UdpTransportConfig {
    /// Buffer size for receiving messages
    pub recv_buffer_size: usize,

    /// Whether to set SO_REUSEADDR
    pub reuse_addr: bool,
}

impl Default for UdpTransportConfig {
    fn default() -> Self {
        Self {
            recv_buffer_size: MAX_UDP_PACKET_SIZE,
            reuse_addr: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_addr(port: u16) -> SocketAddr {
        format!("127.0.0.1:{}", port).parse().unwrap()
    }

    // ========================================================================
    // InMemoryTransport Tests
    // ========================================================================

    #[tokio::test]
    async fn test_in_memory_transport_send_recv() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let (transport1, sender1) = InMemoryTransport::new(addr1);
        let (transport2, sender2) = InMemoryTransport::new(addr2);

        // Connect transports
        transport1.add_peer(addr2, sender2);
        transport2.add_peer(addr1, sender1);

        // Send from transport1 to transport2
        let msg = SwimMessage::Ping {
            seq: 42,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };

        transport1.send(addr2, msg.clone()).await.unwrap();

        // Receive on transport2
        let (from, received) = transport2.recv().await.unwrap();
        assert_eq!(from, addr1);
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn test_in_memory_transport_peer_not_found() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let (transport1, _sender1) = InMemoryTransport::new(addr1);

        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };

        let result = transport1.send(addr2, msg).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SwimError::Transport(_)));
    }

    #[tokio::test]
    async fn test_local_addr() {
        let addr = test_addr(8001);
        let (transport, _sender) = InMemoryTransport::new(addr);
        assert_eq!(transport.local_addr(), addr);
    }

    #[tokio::test]
    async fn test_peer_management() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let (transport1, _sender1) = InMemoryTransport::new(addr1);
        let (_transport2, sender2) = InMemoryTransport::new(addr2);

        assert_eq!(transport1.peer_count(), 0);
        assert!(!transport1.has_peer(&addr2));

        transport1.add_peer(addr2, sender2);
        assert_eq!(transport1.peer_count(), 1);
        assert!(transport1.has_peer(&addr2));

        transport1.remove_peer(&addr2);
        assert_eq!(transport1.peer_count(), 0);
        assert!(!transport1.has_peer(&addr2));
    }

    #[tokio::test]
    async fn test_transport_mesh() {
        let addrs: Vec<SocketAddr> = (8001..=8003).map(test_addr).collect();
        let mesh = create_transport_mesh(addrs.clone());

        assert_eq!(mesh.len(), 3);

        // Each transport should have 2 peers
        for transport in mesh.values() {
            assert_eq!(transport.peer_count(), 2);
        }

        // Send message through mesh
        let t1 = mesh.get(&addrs[0]).unwrap();
        let t2 = mesh.get(&addrs[1]).unwrap();

        let msg = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addrs[0],
            gossip: vec![],
        };

        t1.send(addrs[1], msg.clone()).await.unwrap();

        let (from, received) = t2.recv().await.unwrap();
        assert_eq!(from, addrs[0]);
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn test_bidirectional_communication() {
        let addr1 = test_addr(8001);
        let addr2 = test_addr(8002);

        let mesh = create_transport_mesh(vec![addr1, addr2]);
        let t1 = mesh.get(&addr1).unwrap().clone();
        let t2 = mesh.get(&addr2).unwrap().clone();

        // Send ping from t1 to t2
        let ping = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };
        t1.send(addr2, ping).await.unwrap();

        // Receive ping on t2
        let (from, _msg) = t2.recv().await.unwrap();
        assert_eq!(from, addr1);

        // Send ack from t2 to t1
        let ack = SwimMessage::Ack {
            seq: 1,
            from_id: "node2".to_string(),
            from_addr: addr2,
            gossip: vec![],
        };
        t2.send(addr1, ack).await.unwrap();

        // Receive ack on t1
        let (from, msg) = t1.recv().await.unwrap();
        assert_eq!(from, addr2);
        assert!(matches!(msg, SwimMessage::Ack { seq: 1, .. }));
    }

    // ========================================================================
    // UdpTransport Tests
    // ========================================================================

    #[tokio::test]
    async fn test_udp_transport_bind() {
        // Bind to ephemeral port
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let transport = UdpTransport::bind(addr).await.unwrap();

        // Should have been assigned a real port
        assert_ne!(transport.local_addr().port(), 0);
    }

    #[tokio::test]
    async fn test_udp_transport_send_recv() {
        // Create two transports on ephemeral ports
        let t1 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let t2 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let addr1 = t1.local_addr();
        let addr2 = t2.local_addr();

        // Send from t1 to t2
        let msg = SwimMessage::Ping {
            seq: 42,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };
        t1.send(addr2, msg.clone()).await.unwrap();

        // Receive on t2
        let (from, received) = t2.recv().await.unwrap();
        assert_eq!(from, addr1);
        assert_eq!(received, msg);
    }

    #[tokio::test]
    async fn test_udp_transport_bidirectional() {
        let t1 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let t2 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let addr1 = t1.local_addr();
        let addr2 = t2.local_addr();

        // Send ping t1 -> t2
        let ping = SwimMessage::Ping {
            seq: 1,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: vec![],
        };
        t1.send(addr2, ping).await.unwrap();

        // Receive ping on t2
        let (from, _) = t2.recv().await.unwrap();
        assert_eq!(from, addr1);

        // Send ack t2 -> t1
        let ack = SwimMessage::Ack {
            seq: 1,
            from_id: "node2".to_string(),
            from_addr: addr2,
            gossip: vec![],
        };
        t2.send(addr1, ack).await.unwrap();

        // Receive ack on t1
        let (from, msg) = t1.recv().await.unwrap();
        assert_eq!(from, addr2);
        assert!(matches!(msg, SwimMessage::Ack { seq: 1, .. }));
    }

    #[tokio::test]
    async fn test_udp_transport_with_gossip() {
        use crate::{GossipEntry, MemberState};

        let t1 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let t2 = UdpTransport::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();

        let addr1 = t1.local_addr();
        let addr2 = t2.local_addr();

        // Send message with gossip payload
        let gossip = vec![
            GossipEntry {
                id: "node3".to_string(),
                addr: "127.0.0.1:9003".parse().unwrap(),
                state: MemberState::Alive,
                incarnation: 5,
            },
            GossipEntry {
                id: "node4".to_string(),
                addr: "127.0.0.1:9004".parse().unwrap(),
                state: MemberState::Suspect,
                incarnation: 2,
            },
        ];

        let msg = SwimMessage::Ping {
            seq: 100,
            from_id: "node1".to_string(),
            from_addr: addr1,
            gossip: gossip.clone(),
        };

        t1.send(addr2, msg).await.unwrap();

        let (_, received) = t2.recv().await.unwrap();
        if let SwimMessage::Ping { gossip: recv_gossip, .. } = received {
            assert_eq!(recv_gossip.len(), 2);
            assert_eq!(recv_gossip[0].id, "node3");
            assert_eq!(recv_gossip[1].id, "node4");
        } else {
            panic!("Expected Ping message");
        }
    }
}
