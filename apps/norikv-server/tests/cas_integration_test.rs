//! CAS (Compare-And-Set) integration tests for norikv-server.
//!
//! Verifies that CAS operations work correctly end-to-end through the gRPC stack.

use norikv_server::config::ServerConfig;
use norikv_server::node::Node;
use tempfile::TempDir;
use tokio::time::Duration;

use norikv_transport_grpc::proto::{kv_client::KvClient, PutRequest, Version};

async fn setup_test_node(port_offset: u16) -> (Node, KvClient<tonic::transport::Channel>, TempDir) {
    let temp_dir = TempDir::new().unwrap();

    let config = ServerConfig {
        node_id: format!("cas_test_node_{}", port_offset),
        rpc_addr: format!("127.0.0.1:{}", 50060 + port_offset),
        http_addr: format!("127.0.0.1:{}", 58060 + port_offset),
        data_dir: temp_dir.path().to_path_buf(),
        cluster: norikv_server::config::ClusterConfig {
            seed_nodes: vec![],
            total_shards: 1024,
            replication_factor: 3,
        },
        telemetry: norikv_server::config::TelemetryConfig::default(),
    };

    let mut node = Node::new(config.clone()).await.expect("Failed to create node");
    node.start().await.expect("Failed to start node");

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(2000)).await;

    let client = KvClient::connect(format!("http://{}", config.rpc_addr))
        .await
        .expect("Failed to connect to gRPC server");

    (node, client, temp_dir)
}

async fn put_with_retry(
    client: &mut KvClient<tonic::transport::Channel>,
    req: PutRequest,
) -> norikv_transport_grpc::proto::PutResponse {
    let mut retries = 0;
    loop {
        match client.put(req.clone()).await {
            Ok(resp) => return resp.into_inner(),
            Err(e) if e.message().contains("NOT_LEADER") && retries < 8 => {
                retries += 1;
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("PUT failed after {} retries: {}", retries, e),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cas_success_when_version_matches() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (_node, mut client, _temp_dir) = setup_test_node(0).await;

    // Initial PUT without CAS
    let put_req = PutRequest {
        key: b"cas_key_1".to_vec(),
        value: b"initial_value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let put_resp = put_with_retry(&mut client, put_req).await;
    let version = put_resp.version.expect("PUT should return version");

    tracing::info!("Initial PUT succeeded with version: term={}, index={}", version.term, version.index);

    // Wait for commit
    tokio::time::sleep(Duration::from_millis(500)).await;

    // CAS PUT with matching version
    let cas_req = PutRequest {
        key: b"cas_key_1".to_vec(),
        value: b"updated_value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: Some(version.clone()),
    };

    let cas_resp = put_with_retry(&mut client, cas_req).await;
    assert!(cas_resp.version.is_some(), "CAS should succeed and return version");

    tracing::info!("CAS succeeded with new version: {:?}", cas_resp.version);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cas_fails_when_version_mismatches() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (_node, mut client, _temp_dir) = setup_test_node(1).await;

    // Initial PUT
    let put_req = PutRequest {
        key: b"cas_key_2".to_vec(),
        value: b"initial_value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let put_resp = put_with_retry(&mut client, put_req).await;
    let initial_version = put_resp.version.expect("PUT should return version");

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Update the key again (without CAS)
    let update_req = PutRequest {
        key: b"cas_key_2".to_vec(),
        value: b"updated_value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let update_resp = put_with_retry(&mut client, update_req).await;
    let new_version = update_resp.version.expect("PUT should return version");

    tracing::info!("Updated version: term={}, index={}", new_version.term, new_version.index);

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Try CAS with old/stale version (should fail)
    let stale_cas_req = PutRequest {
        key: b"cas_key_2".to_vec(),
        value: b"cas_attempt".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: Some(initial_version.clone()),
    };

    let result = client.put(stale_cas_req).await;
    assert!(result.is_err(), "CAS with stale version should fail");

    let err = result.unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "Should return FAILED_PRECONDITION");
    assert!(err.message().contains("Version mismatch"), "Error message should mention version mismatch");

    tracing::info!("CAS correctly failed with stale version: {}", err.message());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cas_fails_when_key_not_exists() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (_node, mut client, _temp_dir) = setup_test_node(2).await;

    // Do an initial PUT to ensure the shard is created and ready
    // This prevents lazy shard creation issues during CAS
    let warmup_req = PutRequest {
        key: b"warmup_key".to_vec(),
        value: b"warmup".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };
    put_with_retry(&mut client, warmup_req).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // Now try CAS on a different non-existent key (same shard should be ready)
    let cas_req = PutRequest {
        key: b"nonexistent_key".to_vec(),
        value: b"value".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: Some(Version {
            term: 1,
            index: 1,
        }),
    };

    let result = client.put(cas_req).await;
    assert!(result.is_err(), "CAS on non-existent key should fail");

    let err = result.unwrap_err();
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "Should return FAILED_PRECONDITION");
    assert!(err.message().contains("does not exist"), "Error message should mention key doesn't exist");

    tracing::info!("CAS correctly failed on non-existent key: {}", err.message());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_put_without_cas_overwrites() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (_node, mut client, _temp_dir) = setup_test_node(3).await;

    // Initial PUT
    let put_req = PutRequest {
        key: b"overwrite_key".to_vec(),
        value: b"value1".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    put_with_retry(&mut client, put_req).await;

    tokio::time::sleep(Duration::from_millis(500)).await;

    // PUT again without CAS (should succeed)
    let overwrite_req = PutRequest {
        key: b"overwrite_key".to_vec(),
        value: b"value2".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let result = put_with_retry(&mut client, overwrite_req).await;
    assert!(result.version.is_some(), "Overwrite without CAS should succeed");

    tracing::info!("PUT without CAS successfully overwrote existing key");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_cas_sequential_updates() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init()
        .ok();

    let (_node, mut client, _temp_dir) = setup_test_node(4).await;

    // Initial PUT
    let put_req = PutRequest {
        key: b"sequential_key".to_vec(),
        value: b"v1".to_vec(),
        ttl_ms: 0,
        idempotency_key: String::new(),
        if_match: None,
    };

    let mut version = put_with_retry(&mut client, put_req).await.version.unwrap();
    tracing::info!("Initial version: term={}, index={}", version.term, version.index);

    // Perform 5 sequential CAS updates
    for i in 2..=6 {
        tokio::time::sleep(Duration::from_millis(300)).await;

        let cas_req = PutRequest {
            key: b"sequential_key".to_vec(),
            value: format!("v{}", i).into_bytes(),
            ttl_ms: 0,
            idempotency_key: String::new(),
            if_match: Some(version.clone()),
        };

        let resp = put_with_retry(&mut client, cas_req).await;
        version = resp.version.expect("CAS should return version");

        tracing::info!("CAS update {} succeeded: term={}, index={}", i, version.term, version.index);
    }

    tracing::info!("Sequential CAS updates completed successfully");
}
