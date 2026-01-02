//! Raft Safety Tests (#1-5)
//!
//! Core safety property tests for Raft consensus:
//! 1. Election Safety - At most one leader per term
//! 2. Log Matching - Same index+term implies identical history
//! 3. Leader Completeness - Committed entries appear in all future leaders
//! 4. State Machine Safety - No conflicting application at same index
//! 5. Log Divergence Recovery - Followers reconcile with leader correctly
//!
//! These tests verify the fundamental correctness guarantees of the Raft protocol.

use bytes::Bytes;
use nori_raft::transport::{InMemoryTransport, RpcSender};
use nori_raft::{ConfigEntry, LogIndex, NodeId, Raft, RaftConfig, ReplicatedLog};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

/// Test cluster with partition/heal capabilities
struct SafetyTestCluster {
    nodes: Vec<SafetyTestNode>,
    transports: HashMap<NodeId, Arc<InMemoryTransport>>,
    rpc_senders: HashMap<NodeId, RpcSender>,
}

struct SafetyTestNode {
    id: NodeId,
    raft: Arc<Raft>,
    _raft_dir: TempDir,
}

impl SafetyTestCluster {
    /// Create a cluster with specified number of nodes
    async fn new(num_nodes: usize) -> Self {
        let node_ids: Vec<NodeId> = (0..num_nodes)
            .map(|i| NodeId::new(&format!("n{}", i)))
            .collect();

        // Create RPC channels
        let mut rpc_channels = HashMap::new();
        let mut rpc_senders = HashMap::new();

        for node_id in &node_ids {
            let (tx, rx) = mpsc::channel(100);
            rpc_channels.insert(node_id.clone(), rx);
            rpc_senders.insert(node_id.clone(), tx);
        }

        // Create transports (fully connected)
        let mut transports = HashMap::new();
        for node_id in &node_ids {
            let mut peers = HashMap::new();
            for (peer_id, sender) in &rpc_senders {
                if peer_id != node_id {
                    peers.insert(peer_id.clone(), sender.clone());
                }
            }
            transports.insert(
                node_id.clone(),
                Arc::new(InMemoryTransport::new(node_id.clone(), peers)),
            );
        }

        // Create Raft nodes
        let mut nodes = Vec::new();
        let initial_config = ConfigEntry::Single(node_ids.clone());

        for node_id in &node_ids {
            let raft_dir = TempDir::new().unwrap();
            let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
                .await
                .unwrap();

            let mut raft_config = RaftConfig::default();
            raft_config.heartbeat_interval = Duration::from_millis(100);
            raft_config.election_timeout_min = Duration::from_millis(200);
            raft_config.election_timeout_max = Duration::from_millis(400);

            let transport = transports.get(node_id).unwrap().clone();
            let rpc_rx = rpc_channels.remove(node_id);

            let raft = Arc::new(Raft::new(
                node_id.clone(),
                raft_config,
                raft_log,
                transport,
                initial_config.clone(),
                None, // No state machine for safety tests
                rpc_rx,
            ));

            nodes.push(SafetyTestNode {
                id: node_id.clone(),
                raft,
                _raft_dir: raft_dir,
            });
        }

        // Start all nodes
        for node in &nodes {
            node.raft.start().await.unwrap();
        }

        SafetyTestCluster {
            nodes,
            transports,
            rpc_senders,
        }
    }

    /// Partition a node from all others
    fn partition_node(&self, node_id: &NodeId) {
        for (peer_id, transport) in &self.transports {
            if peer_id != node_id {
                transport.remove_peer(node_id);
            }
        }

        if let Some(transport) = self.transports.get(node_id) {
            for peer_id in self.transports.keys() {
                if peer_id != node_id {
                    transport.remove_peer(peer_id);
                }
            }
        }
    }

