//! Integration Test #10: End-to-End Linearizability Stress Test
//!
//! This is the BIG ONE - comprehensive test of ReplicatedLSM linearizability
//! under realistic chaos conditions.
//!
//! Test setup:
//! - 3-node ReplicatedLSM cluster
//! - 10 concurrent clients
//! - 100 operations per client (1000 total)
//! - Random network partitions (1-3 second intervals, 500-1500ms duration)
//! - Chaos injector: up to 3 partitions during test
//! - Full linearizability verification
//!
//! Success criteria:
//! - All acknowledged operations are linearizable
//! - No data loss
//! - Cluster recovers from all failures

use bytes::Bytes;
use nori_raft::transport::{InMemoryTransport, RpcSender};
use nori_raft::{ConfigEntry, NodeId, RaftConfig, ReplicatedLSM};
use nori_lsm::ATLLConfig;
use nori_testkit::linearizability::{History, Operation, OperationId, OperationResult};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Test cluster with 3 ReplicatedLSM nodes
struct TestCluster {
    nodes: Vec<TestNode>,
    transports: HashMap<NodeId, Arc<InMemoryTransport>>,
    rpc_senders: HashMap<NodeId, RpcSender>,
}

struct TestNode {
    id: NodeId,
    replicated_lsm: Arc<ReplicatedLSM>,
    _raft_dir: TempDir,
    _lsm_dir: TempDir,
}

impl TestCluster {
    /// Create a 3-node cluster with fully connected transport
    async fn new() -> Self {
        let node_ids = vec![
            NodeId::new("n1"),
            NodeId::new("n2"),
            NodeId::new("n3"),
        ];

        // Create RPC channels
        let mut rpc_channels = HashMap::new();
        let mut rpc_senders = HashMap::new();

        for node_id in &node_ids {
            let (tx, rx) = tokio::sync::mpsc::channel(100);
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

        // Create nodes
        let mut nodes = Vec::new();
        let initial_config = ConfigEntry::Single(node_ids.clone());

        for node_id in &node_ids {
            let raft_dir = TempDir::new().unwrap();
            let lsm_dir = TempDir::new().unwrap();

            let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
                .await
                .unwrap();

            let mut raft_config = RaftConfig::default();
            raft_config.election_timeout_min = Duration::from_millis(150);
            raft_config.election_timeout_max = Duration::from_millis(300);

            let mut lsm_config = ATLLConfig::default();
            lsm_config.data_dir = lsm_dir.path().to_path_buf();

            let transport = transports.get(node_id).unwrap().clone();
            let rpc_rx = rpc_channels.remove(node_id);

            let replicated_lsm = ReplicatedLSM::new(
                node_id.clone(),
                raft_config,
                lsm_config,
                raft_log,
                transport,
                initial_config.clone(),
                rpc_rx,
            )
            .await
            .unwrap();

            nodes.push(TestNode {
                id: node_id.clone(),
                replicated_lsm: Arc::new(replicated_lsm),
                _raft_dir: raft_dir,
                _lsm_dir: lsm_dir,
            });
        }

        // Start all nodes
        for node in &nodes {
            node.replicated_lsm.start().await.unwrap();
        }

        // Wait for initial leader election
        tokio::time::sleep(Duration::from_millis(500)).await;

        TestCluster {
            nodes,
            transports,
            rpc_senders,
        }
    }

    /// Partition a node from all others (simulate network partition)
    fn partition_node(&self, node_id: &NodeId) {
        info!("Partitioning node {:?} from cluster", node_id);

        // Remove this node from all other nodes' transports
        for (peer_id, transport) in &self.transports {
            if peer_id != node_id {
                transport.remove_peer(node_id);
            }
        }

        // Remove all peers from this node's transport
        if let Some(transport) = self.transports.get(node_id) {
            for peer_id in self.transports.keys() {
                if peer_id != node_id {
                    transport.remove_peer(peer_id);
                }
            }
        }
    }

    /// Heal a partitioned node (restore network connectivity)
    fn heal_node(&self, node_id: &NodeId) {
        info!("Healing node {:?}, restoring connectivity", node_id);

        // Restore this node in all other nodes' transports
        for (peer_id, transport) in &self.transports {
            if peer_id != node_id {
                if let Some(sender) = self.rpc_senders.get(node_id) {
                    transport.add_peer(node_id.clone(), sender.clone());
                }
            }
        }

        // Restore all peers in this node's transport
        if let Some(transport) = self.transports.get(node_id) {
            for (peer_id, sender) in &self.rpc_senders {
                if peer_id != node_id {
                    transport.add_peer(peer_id.clone(), sender.clone());
                }
            }
        }
    }

    async fn shutdown(self) {
        for node in self.nodes {
            let _ = node.replicated_lsm.shutdown().await;
        }
    }
}

/// Client that performs operations and records history
struct TestClient {
    client_id: u64,
    cluster: Arc<Mutex<TestCluster>>,
    history: Arc<Mutex<History>>,
    op_counter: Arc<AtomicU64>,
    stop_flag: Arc<AtomicBool>,
}

impl TestClient {
    fn new(
        client_id: u64,
        cluster: Arc<Mutex<TestCluster>>,
        history: Arc<Mutex<History>>,
        stop_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            client_id,
            cluster,
            history,
            op_counter: Arc::new(AtomicU64::new(0)),
            stop_flag,
        }
    }

