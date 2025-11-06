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

// ============================================================================
// Advanced Multi-Node Tests with Network Partitions
// ============================================================================

/// Simulates a network partition by removing peers from each other's transports.
///
/// Blocks bidirectional communication between `partition_a` and `partition_b`.
/// Nodes within each partition can still communicate with each other.
///
/// # Example
/// ```ignore
/// // Create partition: [n1] | [n2, n3]
/// create_network_partition(&nodes, &[n1_id], &[n2_id, n3_id]);
/// ```
fn create_network_partition(
    nodes: &[TestNode],
    partition_a: &[NodeId],
    partition_b: &[NodeId],
) {
    // Remove B's peers from A's transports
    for node in nodes {
        if partition_a.contains(&node.id) {
            // This node is in partition A - remove all B peers
            for peer_id in partition_b {
                if let Some(transport) = node.raft.transport_as_inmemory() {
                    transport.remove_peer(peer_id);
                    tracing::info!("Partition: {:?} can no longer reach {:?}", node.id, peer_id);
                }
            }
        } else if partition_b.contains(&node.id) {
            // This node is in partition B - remove all A peers
            for peer_id in partition_a {
                if let Some(transport) = node.raft.transport_as_inmemory() {
                    transport.remove_peer(peer_id);
                    tracing::info!("Partition: {:?} can no longer reach {:?}", node.id, peer_id);
                }
            }
        }
    }
}