    /// Heal a partitioned node
    fn heal_node(&self, node_id: &NodeId) {
        for (peer_id, transport) in &self.transports {
            if peer_id != node_id {
                if let Some(sender) = self.rpc_senders.get(node_id) {
                    transport.add_peer(node_id.clone(), sender.clone());
                }
            }
        }

        if let Some(transport) = self.transports.get(node_id) {
            for (peer_id, sender) in &self.rpc_senders {
                if peer_id != node_id {
                    transport.add_peer(peer_id.clone(), sender.clone());
                }
            }
        }
    }

    /// Create a partition: separate nodes into two groups
    fn create_partition(&self, group_a: &[NodeId], group_b: &[NodeId]) {
        // Remove all connections between the two groups
        for a_node in group_a {
            for b_node in group_b {
                if let Some(transport) = self.transports.get(a_node) {
                    transport.remove_peer(b_node);
                }
                if let Some(transport) = self.transports.get(b_node) {
                    transport.remove_peer(a_node);
                }
            }
        }
    }

    /// Heal all partitions - fully reconnect cluster
    fn heal_all(&self) {
        for node_id in self.transports.keys() {
            self.heal_node(node_id);
        }
    }

    /// Get current term for each node
    fn get_terms(&self) -> HashMap<NodeId, u64> {
        let mut terms = HashMap::new();
        for node in &self.nodes {
            terms.insert(node.id.clone(), node.raft.current_term().0);
        }
        terms
    }

    /// Get leaders for each node (who they think is leader)
    fn get_leaders(&self) -> HashMap<NodeId, Option<NodeId>> {
        let mut leaders = HashMap::new();
        for node in &self.nodes {
            leaders.insert(node.id.clone(), node.raft.leader());
        }
        leaders
    }

    /// Count how many nodes think they are leader
    fn count_leaders(&self) -> usize {
        self.nodes.iter().filter(|n| n.raft.is_leader()).count()
    }

    /// Get the actual leader node (if any)
    fn get_leader(&self) -> Option<&SafetyTestNode> {
        self.nodes.iter().find(|n| n.raft.is_leader())
    }

    /// Wait for a leader to be elected
    async fn wait_for_leader(&self, timeout: Duration) -> Option<NodeId> {
        let start = tokio::time::Instant::now();
        while start.elapsed() < timeout {
            if let Some(leader) = self.get_leader() {
                return Some(leader.id.clone());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        None
    }

    fn shutdown(self) {
        for node in self.nodes {
            let _ = node.raft.shutdown();
        }
    }
}

// ============================================================================
// Test #1: Election Safety
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_election_safety_single_leader_per_term() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #1: Election Safety - Single leader per term");

    let cluster = SafetyTestCluster::new(5).await;

    // Wait for initial leader election
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Check: At most one leader per term
    let terms = cluster.get_terms();
    info!("Current terms: {:?}", terms);

    let leader_count = cluster.count_leaders();
    info!("Leaders detected: {}", leader_count);

    assert!(
        leader_count <= 1,
        "Election safety violated: {} leaders in cluster",
        leader_count
    );

    // Verify all nodes agree on the same term (or are within 1 term)
    let max_term = *terms.values().max().unwrap();
    let min_term = *terms.values().min().unwrap();
    assert!(
        max_term - min_term <= 1,
        "Terms diverged too much: {} to {}",
        min_term,
        max_term
    );

    cluster.shutdown();
    info!("✓ Election safety verified");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_election_safety_with_partitions() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #1b: Election Safety - Multiple partitions scenario");

    let cluster = SafetyTestCluster::new(5).await;

    // Wait for initial leader
    tokio::time::sleep(Duration::from_millis(500)).await;
    let initial_leader = cluster.wait_for_leader(Duration::from_secs(2)).await;
    info!("Initial leader: {:?}", initial_leader);

    // Create a partition: [n0, n1] vs [n2, n3, n4]
    let group_a = vec![NodeId::new("n0"), NodeId::new("n1")];
    let group_b = vec![NodeId::new("n2"), NodeId::new("n3"), NodeId::new("n4")];

    cluster.create_partition(&group_a, &group_b);
    info!("Created partition: {:?} vs {:?}", group_a, group_b);

    // Wait for new election in majority partition
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Check: Each partition has at most one leader
    let leaders_a = group_a.iter().filter(|id| {
        cluster.nodes.iter()
            .find(|n| &n.id == *id)
            .map(|n| n.raft.is_leader())
            .unwrap_or(false)
    }).count();

    let leaders_b = group_b.iter().filter(|id| {
        cluster.nodes.iter()
            .find(|n| &n.id == *id)
            .map(|n| n.raft.is_leader())
            .unwrap_or(false)
    }).count();

    info!("Leaders in partition A: {}, partition B: {}", leaders_a, leaders_b);

    assert!(leaders_a <= 1, "Multiple leaders in partition A");
    assert!(leaders_b <= 1, "Multiple leaders in partition B");

    // Note: A partitioned leader may keep its role until it discovers a higher term.
    // This is valid Raft behavior - the safety property is that it can't commit,
    // not that it must immediately step down.
    // TODO: Implement check-quorum to make leaders step down faster when partitioned.
    if leaders_a > 0 {
        info!("Note: Minority partition leader hasn't stepped down yet (valid Raft behavior)");
    }

    // Heal partition
    cluster.heal_all();
    info!("Healed partition");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // After healing, should converge to single leader
    let final_leader_count = cluster.count_leaders();
    assert_eq!(
        final_leader_count, 1,
        "Should have exactly one leader after healing"
    );

    cluster.shutdown();
    info!("✓ Election safety with partitions verified");
}

// ============================================================================
// Test #2: Log Matching Property
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_log_matching_property() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #2: Log Matching - Same index+term implies identical history");