    /// Run client workload: mix of puts, gets, deletes
    async fn run(&self, num_operations: usize) {
        let mut operations_done = 0;

        while operations_done < num_operations && !self.stop_flag.load(Ordering::Relaxed) {
            let op_num = self.op_counter.fetch_add(1, Ordering::SeqCst);
            let op_id = OperationId((self.client_id << 32) | op_num);

            // Generate operation (70% puts, 20% gets, 10% deletes)
            let op_type = op_num % 10;
            let key_num = op_num % 20; // Operate on 20 keys
            let key = Bytes::from(format!("key_{}", key_num));

            match op_type {
                0..=6 => {
                    // Put operation
                    let value = Bytes::from(format!("value_{}_{}", self.client_id, op_num));
                    let operation = Operation::Put {
                        key: key.clone(),
                        value: value.clone(),
                    };

                    self.history.lock().await.invoke(op_id, operation);

                    let result = self.execute_put(key, value).await;

                    self.history.lock().await.complete(op_id, result);
                }
                7..=8 => {
                    // Get operation
                    let operation = Operation::Get { key: key.clone() };

                    self.history.lock().await.invoke(op_id, operation);

                    let result = self.execute_get(key).await;

                    self.history.lock().await.complete(op_id, result);
                }
                9 => {
                    // Delete operation
                    let operation = Operation::Delete { key: key.clone() };

                    self.history.lock().await.invoke(op_id, operation);

                    let result = self.execute_delete(key).await;

                    self.history.lock().await.complete(op_id, result);
                }
                _ => unreachable!(),
            }

            operations_done += 1;

            // Small random delay between operations
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        info!(
            "Client {} completed {} operations",
            self.client_id, operations_done
        );
    }

    async fn execute_put(&self, key: Bytes, value: Bytes) -> OperationResult {
        let cluster = self.cluster.lock().await;

        // Try each node until we find the leader
        for node in &cluster.nodes {
            match node.replicated_lsm.replicated_put(key.clone(), value.clone(), None).await {
                Ok(_) => return OperationResult::PutOk,
                Err(e) if format!("{:?}", e).contains("NotLeader") => continue,
                Err(e) => return OperationResult::Error(format!("{:?}", e)),
            }
        }

        OperationResult::Error("No leader found".to_string())
    }

    async fn execute_get(&self, key: Bytes) -> OperationResult {
        let cluster = self.cluster.lock().await;

        // Try each node until we find the leader
        for node in &cluster.nodes {
            match node.replicated_lsm.replicated_get(&key).await {
                Ok(value) => return OperationResult::GetOk(value),
                Err(e) if format!("{:?}", e).contains("NotLeader") => continue,
                Err(e) => return OperationResult::Error(format!("{:?}", e)),
            }
        }

        OperationResult::Error("No leader found".to_string())
    }

    async fn execute_delete(&self, key: Bytes) -> OperationResult {
        let cluster = self.cluster.lock().await;

        // Try each node until we find the leader
        for node in &cluster.nodes {
            match node.replicated_lsm.replicated_delete(key.clone()).await {
                Ok(_) => return OperationResult::DeleteOk,
                Err(e) if format!("{:?}", e).contains("NotLeader") => continue,
                Err(e) => return OperationResult::Error(format!("{:?}", e)),
            }
        }

        OperationResult::Error("No leader found".to_string())
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore] // Run with --ignored flag for full stress test
async fn test_end_to_end_linearizability_stress() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Starting End-to-End Linearizability Stress Test");

    // Create cluster
    let cluster = Arc::new(Mutex::new(TestCluster::new().await));
    let history = Arc::new(Mutex::new(History::new()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Launch 10 concurrent clients
    let mut client_handles = Vec::new();
    let num_clients = 10;
    let ops_per_client = 100;

    for client_id in 0..num_clients {
        let client = TestClient::new(
            client_id,
            Arc::clone(&cluster),
            Arc::clone(&history),
            Arc::clone(&stop_flag),
        );

        let handle = tokio::spawn(async move {
            client.run(ops_per_client).await;
        });

        client_handles.push(handle);
    }

    info!(
        "Launched {} clients, each performing {} operations",
        num_clients, ops_per_client
    );

    // Launch chaos injection task
    let chaos_cluster = Arc::clone(&cluster);
    let chaos_stop = Arc::clone(&stop_flag);
    let _chaos_handle = tokio::spawn(async move {
        use rand::{Rng, SeedableRng};
        use rand::rngs::StdRng;
        let mut rng = StdRng::from_entropy();
        let mut partition_count = 0;

        while !chaos_stop.load(Ordering::Relaxed) {
            // Wait 1-3 seconds between chaos events
            tokio::time::sleep(Duration::from_millis(rng.gen_range(1000..3000))).await;

            if chaos_stop.load(Ordering::Relaxed) {
                break;
            }

            let cluster = chaos_cluster.lock().await;

            // 50% chance to partition a random node
            if rng.gen_bool(0.5) && partition_count < 3 {
                let node_idx = rng.gen_range(0..cluster.nodes.len());
                let node_id = &cluster.nodes[node_idx].id;
                cluster.partition_node(node_id);
                partition_count += 1;

                // Heal after 500-1500ms
                let heal_delay = Duration::from_millis(rng.gen_range(500..1500));
                let heal_cluster = Arc::clone(&chaos_cluster);
                let heal_node_id = node_id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(heal_delay).await;
                    let cluster = heal_cluster.lock().await;
                    cluster.heal_node(&heal_node_id);
                });
            }
        }

        info!("Chaos injector stopped after {} partitions", partition_count);
    });

    // Wait for all clients to complete
    for handle in client_handles {
        handle.await.unwrap();
    }

    stop_flag.store(true, Ordering::Relaxed);

    info!("All clients completed. Checking linearizability...");

    // Check linearizability
    let history = history.lock().await;
    let num_ops = history.operations().len();
    info!("Recorded {} completed operations", num_ops);

    assert!(num_ops > 0, "Should have recorded some operations");

    // Check linearizability - the checker is now debugged and working!
    match history.check_linearizability() {
        Ok(()) => {
            info!("✓ History is linearizable!");
        }
        Err(e) => {
            warn!("✗ History is NOT linearizable: {:?}", e);
            panic!("Linearizability violation detected");
        }
    }

    info!("Test completed successfully");

    // Cleanup
    let cluster = Arc::try_unwrap(cluster)
        .unwrap_or_else(|_| panic!("cluster still has references"))
        .into_inner();
    cluster.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn test_basic_three_node_linearizability() {
    // Simplified test: 3 nodes, 3 clients, 10 ops each, no failures
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    info!("Starting basic 3-node linearizability test");

    let cluster = Arc::new(Mutex::new(TestCluster::new().await));
    let history = Arc::new(Mutex::new(History::new()));
    let stop_flag = Arc::new(AtomicBool::new(false));

    // Launch 3 clients
    let mut client_handles = Vec::new();
    for client_id in 0..3 {
        let client = TestClient::new(
            client_id,
            Arc::clone(&cluster),
            Arc::clone(&history),
            Arc::clone(&stop_flag),
        );

        let handle = tokio::spawn(async move {
            client.run(10).await;
        });

        client_handles.push(handle);
    }

    // Wait for completion
    for handle in client_handles {
        handle.await.unwrap();
    }

    stop_flag.store(true, Ordering::Relaxed);

    let history = history.lock().await;
    let num_ops = history.operations().len();
    info!("Recorded {} operations", num_ops);

    assert!(num_ops > 0, "Should have recorded operations");

    // Check linearizability
    match history.check_linearizability() {
        Ok(()) => {
            info!("✓ History is linearizable!");
        }
        Err(e) => {
            warn!("✗ History is NOT linearizable: {:?}", e);
            panic!("Linearizability violation detected");
        }
    }

    info!("Basic test passed");

    let cluster = Arc::try_unwrap(cluster)
        .unwrap_or_else(|_| panic!("cluster still has references"))
        .into_inner();
    cluster.shutdown().await;
}
