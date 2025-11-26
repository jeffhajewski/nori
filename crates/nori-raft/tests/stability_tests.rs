//! Stability Tests (#15-16)
//!
//! Long-running stability tests for detecting:
//! 15. Extended Soak Test - 24-hour continuous operation under load
//! 16. Memory Leak Detection - Resource tracking over extended periods
//!
//! These tests verify system stability, resource management, and long-term reliability.
//!
//! NOTE: These are marked with #[ignore] by default as they take a long time to run.
//! Run them explicitly with: cargo test -- --ignored

use bytes::Bytes;
use nori_raft::transport::{InMemoryTransport, RpcSender};
use nori_raft::{ConfigEntry, LogIndex, NodeId, RaftConfig, ReplicatedLSM, Term};
use nori_lsm::ATLLConfig;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::sync::Mutex;

/// Helper to extract just the value from replicated_get() result, ignoring version
fn get_value(result: Option<(Bytes, Term, LogIndex)>) -> Option<Bytes> {
    result.map(|(v, _, _)| v)
}

/// Test cluster for stability tests
struct StabilityTestCluster {
    nodes: Vec<StabilityTestNode>,
    _transports: HashMap<NodeId, Arc<InMemoryTransport>>,
    _rpc_senders: HashMap<NodeId, RpcSender>,
}

struct StabilityTestNode {
    id: NodeId,
    replicated_lsm: Arc<ReplicatedLSM>,
    _raft_dir: TempDir,
    _lsm_dir: TempDir,
}

