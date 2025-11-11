//! Smoke tests for norikv-server.
//!
//! Verifies that the server can start up, become a leader, and shut down cleanly.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use tempfile::TempDir;
use tokio::time::Duration;

// Import gRPC client for KV operations
use norikv_transport_grpc::proto::{
    kv_client::KvClient, GetRequest, PutRequest,
};

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
        rpc_addr: "127.0.0.1:50100".to_string(),
        http_addr: "127.0.0.1:58100".to_string(), // HTTP on different port
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

    // Give it time to elect shard 0 as leader
    tokio::time::sleep(Duration::from_millis(1500)).await;

    tracing::info!("Node started, shard 0 should be leader");

    // Connect gRPC client
    let base_addr = "http://127.0.0.1:50100";
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut client = KvClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect to gRPC server");

    tracing::info!("gRPC client connected");

    // Perform a basic PUT operation
    let put_req = PutRequest {
        key: b"smoke_test_key".to_vec(),
        value: b"smoke_test_value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let put_resp = client
        .put(put_req)
        .await
        .expect("Failed to put key")
        .into_inner();

    assert!(put_resp.version.is_some(), "PUT should return version");

    tracing::info!("Successfully wrote key");

    // Wait for apply
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Read it back via GET
    let get_req = GetRequest {
        key: b"smoke_test_key".to_vec(),
        consistency: String::new(),
    };

    let get_resp = client
        .get(get_req)
        .await
        .expect("Failed to get key")
        .into_inner();

    assert!(!get_resp.value.is_empty(), "Key should exist");
    assert_eq!(
        get_resp.value,
        b"smoke_test_value".to_vec(),
        "Value should match"
    );

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
        http_addr: "127.0.0.1:0".to_string(),
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
        http_addr: "127.0.0.1:0".to_string(),
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig::default(),
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    assert!(
        config.validate().is_err(),
        "Invalid rpc_addr should fail validation"
    );
}
