//! gRPC integration tests for norikv-server.
//!
//! Verifies that gRPC clients can connect and perform KV operations.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use tempfile::TempDir;
use tokio::time::Duration;

// Re-export the proto types and client stubs
use norikv_transport_grpc::proto::{
    kv_client::KvClient, DeleteRequest, GetRequest, PutRequest,
};

#[tokio::test(flavor = "multi_thread")]
async fn test_grpc_put_get_delete() {
    // Initialize tracing for test visibility
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting gRPC integration test");

    // Create temp directory for test
    let temp_dir = TempDir::new().unwrap();

    // Create configuration with fixed port for testing
    let config = ServerConfig {
        node_id: "grpc_test_node".to_string(),
        rpc_addr: "127.0.0.1:50051".to_string(),
        http_addr: "127.0.0.1:58051".to_string(), // HTTP on different port
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig {
            seed_nodes: vec![],
            total_shards: 1024,
            replication_factor: 3,
        },
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    // Create and start node
    let mut node = Node::new(config).await.expect("Failed to create node");
    node.start().await.expect("Failed to start node");

    // Give it time to elect itself as leader for shard 0
    // Note: Need at least 1.5-2 seconds for reliable leader election in tests
    tokio::time::sleep(Duration::from_millis(2000)).await;

    tracing::info!("Node started and became leader");

    // Connect to the gRPC server
    let base_addr = "http://127.0.0.1:50051";

    // Wait a bit for gRPC server to be fully ready
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Create gRPC client
    let mut client = KvClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect to gRPC server");

    tracing::info!("gRPC client connected");

    // Test 1: PUT a key (with retry for lazy shard creation)
    let put_req = PutRequest {
        key: b"test_key".to_vec(),
        value: b"test_value".to_vec(),
        ttl_ms: 0, // No TTL
        idempotency_key: String::new(),
        if_match: None,
    };

    let mut retries = 0;
    let put_resp = loop {
        match client.put(put_req.clone()).await {
            Ok(resp) => break resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && retries < 8 => {
                retries += 1;
                tracing::debug!("PUT retry {} due to NOT_LEADER (lazy shard creation)", retries);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("PUT request failed after {} retries: {}", retries, e),
        }
    };

    assert!(
        put_resp.version.is_some(),
        "PUT response should include version"
    );
    tracing::info!("PUT succeeded after {} retries: {:?}", retries, put_resp.version);

    // Wait for apply and stabilization
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Test 2: GET the key back (with retry for lazy shard creation)
    let get_req = GetRequest {
        key: b"test_key".to_vec(),
        consistency: String::new(), // Default consistency
    };

    let mut get_retries = 0;
    let get_resp = loop {
        match client.get(get_req.clone()).await {
            Ok(resp) => break resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && get_retries < 8 => {
                get_retries += 1;
                tracing::debug!("GET retry {} due to NOT_LEADER", get_retries);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("GET request failed after {} retries: {}", get_retries, e),
        }
    };

    assert!(!get_resp.value.is_empty(), "Key should exist");
    assert_eq!(
        get_resp.value,
        b"test_value".to_vec(),
        "Value should match"
    );
    tracing::info!("GET succeeded: value matches");

    // Test 3: DELETE the key (with retry for lazy shard creation)
    let delete_req = DeleteRequest {
        key: b"test_key".to_vec(),
        idempotency_key: String::new(),
        if_match: None,
    };

    let mut delete_retries = 0;
    let delete_resp = loop {
        match client.delete(delete_req.clone()).await {
            Ok(resp) => break resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && delete_retries < 8 => {
                delete_retries += 1;
                tracing::debug!("DELETE retry {} due to NOT_LEADER", delete_retries);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("DELETE request failed after {} retries: {}", delete_retries, e),
        }
    };

    assert!(
        delete_resp.tombstoned,
        "DELETE response should indicate tombstoned"
    );
    tracing::info!("DELETE succeeded: tombstoned={}", delete_resp.tombstoned);

    // Wait for apply
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Test 4: GET should return empty bytes
    let get_req2 = GetRequest {
        key: b"test_key".to_vec(),
        consistency: String::new(),
    };

    let get_resp2 = client
        .get(get_req2)
        .await
        .expect("GET request failed")
        .into_inner();

    assert!(
        get_resp2.value.is_empty(),
        "Key should not exist after DELETE"
    );
    tracing::info!("GET after DELETE correctly returns empty bytes");

    // Test 5: PUT with TTL
    let put_ttl_req = PutRequest {
        key: b"ttl_key".to_vec(),
        value: b"expires_soon".to_vec(),
        ttl_ms: 5000, // 5 second TTL
        idempotency_key: String::new(),
        if_match: None,
    };

    let mut ttl_retries = 0;
    let put_ttl_resp = loop {
        match client.put(put_ttl_req.clone()).await {
            Ok(resp) => break resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && ttl_retries < 8 => {
                ttl_retries += 1;
                tracing::debug!("PUT TTL retry {} due to NOT_LEADER", ttl_retries);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("PUT with TTL failed after {} retries: {}", ttl_retries, e),
        }
    };

    assert!(
        put_ttl_resp.version.is_some(),
        "PUT with TTL should succeed"
    );
    tracing::info!("PUT with TTL succeeded");

    // Shutdown node
    node.shutdown().await.expect("Failed to shutdown node");

    tracing::info!("gRPC integration test completed successfully");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_grpc_get_missing_key() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    // Create temp directory for test
    let temp_dir = TempDir::new().unwrap();

    // Create configuration (use different port to avoid conflict)
    let config = ServerConfig {
        node_id: "grpc_test_missing".to_string(),
        rpc_addr: "127.0.0.1:50052".to_string(),
        http_addr: "127.0.0.1:58052".to_string(), // HTTP on different port
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig::default(),
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    // Create and start node
    let mut node = Node::new(config).await.expect("Failed to create node");
    node.start().await.expect("Failed to start node");

    // Wait for leader election
    // Note: Need at least 1.5-2 seconds for reliable leader election in tests
    tokio::time::sleep(Duration::from_millis(2000)).await;

    // Create gRPC client
    let base_addr = "http://127.0.0.1:50052";
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut client = KvClient::connect(base_addr.to_string())
        .await
        .expect("Failed to connect to gRPC server");

    // GET a key that doesn't exist (with retry for lazy shard creation)
    let get_req = GetRequest {
        key: b"nonexistent_key".to_vec(),
        consistency: String::new(),
    };

    let mut retries = 0;
    let get_resp = loop {
        match client.get(get_req.clone()).await {
            Ok(resp) => break resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && retries < 8 => {
                retries += 1;
                tracing::debug!("GET retry {} due to NOT_LEADER (lazy shard creation)", retries);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("GET request failed after {} retries: {}", retries, e),
        }
    };

    assert!(
        get_resp.value.is_empty(),
        "Nonexistent key should return empty bytes"
    );
    tracing::info!("GET missing key correctly returns empty bytes");

    // Shutdown
    node.shutdown().await.expect("Failed to shutdown node");
}
