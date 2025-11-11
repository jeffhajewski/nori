//! Admin service integration tests for norikv-server.
//!
//! Verifies administrative operations like SnapshotShard work correctly.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use tempfile::TempDir;
use tokio::time::Duration;

// Re-export the proto types and client stubs
use norikv_transport_grpc::proto::{
    admin_client::AdminClient, kv_client::KvClient, PutRequest, ShardInfo,
};

/// Helper to create a test node with a specific port
async fn create_test_node(port: u16) -> (Node, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    let config = ServerConfig {
        node_id: format!("admin_test_node_{}", port),
        rpc_addr: format!("127.0.0.1:{}", port),
        http_addr: format!("127.0.0.1:{}", port + 1000), // HTTP on different port
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig {
            seed_nodes: vec![],
            total_shards: 8, // Small number for faster testing
            replication_factor: 3,
        },
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    let node = Node::new(config).await.expect("Failed to create node");
    (node, temp_dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_admin_snapshot_shard() {
    // Initialize tracing for test visibility
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting Admin SnapshotShard integration test");

    // Create and start node
    let (mut node, _temp_dir) = create_test_node(50061).await;
    node.start().await.expect("Failed to start node");

    // Give it time to elect itself as leader for at least one shard
    tokio::time::sleep(Duration::from_millis(1500)).await;

    tracing::info!("Node started");

    // Connect to the gRPC server
    let base_addr = "http://127.0.0.1:50061";

    // Wait for gRPC server to be fully ready
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create KV client to write some data first
    let mut kv_client = KvClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect KV client");

    tracing::info!("KV client connected");

    // Write some test data to trigger shard creation
    // Use keys that will definitely go to shard 0 (which is already leader)
    // to avoid waiting for dynamic shard leader election
    for i in 0..5 {
        let put_req = PutRequest {
            key: format!("shard0_test_key_{}", i).into_bytes(),
            value: format!("test_value_{}", i).into_bytes(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: None,
        };

        // Retry on NOT_LEADER errors (shard might be electing)
        let mut retries = 0;
        loop {
            match kv_client.put(put_req.clone()).await {
                Ok(_) => break,
                Err(e) if e.message().contains("NOT_LEADER") && retries < 5 => {
                    retries += 1;
                    tracing::info!("Waiting for shard to elect leader, retry {}", retries);
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
                Err(e) => panic!("PUT request failed after {} retries: {}", retries, e),
            }
        }
    }

    tracing::info!("Test data written");

    // Wait for writes to apply
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Create Admin client
    let mut admin_client = AdminClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect Admin client");

    tracing::info!("Admin client connected");

    // Find a shard that exists and is leader
    let active_shards = node.shard_manager().active_shards().await;
    assert!(
        !active_shards.is_empty(),
        "Should have at least one active shard"
    );

    let mut found_leader_shard = None;
    for shard_id in active_shards {
        if let Ok(info) = node.shard_manager().shard_info(shard_id).await {
            if info.is_leader {
                found_leader_shard = Some(shard_id);
                break;
            }
        }
    }

    let leader_shard_id = found_leader_shard.expect("Should have at least one leader shard");
    tracing::info!("Testing with leader shard ID: {}", leader_shard_id);

    // Test 1: Trigger snapshot on leader shard (should succeed)
    let snapshot_req = ShardInfo {
        id: leader_shard_id,
        replicas: vec![],
    };

    let snapshot_resp = admin_client
        .snapshot_shard(snapshot_req)
        .await
        .expect("SnapshotShard request failed")
        .into_inner();

    assert_eq!(
        snapshot_resp.id, leader_shard_id,
        "Response should return same shard ID"
    );
    assert!(
        !snapshot_resp.replicas.is_empty(),
        "Response should include replica info"
    );
    assert!(
        snapshot_resp.replicas[0].leader,
        "Replica should be marked as leader"
    );

    tracing::info!(
        "SnapshotShard succeeded for shard {}: {:?}",
        leader_shard_id,
        snapshot_resp
    );

    // Test 2: Try to snapshot a shard that doesn't exist (should fail)
    let nonexistent_shard_req = ShardInfo {
        id: 999, // Assuming this shard doesn't exist
        replicas: vec![],
    };

    let result = admin_client.snapshot_shard(nonexistent_shard_req).await;
    assert!(
        result.is_err(),
        "SnapshotShard on non-existent shard should fail"
    );

    let err = result.unwrap_err();
    // Check that it's a NOT_FOUND error (tonic Status code)
    assert!(
        err.message().contains("not found") || err.message().contains("Not found"),
        "Error should indicate shard not found: {}",
        err.message()
    );

    tracing::info!("SnapshotShard correctly rejected non-existent shard");

    // Shutdown
    node.shutdown().await.expect("Failed to shutdown node");
    tracing::info!("Admin SnapshotShard test completed successfully");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_admin_transfer_shard_unimplemented() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting Admin TransferShard (unimplemented) test");

    // Create and start node
    let (mut node, _temp_dir) = create_test_node(50062).await;
    node.start().await.expect("Failed to start node");

    tokio::time::sleep(Duration::from_millis(1000)).await;

    let base_addr = "http://127.0.0.1:50062";
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create Admin client
    let mut admin_client = AdminClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect Admin client");

    // Test: TransferShard should return UNIMPLEMENTED
    let transfer_req = ShardInfo {
        id: 0,
        replicas: vec![],
    };

    let result = admin_client.transfer_shard(transfer_req).await;
    assert!(
        result.is_err(),
        "TransferShard should return unimplemented error"
    );

    let err = result.unwrap_err();
    // Check that it's an UNIMPLEMENTED error
    assert!(
        err.message().contains("not yet implemented") || err.message().contains("Unimplemented"),
        "Error should indicate unimplemented: {}",
        err.message()
    );

    tracing::info!(
        "TransferShard correctly returned UNIMPLEMENTED: {}",
        err.message()
    );

    // Shutdown
    node.shutdown().await.expect("Failed to shutdown node");
    tracing::info!("Admin TransferShard test completed successfully");
}