/// Heals a network partition by restoring peer connections.
///
/// Restores bidirectional communication between previously partitioned node sets.
fn heal_network_partition(
    nodes: &[TestNode],
    rpc_senders: &HashMap<NodeId, RpcSender>,
    partition_a: &[NodeId],
    partition_b: &[NodeId],
) {
    // Restore B's peers to A's transports
    for node in nodes {
        if partition_a.contains(&node.id) {
            // This node is in partition A - restore all B peers
            for peer_id in partition_b {
                if let Some(sender) = rpc_senders.get(peer_id) {
                    if let Some(transport) = node.raft.transport_as_inmemory() {
                        transport.add_peer(peer_id.clone(), sender.clone());
                        tracing::info!("Heal: {:?} can now reach {:?}", node.id, peer_id);
                    }
                }
            }
        } else if partition_b.contains(&node.id) {
            // This node is in partition B - restore all A peers
            for peer_id in partition_a {
                if let Some(sender) = rpc_senders.get(peer_id) {
                    if let Some(transport) = node.raft.transport_as_inmemory() {
                        transport.add_peer(peer_id.clone(), sender.clone());
                        tracing::info!("Heal: {:?} can now reach {:?}", node.id, peer_id);
                    }
                }
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_split_brain_prevention() {
    // This test verifies that at most one leader exists per term,
    // even when the cluster experiences issues

    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for initial leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Verify exactly one leader
    let leaders: Vec<_> = nodes.iter().filter(|n| n.raft.is_leader()).collect();
    assert_eq!(
        leaders.len(),
        1,
        "Should have exactly one leader, found {}",
        leaders.len()
    );

    // Over multiple election cycles, verify we never see split-brain
    for cycle in 0..5 {
        tokio::time::sleep(Duration::from_millis(300)).await;

        let current_leaders: Vec<_> = nodes.iter().filter(|n| n.raft.is_leader()).collect();

        assert!(
            current_leaders.len() <= 1,
            "Cycle {}: Split-brain detected! Found {} leaders: {:?}",
            cycle,
            current_leaders.len(),
            current_leaders.iter().map(|n| &n.id).collect::<Vec<_>>()
        );

        if !current_leaders.is_empty() {
            println!("Cycle {}: Leader is {:?}", cycle, current_leaders[0].id);
        }
    }

    // Shutdown
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_leader_crash_and_recovery() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Find the initial leader
    let initial_leader_id = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .map(|n| n.id.clone())
        .expect("Should have a leader");

    info!("Initial leader: {:?}", initial_leader_id);

    // Propose some commands on the initial leader
    for i in 1..=3 {
        let cmd = Bytes::from(format!("cmd_before_crash_{}", i));
        nodes
            .iter()
            .find(|n| n.id == initial_leader_id)
            .unwrap()
            .raft
            .propose(cmd)
            .await
            .expect("Propose should succeed");
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Crash the leader
    info!("Crashing leader {:?}", initial_leader_id);
    for node in &nodes {
        if node.id == initial_leader_id {
            node.raft.shutdown().unwrap();
            break;
        }
    }

    // Wait for new leader election (followers have longer timeout)
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Verify a new leader was elected
    let new_leaders: Vec<_> = nodes
        .iter()
        .filter(|n| n.id != initial_leader_id && n.raft.is_leader())
        .collect();

    assert_eq!(
        new_leaders.len(),
        1,
        "Should have elected exactly one new leader"
    );

    let new_leader = new_leaders[0];
    info!("New leader elected: {:?}", new_leader.id);

    // Propose commands on the new leader to verify it's operational
    for i in 1..=3 {
        let cmd = Bytes::from(format!("cmd_after_recovery_{}", i));
        new_leader
            .raft
            .propose(cmd)
            .await
            .expect("Propose should succeed on new leader");
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify the remaining follower also recognizes the new leader
    for node in &nodes {
        if node.id != initial_leader_id && node.id != new_leader.id {
            assert_eq!(
                node.raft.leader(),
                Some(new_leader.id.clone()),
                "Follower should recognize new leader"
            );
        }
    }

    // Shutdown remaining nodes
    for node in &nodes {
        if node.id != initial_leader_id {
            node.raft.shutdown().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_follower_crash_and_rejoin() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Find leader and a follower
    let leader = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .expect("Should have a leader");

    let follower_id = nodes
        .iter()
        .find(|n| !n.raft.is_leader())
        .map(|n| n.id.clone())
        .expect("Should have at least one follower");

    info!("Leader: {:?}, Crashing follower: {:?}", leader.id, follower_id);

    // Crash a follower
    for node in &nodes {
        if node.id == follower_id {
            node.raft.shutdown().unwrap();
            break;
        }
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Leader should still be operational with 2/3 nodes
    // Propose commands while follower is down
    for i in 1..=5 {
        let cmd = Bytes::from(format!("cmd_during_follower_down_{}", i));
        leader
            .raft
            .propose(cmd)
            .await
            .expect("Leader should still accept proposals with quorum");
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Verify leader is still the same
    assert!(leader.raft.is_leader(), "Leader should remain leader");

    // Note: In a full implementation, we would restart the follower here
    // and verify it catches up via log replication or snapshot
    // For now, we verify the cluster continues operating

    // Shutdown remaining nodes
    for node in &nodes {
        if node.id != follower_id {
            node.raft.shutdown().unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent_proposals() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Find the leader
    let leader = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .expect("Should have a leader");

    info!("Testing concurrent proposals on leader: {:?}", leader.id);

    // Spawn multiple tasks that propose concurrently
    let mut handles = Vec::new();
    for task_id in 0..5 {
        let raft = leader.raft.clone();
        let handle = tokio::spawn(async move {
            let mut successes = 0;
            for i in 0..10 {
                let cmd = Bytes::from(format!("task_{}_cmd_{}", task_id, i));
                if raft.propose(cmd).await.is_ok() {
                    successes += 1;
                }
                // Small delay between proposals
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            successes
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    let mut total_successes = 0;
    for handle in handles {
        let successes = handle.await.unwrap();
        total_successes += successes;
    }

    info!("Total successful proposals: {}/50", total_successes);

    // We expect most proposals to succeed (some may fail due to leadership changes)
    assert!(
        total_successes >= 40,
        "Should have at least 40/50 successful proposals, got {}",
        total_successes
    );

    // Shutdown
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_network_partition_and_recovery() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (nodes, rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for initial leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let initial_leader_id = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .map(|n| n.id.clone())
        .expect("Should have a leader");

    info!("Initial leader: {:?}", initial_leader_id);

    // Propose some commands before partition
    let leader = nodes.iter().find(|n| n.id == initial_leader_id).unwrap();
    for i in 1..=5 {
        leader
            .raft
            .propose(Bytes::from(format!("before_partition_{}", i)))
            .await
            .expect("Propose should succeed");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create partition: [leader] | [follower1, follower2]
    // This simulates a network split where the leader is isolated
    let follower_ids: Vec<NodeId> = nodes
        .iter()
        .filter(|n| n.id != initial_leader_id)
        .map(|n| n.id.clone())
        .collect();

    info!(
        "Creating partition: [{:?}] | [{:?}, {:?}]",
        initial_leader_id, follower_ids[0], follower_ids[1]
    );

    create_network_partition(&nodes, &[initial_leader_id.clone()], &follower_ids);

    // Wait for new election in majority partition (follower1, follower2)
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // The two followers should elect a new leader
    let new_leaders: Vec<_> = nodes
        .iter()
        .filter(|n| follower_ids.contains(&n.id) && n.raft.is_leader())
        .collect();

    assert_eq!(
        new_leaders.len(),
        1,
        "Majority partition should have elected a new leader"
    );

    let new_leader = new_leaders[0];
    info!("New leader in majority partition: {:?}", new_leader.id);

    // Propose commands on new leader
    for i in 1..=5 {
        new_leader
            .raft
            .propose(Bytes::from(format!("during_partition_{}", i)))
            .await
            .expect("New leader should accept proposals");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Heal the partition
    info!("Healing network partition");
    heal_network_partition(&nodes, &rpc_senders, &[initial_leader_id.clone()], &follower_ids);

    // Wait for cluster to stabilize
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // After healing, there should still be exactly one leader
    let leaders_after_heal: Vec<_> = nodes.iter().filter(|n| n.raft.is_leader()).collect();

    assert_eq!(
        leaders_after_heal.len(),
        1,
        "After partition heals, should have exactly one leader"
    );

    // The old leader should have stepped down and recognized the new leader
    let old_leader_node = nodes.iter().find(|n| n.id == initial_leader_id).unwrap();
    assert!(
        !old_leader_node.raft.is_leader(),
        "Old isolated leader should step down after partition heals"
    );

    // Verify new proposals work on the final leader
    let final_leader = leaders_after_heal[0];
    info!("Final leader after recovery: {:?}", final_leader.id);

    for i in 1..=3 {
        final_leader
            .raft
            .propose(Bytes::from(format!("after_recovery_{}", i)))
            .await
            .expect("Final leader should accept proposals");
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // All nodes should agree on the final leader
    for node in &nodes {
        assert_eq!(
            node.raft.leader(),
            Some(final_leader.id.clone()),
            "Node {:?} should recognize {:?} as leader",
            node.id,
            final_leader.id
        );
    }

    info!("Partition recovery test completed successfully");

    // Shutdown
    for node in &nodes {
        node.raft.shutdown().unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_majority_quorum_requirement() {
    let (nodes, _rpc_senders) = create_three_node_cluster().await;

    // Start all nodes
    for node in &nodes {
        node.raft.start().await.unwrap();
    }

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Find the leader
    let leader_id = nodes
        .iter()
        .find(|n| n.raft.is_leader())
        .map(|n| n.id.clone())
        .expect("Should have a leader");

    info!("Leader: {:?}", leader_id);

    // Propose a command (should succeed with 3 nodes)
    let leader = nodes.iter().find(|n| n.id == leader_id).unwrap();
    let result = leader
        .raft
        .propose(Bytes::from("test_with_all_nodes"))
        .await;
    assert!(result.is_ok(), "Proposal should succeed with all nodes up");

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Shutdown TWO followers (breaking quorum)
    let mut shutdown_count = 0;
    for node in &nodes {
        if node.id != leader_id {
            node.raft.shutdown().unwrap();
            shutdown_count += 1;
            if shutdown_count == 2 {
                break;
            }
        }
    }

    // Give time for leader to detect loss of quorum
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Propose should still work (appends to local log)
    // but commit will not advance (can't be tested without checking internal state)
    let result = leader
        .raft
        .propose(Bytes::from("test_without_quorum"))
        .await;

    // Leader can still accept proposals (they go to local log)
    // In a production system, you'd want timeouts for commit confirmation
    assert!(
        result.is_ok() || matches!(result, Err(nori_raft::RaftError::NotLeader { .. })),
        "Proposal may succeed locally but won't commit without quorum"
    );

    // Shutdown leader
    leader.raft.shutdown().unwrap();
}