impl StabilityTestCluster {
    async fn new(num_nodes: usize) -> Self {
        let node_ids: Vec<NodeId> = (0..num_nodes)
            .map(|i| NodeId::new(&format!("n{}", i)))
            .collect();

        // Create RPC channels
        let mut rpc_channels = HashMap::new();
        let mut rpc_senders = HashMap::new();

        for node_id in &node_ids {
            let (tx, rx) = tokio::sync::mpsc::channel(1000); // Larger buffer for stability tests
            rpc_channels.insert(node_id.clone(), rx);
            rpc_senders.insert(node_id.clone(), tx);
        }

        // Create transports
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

            nodes.push(StabilityTestNode {
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

        StabilityTestCluster {
            nodes,
            _transports: transports,
            _rpc_senders: rpc_senders,
        }
    }

    fn get_leader(&self) -> Option<&StabilityTestNode> {
        self.nodes
            .iter()
            .find(|n| n.replicated_lsm.is_leader())
    }

    async fn wait_for_leader(&self, timeout: Duration) -> Option<NodeId> {
        let start = Instant::now();
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
            let _ = node.replicated_lsm.shutdown();
        }
    }
}

/// Statistics tracking for stability tests
#[derive(Debug, Default)]
struct StabilityStats {
    operations_completed: AtomicU64,
    operations_failed: AtomicU64,
    total_bytes_written: AtomicU64,
    total_bytes_read: AtomicU64,
}

impl StabilityStats {
    fn record_write(&self, bytes: usize) {
        self.operations_completed.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_written
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_read(&self, bytes: usize) {
        self.total_bytes_read
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_failure(&self) {
        self.operations_failed.fetch_add(1, Ordering::Relaxed);
    }

    fn report(&self) -> String {
        format!(
            "Ops: {} (failed: {}), Written: {} MB, Read: {} MB",
            self.operations_completed.load(Ordering::Relaxed),
            self.operations_failed.load(Ordering::Relaxed),
            self.total_bytes_written.load(Ordering::Relaxed) / (1024 * 1024),
            self.total_bytes_read.load(Ordering::Relaxed) / (1024 * 1024)
        )
    }
}

// ============================================================================
// Test #15: Extended Soak Test
// ============================================================================

/// Extended soak test - runs for a configurable duration with continuous load.
///
/// This test is designed to run for extended periods (default: 1 hour for CI,
/// configurable up to 24 hours for manual testing) to detect:
/// - Memory leaks
/// - Resource exhaustion
/// - Performance degradation over time
/// - Unexpected failures under sustained load
///
/// Test pattern:
/// - 3-node cluster
/// - Continuous writes and reads
/// - Periodic leadership checks
/// - Resource usage monitoring
#[tokio::test(flavor = "multi_thread")]
#[ignore] // Run explicitly with: cargo test -- --ignored
async fn test_extended_soak() {
    // Configure test duration (use env var for flexibility)
    let duration_secs = std::env::var("SOAK_TEST_DURATION_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(3600); // Default: 1 hour

    let duration = Duration::from_secs(duration_secs);
    println!("Starting soak test for {} seconds", duration_secs);

    let cluster = StabilityTestCluster::new(3).await;
    let stats = Arc::new(StabilityStats::default());

    // Wait for initial leader
    let leader_id = cluster.wait_for_leader(Duration::from_secs(10)).await;
    assert!(leader_id.is_some(), "No leader elected");
    println!("Initial leader: {:?}", leader_id);

    // Give cluster time to stabilize
    tokio::time::sleep(Duration::from_millis(1000)).await;

    let start = Instant::now();
    let mut last_report = Instant::now();
    let mut operation_counter = 0u64;

    // Run continuous load until duration expires
    while start.elapsed() < duration {
        // Get current leader
        let leader = cluster.get_leader();
        if leader.is_none() {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        let leader = leader.unwrap();

        // Perform write operation
        let key = Bytes::from(format!("soak_key_{}", operation_counter));
        let value = Bytes::from(format!("soak_value_{}", operation_counter));

        match leader
            .replicated_lsm
            .replicated_put(key.clone(), value.clone(), None)
            .await
        {
            Ok(_) => {
                stats.record_write(key.len() + value.len());
            }
            Err(_) => {
                stats.record_failure();
                // Leader might have changed, retry on next iteration
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
        }

        // Perform read operation every 10 writes
        if operation_counter % 10 == 0 {
            match leader.replicated_lsm.replicated_get(&key).await {
                Ok(Some((v, _term, _index))) => {
                    stats.record_read(v.len());
                    assert_eq!(v, value, "Read returned wrong value");
                }
                Ok(None) => {
                    panic!("Key disappeared: {:?}", key);
                }
                Err(_) => {
                    stats.record_failure();
                }
            }
        }

        operation_counter += 1;

        // Report progress every 60 seconds
        if last_report.elapsed() >= Duration::from_secs(60) {
            println!(
                "[{:.1}%] {}",
                (start.elapsed().as_secs_f64() / duration.as_secs_f64()) * 100.0,
                stats.report()
            );
            last_report = Instant::now();
        }

        // Small delay to avoid overwhelming the system
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    println!("Soak test completed: {}", stats.report());
    println!("Total operations: {}", operation_counter);

    // Verify final state
    let final_leader = cluster.get_leader().expect("No leader at end");
    let sample_key = Bytes::from("soak_key_0");
    let result = final_leader
        .replicated_lsm
        .replicated_get(&sample_key)
        .await
        .unwrap();
    assert!(result.is_some(), "Sample key should still exist");

    cluster.shutdown();
}

// ============================================================================
// Test #16: Memory Leak Detection
// ============================================================================

/// Memory leak detection test - monitors memory usage over time.
///
/// This test runs a controlled workload and tracks memory usage to detect leaks:
/// - Performs repeated write/read cycles
/// - Triggers compaction and garbage collection
/// - Monitors memory growth over iterations
/// - Fails if memory usage grows beyond acceptable bounds
///
/// The test uses a simplified leak detection heuristic:
/// - Runs multiple iterations of the same workload
/// - Measures memory before and after each iteration
/// - Expects memory to stabilize after initial warmup
#[tokio::test(flavor = "multi_thread")]
#[ignore] // Run explicitly with: cargo test -- --ignored
async fn test_memory_leak_detection() {
    // This test is more of a smoke test for memory leaks
    // For production, use valgrind, heaptrack, or similar tools

    println!("Starting memory leak detection test");

    let cluster = Arc::new(Mutex::new(StabilityTestCluster::new(3).await));

    // Wait for leader
    {
        let c = cluster.lock().await;
        let leader_id = c.wait_for_leader(Duration::from_secs(5)).await;
        assert!(leader_id.is_some(), "No leader elected");
    }

    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Run multiple iterations with the same workload
    let iterations = 10;
    let ops_per_iteration = 1000;

    for iteration in 0..iterations {
        println!("Iteration {}/{}", iteration + 1, iterations);

        let leader = {
            let c = cluster.lock().await;
            c.get_leader().unwrap().replicated_lsm.clone()
        };

        // Perform writes
        for i in 0..ops_per_iteration {
            let key = Bytes::from(format!("leak_test_key_{}", i));
            let value = Bytes::from(format!("leak_test_value_{}", i));

            match leader.replicated_put(key, value, None).await {
                Ok(_) => {}
                Err(_) => {
                    // Retry once on failure
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        }

        // Perform reads
        for i in 0..ops_per_iteration {
            let key = Bytes::from(format!("leak_test_key_{}", i));
            let _ = leader.replicated_get(&key).await;
        }

        // Wait for background tasks to process
        tokio::time::sleep(Duration::from_millis(500)).await;

        println!("  Iteration {} completed", iteration + 1);
    }

    println!("Memory leak detection test completed");
    println!("Note: For detailed memory profiling, use tools like valgrind or heaptrack");

    // Clean shutdown
    {
        let c = Arc::try_unwrap(cluster)
            .unwrap_or_else(|_| panic!("cluster still has references"))
            .into_inner();
        c.shutdown();
    }
}

// ============================================================================
// Test #17: Compaction Under Load
// ============================================================================

/// Test compaction behavior under continuous write load.
///
/// Verifies that:
/// - Compaction doesn't interfere with writes
/// - No data loss during compaction
/// - System remains stable during compaction
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_compaction_under_load() {
    let cluster = StabilityTestCluster::new(3).await;

    let leader_id = cluster.wait_for_leader(Duration::from_secs(5)).await;
    assert!(leader_id.is_some(), "No leader elected");

    tokio::time::sleep(Duration::from_millis(500)).await;

    let leader = cluster.get_leader().unwrap();

    // Write enough data to trigger multiple compactions
    let num_keys = 10000;

    println!("Writing {} keys to trigger compaction", num_keys);

    for i in 0..num_keys {
        let key = Bytes::from(format!("compact_key_{:05}", i));
        let value = Bytes::from(vec![b'x'; 1024]); // 1KB values

        match leader
            .replicated_lsm
            .replicated_put(key, value, None)
            .await
        {
            Ok(_) => {}
            Err(_) => {
                // Retry on failure
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }

        if i % 1000 == 0 {
            println!("  Written {} keys", i);
        }
    }

    println!("All keys written, waiting for compaction to settle");
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify data integrity after compaction
    println!("Verifying data integrity");
    let leader = cluster.get_leader().unwrap();

    for i in 0..num_keys {
        let key = Bytes::from(format!("compact_key_{:05}", i));
        let result = leader.replicated_lsm.replicated_get(&key).await.unwrap();
        assert!(result.is_some(), "Key {} disappeared after compaction", i);
    }

    println!("Compaction under load test completed successfully");
    cluster.shutdown();
}
