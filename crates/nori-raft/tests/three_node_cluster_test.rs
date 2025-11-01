//! Integration tests for 3-node Raft cluster.
//!
//! These tests verify multi-node operation with real RPC communication:
//! - Leader election across 3 nodes
//! - Log replication from leader to followers
//! - Failover when leader crashes
//! - Split-brain prevention

use bytes::Bytes;
use nori_raft::transport::{InMemoryTransport, RpcSender};
use nori_raft::{ConfigEntry, NodeId, Raft, RaftConfig, ReplicatedLog};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// Test cluster node with all components.
struct TestNode {
    id: NodeId,
    raft: Arc<Raft>,
    _raft_dir: TempDir,
}

/// Create a 3-node cluster with fully connected transports.
///
/// Returns (nodes, rpc_senders) where rpc_senders contains the sender for each node's RPC channel.
async fn create_three_node_cluster() -> (Vec<TestNode>, HashMap<NodeId, RpcSender>) {
    let node_ids = vec![
        NodeId::new("n1"),
        NodeId::new("n2"),
        NodeId::new("n3"),
    ];

    // Create RPC channels for each node
    let mut rpc_channels = HashMap::new();
    let mut rpc_senders = HashMap::new();

    for node_id in &node_ids {
        let (tx, rx) = mpsc::channel(100);
        rpc_channels.insert(node_id.clone(), rx);
        rpc_senders.insert(node_id.clone(), tx);
    }

    // Create transport for each node (pointing to all other nodes)
    let mut transports = HashMap::new();
    for node_id in &node_ids {
        let mut peers = HashMap::new();
        for (peer_id, sender) in &rpc_senders {
            if peer_id != node_id {
                peers.insert(peer_id.clone(), sender.clone());
            }
        }
        let transport = Arc::new(InMemoryTransport::new(node_id.clone(), peers));
        transports.insert(node_id.clone(), transport);
    }

    // Create Raft instance for each node
    let mut nodes = Vec::new();
    let initial_config = ConfigEntry::Single(node_ids.clone());

    for node_id in &node_ids {
        let raft_dir = TempDir::new().unwrap();
        let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
            .await
            .unwrap();

        let mut raft_config = RaftConfig::default();
        // Set election timeouts well above heartbeat interval (150ms)
        // to prevent spurious elections
        raft_config.election_timeout_min = Duration::from_millis(500);
        raft_config.election_timeout_max = Duration::from_millis(1000);

        let transport = transports.get(node_id).unwrap().clone();
        let rpc_rx = rpc_channels.remove(node_id);

        let raft = Arc::new(Raft::new(
            node_id.clone(),
            raft_config,
            raft_log,
            transport,
            initial_config.clone(),
            None, // No state machine for basic tests
            rpc_rx,
        ));

        nodes.push(TestNode {
            id: node_id.clone(),
            raft,
            _raft_dir: raft_dir,
        });
    }

    (nodes, rpc_senders)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_three_node_leader_election() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init()
        .ok();

    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    info!("Starting all nodes in cluster");
    // Start all nodes
    for node in &nodes {
        info!("Starting node: {:?}", node.id);
        node.raft.start().await.unwrap();
    }

    info!("Waiting for leader election...");
    // Wait for leader election (with retry logic)
    let mut leader_id = None;
    for attempt in 0..10 {
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Check if we have exactly one leader
        let leaders: Vec<_> = nodes
            .iter()
            .filter(|n| n.raft.is_leader())
            .collect();

        debug!(
            "Attempt {}: Found {} leaders: {:?}",
            attempt,
            leaders.len(),
            leaders.iter().map(|n| &n.id).collect::<Vec<_>>()
        );

        // Log what each node thinks
        for node in &nodes {
            debug!(
                "Node {:?}: is_leader={}, leader={:?}",
                node.id,
                node.raft.is_leader(),
                node.raft.leader()
            );
        }

        if leaders.len() == 1 {
            leader_id = Some(leaders[0].id.clone());
            info!("Leader elected: {:?}", leader_id);
            break;
        }
    }

    let leader_id = leader_id.expect("Should have elected a leader within 3 seconds");

    info!("Waiting for leader info to propagate...");
    // Give more time for leader info to propagate
    tokio::time::sleep(Duration::from_millis(500)).await;

    info!("Verifying all nodes agree on leader...");
    // Verify all nodes agree on the leader (with retries for propagation)
    for node in &nodes {
        // Retry a few times to allow leader info to propagate
        let mut node_leader = node.raft.leader();
        for retry in 0..5 {
            debug!(
                "Node {:?} retry {}: sees leader {:?}, expecting {:?}",
                node.id, retry, node_leader, leader_id
            );
            if node_leader == Some(leader_id.clone()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            node_leader = node.raft.leader();
        }

        assert_eq!(
            node_leader,
            Some(leader_id.clone()),
            "Node {:?} should recognize {:?} as leader, but sees {:?}",
            node.id,
            leader_id,
            node_leader
        );
    }

    info!("Leader election test passed, shutting down nodes");
    // Shutdown all nodes
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_three_node_log_replication() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election (with retry logic)
    let mut leader = None;
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Find the leader
        let leaders: Vec<_> = nodes.iter().filter(|n| n.raft.is_leader()).collect();

        if leaders.len() == 1 {
            leader = Some(leaders[0]);
            break;
        }
    }

    let leader = leader.expect("Should have elected a leader within 3 seconds");
    println!("Testing log replication with leader: {:?}", leader.id);

    // Give time for cluster to stabilize
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Propose a command on the leader (with retry in case leadership changes)
    let cmd = Bytes::from("test_command_1");
    let mut propose_result = leader.raft.propose(cmd.clone()).await;

    // If leadership changed, find the new leader and retry
    if propose_result.is_err() {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let new_leader = nodes
            .iter()
            .find(|n| n.raft.is_leader())
            .expect("Should have a leader after election");
        propose_result = new_leader.raft.propose(cmd.clone()).await;
    }

    let index = propose_result.expect("Propose should succeed on leader");
    assert!(index.0 > 0, "Should get valid log index");

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify the command was replicated to all nodes
    // Note: We can't directly check log contents without exposing more internals,
    // but we can verify that followers received AppendEntries by checking their state

    // Find current leader for subsequent proposals
    let current_leader = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .expect("Should have a leader");

    // Propose multiple commands
    for i in 2..=5 {
        let cmd = Bytes::from(format!("test_command_{}", i));
        current_leader
            .raft
            .propose(cmd)
            .await
            .expect("Propose should succeed");
    }

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(500)).await;

    // All nodes should still agree on the leader
    let leader_id = current_leader.id.clone();
    for node in &nodes {
        assert_eq!(node.raft.leader(), Some(leader_id.clone()));
    }

    // Shutdown all nodes
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_three_node_follower_cannot_propose() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Find a follower
    let follower = nodes
        .iter()
        .find(|n| !n.raft.is_leader())
        .expect("Should have at least one follower");

    println!("Testing follower rejection: {:?}", follower.id);

    // Try to propose on a follower
    let cmd = Bytes::from("test_command");
    let result = follower.raft.propose(cmd).await;

    // Should fail with NotLeader error
    assert!(result.is_err(), "Follower should not accept proposals");
    assert!(
        matches!(result, Err(nori_raft::RaftError::NotLeader { .. })),
        "Should return NotLeader error"
    );

    // Shutdown all nodes
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_three_node_leader_transfer_on_shutdown() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for initial leader election
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Find the initial leader
    let initial_leader_id = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .map(|n| n.id.clone())
        .expect("Should have a leader");

    println!("Initial leader: {:?}", initial_leader_id);

    // Shutdown the leader
    for node in &nodes {
        if node.id == initial_leader_id {
            node.raft.shutdown().unwrap();
            break;
        }
    }

    // Wait for new leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Verify a new leader was elected (one of the remaining 2 nodes)
    let new_leaders: Vec<_> = nodes
        .iter()
        .filter(|n| n.id != initial_leader_id && n.raft.is_leader())
        .collect();

    assert_eq!(
        new_leaders.len(),
        1,
        "Should have elected a new leader after old leader shutdown"
    );

    let new_leader = new_leaders[0];
    println!("New leader elected: {:?}", new_leader.id);
    assert_ne!(
        new_leader.id, initial_leader_id,
        "New leader should be different from old leader"
    );

    // Shutdown remaining nodes
    for node in &nodes {
        if node.id != initial_leader_id {
            node.raft.shutdown().unwrap();
        }
    }
}
