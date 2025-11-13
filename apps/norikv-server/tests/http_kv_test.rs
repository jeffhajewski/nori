//! HTTP KV REST API integration tests.
//!
//! Verifies that HTTP clients can perform KV operations via REST API.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use tempfile::TempDir;
use tokio::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn test_http_kv_put_get_delete() {
    // Initialize tracing for test visibility
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    tracing::info!("Starting HTTP KV integration test");

    // Create temp directory for test
    let temp_dir = TempDir::new().unwrap();

    // Create configuration with fixed ports for testing
    let config = ServerConfig {
        node_id: "http_kv_test_node".to_string(),
        rpc_addr: "127.0.0.1:50052".to_string(),
        http_addr: "127.0.0.1:58052".to_string(),
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
    tokio::time::sleep(Duration::from_millis(2000)).await;

    tracing::info!("Node started and became leader");

    let base_url = "http://127.0.0.1:58052";
    let client = reqwest::Client::new();

    // Wait for HTTP server to be ready
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Test 1: PUT a key (with retry for lazy shard creation)
    let key = "test_key";
    let value = b"test_value";

    let mut retries = 0;
    let put_response = loop {
        let resp = client
            .put(format!("{}/kv/{}", base_url, key))
            .body(value.to_vec())
            .send()
            .await
            .expect("Failed to send PUT request");

        if resp.status().is_success() {
            break resp;
        } else if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE && retries < 8 {
            retries += 1;
            tracing::debug!("PUT retry {} due to NOT_LEADER (lazy shard creation)", retries);
            tokio::time::sleep(Duration::from_millis(500)).await;
        } else {
            panic!("PUT request failed with status: {} after {} retries", resp.status(), retries);
        }
    };

    assert_eq!(put_response.status(), reqwest::StatusCode::OK);
    let put_body: serde_json::Value = put_response.json().await.expect("Failed to parse PUT response");
    assert!(put_body["version"]["term"].is_number(), "PUT should return version with term");
    assert!(put_body["version"]["index"].is_number(), "PUT should return version with index");
    tracing::info!("PUT succeeded after {} retries: {:?}", retries, put_body);

    // Wait for apply and stabilization
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Test 2: GET the key back (with retry for lazy shard creation)
    let mut retries = 0;
    let get_response = loop {
        let resp = client
            .get(format!("{}/kv/{}", base_url, key))
            .send()
            .await
            .expect("Failed to send GET request");

        if resp.status().is_success() {
            break resp;
        } else if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE && retries < 8 {
            retries += 1;
            tracing::debug!("GET retry {} due to NOT_LEADER (lazy shard creation)", retries);
            tokio::time::sleep(Duration::from_millis(500)).await;
        } else {
            panic!("GET request failed with status: {} after {} retries", resp.status(), retries);
        }
    };

    assert_eq!(get_response.status(), reqwest::StatusCode::OK);
    let get_body = get_response.bytes().await.expect("Failed to read GET response body");
    assert_eq!(get_body.as_ref(), value, "GET should return the same value");
    tracing::info!("GET succeeded after {} retries: {:?}", retries, String::from_utf8_lossy(&get_body));

    // Test 3: DELETE the key (with retry)
    let mut retries = 0;
    let delete_response = loop {
        let resp = client
            .delete(format!("{}/kv/{}", base_url, key))
            .send()
            .await
            .expect("Failed to send DELETE request");

        if resp.status().is_success() {
            break resp;
        } else if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE && retries < 8 {
            retries += 1;
            tracing::debug!("DELETE retry {} due to NOT_LEADER", retries);
            tokio::time::sleep(Duration::from_millis(500)).await;
        } else {
            panic!("DELETE request failed with status: {} after {} retries", resp.status(), retries);
        }
    };

    assert_eq!(delete_response.status(), reqwest::StatusCode::OK);
    tracing::info!("DELETE succeeded after {} retries", retries);

    // Wait for deletion to be applied
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Test 4: GET should now return 404
    let get_response = client
        .get(format!("{}/kv/{}", base_url, key))
        .send()
        .await
        .expect("Failed to send GET request");

    assert_eq!(get_response.status(), reqwest::StatusCode::NOT_FOUND, "GET after DELETE should return 404");
    tracing::info!("GET after DELETE returned 404 as expected");

    tracing::info!("HTTP KV integration test completed successfully");
}