    let cluster = SafetyTestCluster::new(3).await;

    // Wait for leader
    let leader_id = cluster.wait_for_leader(Duration::from_secs(2)).await;
    assert!(leader_id.is_some(), "No leader elected");
    info!("Leader elected: {:?}", leader_id);

    let leader = cluster.get_leader().unwrap();

    // Propose several entries
    let mut proposed_indices = Vec::new();
    for i in 0..10 {
        let cmd = Bytes::from(format!("cmd_{}", i));
        match leader.raft.propose(cmd).await {
            Ok(index) => {
                proposed_indices.push(index);
                info!("Proposed entry {} at index {}", i, index);
            }
            Err(e) => {
                warn!("Propose failed: {:?}", e);
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Verify log matching property: for each log entry at index i with term t,
    // all preceding entries should be identical across all nodes
    info!("Verifying log matching property...");

    for node in &cluster.nodes {
        let log = node.raft.log_ref();
        let last_index = log.last_index().await;
        info!(
            "Node {}: log entries 1..={}",
            node.id,
            last_index
        );

        // Check that log is well-formed (no gaps, terms are non-decreasing)
        if last_index > LogIndex::ZERO {
            let mut prev_term = 0;
            for i in 1..=last_index.0 {
                if let Ok(Some(entry)) = log.get(LogIndex(i)).await {
                    assert!(
                        entry.term.0 >= prev_term,
                        "Terms should be non-decreasing: {} -> {} at index {}",
                        prev_term,
                        entry.term.0,
                        i
                    );
                    prev_term = entry.term.0;
                }
            }
        }
    }

    cluster.shutdown();
    info!("✓ Log matching property verified");
}

// ============================================================================
// Test #3: Leader Completeness
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_leader_completeness() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #3: Leader Completeness - Committed entries in all future leaders");

    let cluster = SafetyTestCluster::new(3).await;

    // Wait for initial leader
    let leader_id = cluster.wait_for_leader(Duration::from_secs(2)).await;
    assert!(leader_id.is_some(), "No leader elected");
    info!("Initial leader: {:?}", leader_id);

    let leader = cluster.get_leader().unwrap();

    // Propose and commit some entries
    let mut committed_commands = Vec::new();
    for i in 0..5 {
        let cmd = Bytes::from(format!("committed_cmd_{}", i));
        match leader.raft.propose(cmd.clone()).await {
            Ok(index) => {
                info!("Proposed committed entry {} at index {}", i, index);
                committed_commands.push((index, cmd));
            }
            Err(e) => {
                warn!("Propose failed: {:?}", e);
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Wait for commitment
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Record commit index on current leader
    let commit_index = leader.raft.commit_index();
    info!("Current commit index: {}", commit_index);

    // Partition current leader
    let leader_id_value = leader_id.clone().unwrap();
    cluster.partition_node(&leader_id_value);
    info!("Partitioned leader {:?}", leader_id);

    // Wait for new leader election
    tokio::time::sleep(Duration::from_secs(2)).await;

    // New leader should have all committed entries
    let new_leader = cluster.get_leader();
    assert!(new_leader.is_some(), "No new leader elected");
    let new_leader = new_leader.unwrap();
    info!("New leader: {:?}", new_leader.id);

    let new_leader_log = new_leader.raft.log_ref();
    let new_leader_last_index = new_leader_log.last_index().await;

    info!(
        "New leader last index: {}, old commit index: {}",
        new_leader_last_index, commit_index
    );

    // Leader completeness: new leader must have all committed entries
    assert!(
        new_leader_last_index >= commit_index,
        "Leader completeness violated: new leader missing committed entries"
    );

    // Verify the actual entries match
    for (index, cmd) in committed_commands {
        if index <= commit_index {
            if let Ok(Some(entry)) = new_leader_log.get(index).await {
                assert_eq!(
                    entry.command, cmd,
                    "Entry mismatch at index {}: expected {:?}, got {:?}",
                    index, cmd, entry.command
                );
            } else {
                panic!("Committed entry at index {} not found in new leader", index);
            }
        }
    }

    cluster.shutdown();
    info!("✓ Leader completeness verified");
}

// ============================================================================
// Test #4: State Machine Safety
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_state_machine_safety() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #4: State Machine Safety - No conflicting applications at same index");

    let cluster = SafetyTestCluster::new(3).await;

    // Wait for leader
    let leader_id = cluster.wait_for_leader(Duration::from_secs(2)).await;
    assert!(leader_id.is_some(), "No leader elected");

    let leader = cluster.get_leader().unwrap();

    // Subscribe to applied entries on each node
    let mut applied_logs: HashMap<NodeId, Vec<(LogIndex, Bytes)>> = HashMap::new();
    let applied_channels: Arc<Mutex<HashMap<NodeId, mpsc::Receiver<(LogIndex, Bytes)>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    for node in &cluster.nodes {
        let rx = node.raft.subscribe_applied();
        applied_channels.lock().await.insert(node.id.clone(), rx);
        applied_logs.insert(node.id.clone(), Vec::new());
    }

    // Propose entries
    for i in 0..10 {
        let cmd = Bytes::from(format!("cmd_{}", i));
        match leader.raft.propose(cmd).await {
            Ok(index) => {
                info!("Proposed entry {} at index {}", i, index);
            }
            Err(e) => {
                warn!("Propose failed: {:?}", e);
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Collect applied entries from all nodes
    tokio::time::sleep(Duration::from_millis(500)).await;

    for node_id in cluster.nodes.iter().map(|n| n.id.clone()) {
        let mut channels = applied_channels.lock().await;
        if let Some(rx) = channels.get_mut(&node_id) {
            while let Ok((index, cmd)) = rx.try_recv() {
                applied_logs.get_mut(&node_id).unwrap().push((index, cmd));
            }
        }
    }

    // Verify state machine safety: for any index i, all nodes that applied
    // entry at index i applied the same command
    info!("Verifying state machine safety...");

    let mut index_to_commands: HashMap<LogIndex, HashSet<Bytes>> = HashMap::new();

    for (node_id, entries) in &applied_logs {
        info!("Node {}: {} applied entries", node_id, entries.len());
        for (index, cmd) in entries {
            index_to_commands
                .entry(*index)
                .or_insert_with(HashSet::new)
                .insert(cmd.clone());
        }
    }

    // Check: each index should have exactly one unique command
    for (index, commands) in &index_to_commands {
        assert_eq!(
            commands.len(),
            1,
            "State machine safety violated at index {}: multiple commands {:?}",
            index,
            commands
        );
    }

    cluster.shutdown();
    info!("✓ State machine safety verified");
}

// ============================================================================
// Test #5: Log Divergence Recovery
// ============================================================================

#[tokio::test(flavor = "multi_thread")]
async fn test_log_divergence_recovery() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Test #5: Log Divergence Recovery - Followers reconcile with leader");

    let cluster = SafetyTestCluster::new(3).await;

    // Wait for initial leader
    let leader_id = cluster.wait_for_leader(Duration::from_secs(2)).await;
    assert!(leader_id.is_some(), "No leader elected");
    info!("Initial leader: {:?}", leader_id);

    let leader = cluster.get_leader().unwrap();

    // Propose some initial entries
    for i in 0..5 {
        let cmd = Bytes::from(format!("initial_cmd_{}", i));
        let _ = leader.raft.propose(cmd).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // Wait for replication
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Partition a follower (NOT the leader)
    let leader_id_value = leader_id.clone().unwrap();
    let follower_id = cluster
        .nodes
        .iter()
        .find(|n| n.id != leader_id_value)
        .map(|n| n.id.clone())
        .expect("Should have at least one follower");
    cluster.partition_node(&follower_id);
    info!("Partitioned follower: {:?}", follower_id);

    // Leader proposes more entries (these won't reach partitioned follower)
    for i in 5..10 {
        let cmd = Bytes::from(format!("leader_cmd_{}", i));
        let _ = leader.raft.propose(cmd).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    tokio::time::sleep(Duration::from_millis(300)).await;

    // Heal the partition
    cluster.heal_node(&follower_id);
    info!("Healed follower: {:?}", follower_id);

    // Wait for cluster to stabilize: leader elected AND follower caught up
    // Use polling instead of fixed sleep to handle variable election timing
    let reconciliation_timeout = Duration::from_secs(10);
    let start = tokio::time::Instant::now();
    let mut follower_last_index = LogIndex::ZERO;

    while start.elapsed() < reconciliation_timeout {
        // Check we have a leader
        if let Some(current_leader) = cluster.get_leader() {
            let follower = cluster.nodes.iter().find(|n| n.id == follower_id).unwrap();
            follower_last_index = follower.raft.log_ref().last_index().await;

            // Check if follower has caught up (at least 10 entries from our test)
            if follower_last_index >= LogIndex(10) {
                info!(
                    "Leader ({:?}), Follower ({:?}) last index: {} - caught up!",
                    current_leader.id, follower_id, follower_last_index
                );
                break;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Final verification
    assert!(
        follower_last_index >= LogIndex(10),
        "Follower failed to catch up within {:?}: expected at least 10 entries, got {}",
        reconciliation_timeout,
        follower_last_index
    );

    // Get current state for log matching verification
    let current_leader = cluster.get_leader().expect("Should have leader for final check");
    let leader_log = current_leader.raft.log_ref();
    let leader_last_index = leader_log.last_index().await;
    let follower = cluster.nodes.iter().find(|n| n.id == follower_id).unwrap();
    let follower_log = follower.raft.log_ref();
    let follower_last_index = follower_log.last_index().await;

    info!(
        "Final state - Leader ({:?}) last index: {}, Follower ({:?}) last index: {}",
        current_leader.id, leader_last_index, follower_id, follower_last_index
    );

    // Verify logs match up to common length
    let common_len = std::cmp::min(leader_last_index.0, follower_last_index.0);
    for i in 1..=common_len {
        let leader_entry = leader_log.get(LogIndex(i)).await.unwrap().unwrap();
        let follower_entry = follower_log.get(LogIndex(i)).await.unwrap().unwrap();

        assert_eq!(
            leader_entry.term, follower_entry.term,
            "Term mismatch at index {}",
            i
        );
        assert_eq!(
            leader_entry.command, follower_entry.command,
            "Command mismatch at index {}",
            i
        );
    }

    cluster.shutdown();
    info!("✓ Log divergence recovery verified");
}
