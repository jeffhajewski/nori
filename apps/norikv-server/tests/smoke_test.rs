//! Smoke tests for norikv-server.
//!
//! Verifies that the server can start up, become a leader, and shut down cleanly.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_single_node_lifecycle() {
    // Initialize tracing for test visibility
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting single-node lifecycle test");

    // Create temp directory for test
    let temp_dir = TempDir::new().unwrap();

    // Create configuration
    let config = ServerConfig {
        node_id: "test_node_1".to_string(),
        rpc_addr: "127.0.0.1:0".to_string(), // Random port
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig {
            seed_nodes: vec![],
            total_shards: 1024,
            replication_factor: 3,
        },
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    // Create node
    let mut node = Node::new(config).await.expect("Failed to create node");

    // Start node
    node.start().await.expect("Failed to start node");

    // Give it time to elect itself as leader
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify it became leader
    assert!(
        node.replicated_lsm().is_leader(),
        "Single node should elect itself as leader"
    );

    tracing::info!("Node successfully became leader");

    // Perform a basic operation
    let key = bytes::Bytes::from("smoke_test_key");
    let value = bytes::Bytes::from("smoke_test_value");

    let _index = node
        .replicated_lsm()
        .replicated_put(key.clone(), value.clone(), None)
        .await
        .expect("Failed to put key");

    tracing::info!("Successfully wrote key");

    // Wait for apply
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Read it back
    let read_value = node
        .replicated_lsm()
        .replicated_get(b"smoke_test_key")
        .await
        .expect("Failed to get key");

    assert!(read_value.is_some(), "Key should exist");
    assert_eq!(read_value.unwrap(), value, "Value should match");

    tracing::info!("Successfully read key back");

    // Shutdown node
    node.shutdown().await.expect("Failed to shutdown node");

    tracing::info!("Node shutdown successfully");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_config_validation() {
    // Test invalid node_id
    let temp_dir = TempDir::new().unwrap();

    let config = ServerConfig {
        node_id: "".to_string(), // Empty node_id should fail
        rpc_addr: "127.0.0.1:0".to_string(),
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig::default(),
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    assert!(
        config.validate().is_err(),
        "Empty node_id should fail validation"
    );

    // Test invalid rpc_addr
    let config = ServerConfig {
        node_id: "test_node".to_string(),
        rpc_addr: "not_a_valid_address".to_string(),
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig::default(),
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    assert!(
        config.validate().is_err(),
        "Invalid rpc_addr should fail validation"
    );
}
