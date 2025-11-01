//! Integration tests for ReplicatedLSM.
//!
//! These tests verify the end-to-end integration between Raft consensus and LSM storage.
//! Focus areas:
//! - Commands flow through Raft proposal → commit → apply → LSM
//! - State machine correctly applies Put/Delete operations
//! - Linearizable reads work correctly
//! - Persistence survives restarts

use bytes::Bytes;
use nori_raft::transport::InMemoryTransport;
use nori_raft::{ConfigEntry, NodeId, RaftConfig, ReplicatedLSM};
use nori_lsm::ATLLConfig;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

/// Helper to create a single-node ReplicatedLSM for integration testing.
async fn create_single_node_cluster() -> (Arc<ReplicatedLSM>, TempDir, TempDir) {
    let node_id = NodeId::new("node1");
    let raft_dir = TempDir::new().unwrap();
    let lsm_dir = TempDir::new().unwrap();

    let mut raft_config = RaftConfig::default();
    raft_config.election_timeout_min = Duration::from_millis(150);
    raft_config.election_timeout_max = Duration::from_millis(300);

    let mut lsm_config = ATLLConfig::default();
    lsm_config.data_dir = lsm_dir.path().to_path_buf();

    let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
        .await
        .unwrap();

    let transport: Arc<dyn nori_raft::transport::RaftTransport> =
        Arc::new(InMemoryTransport::new(node_id.clone(), HashMap::new()));

    let initial_config = ConfigEntry::Single(vec![node_id.clone()]);

    let replicated_lsm = ReplicatedLSM::new(
        node_id,
        raft_config,
        lsm_config,
        raft_log,
        transport,
        initial_config,
    )
    .await
    .unwrap();

    (Arc::new(replicated_lsm), raft_dir, lsm_dir)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_end_to_end_put_get_delete() {
    let (replicated_lsm, _raft_dir, _lsm_dir) = create_single_node_cluster().await;

    // Start the node
    replicated_lsm.start().await.unwrap();

    // Wait for leader election
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(replicated_lsm.is_leader(), "Single node should become leader");

    // Test 1: Write a key-value pair
    let key1 = Bytes::from("integration_key1");
    let value1 = Bytes::from("integration_value1");

    let index = replicated_lsm
        .replicated_put(key1.clone(), value1.clone(), None)
        .await
        .expect("Put should succeed on leader");

    assert!(index.0 > 0, "Should get valid log index");

    // Wait for application
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Test 2: Read the value back
    let result = replicated_lsm
        .replicated_get(b"integration_key1")
        .await
        .expect("Get should succeed");

    assert_eq!(result, Some(Bytes::from("integration_value1")));

    // Test 3: Write another key
    replicated_lsm
        .replicated_put(Bytes::from("key2"), Bytes::from("value2"), None)
        .await
        .expect("Second put should succeed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Test 4: Read both keys
    let result1 = replicated_lsm.replicated_get(b"integration_key1").await.unwrap();
    let result2 = replicated_lsm.replicated_get(b"key2").await.unwrap();

    assert_eq!(result1, Some(Bytes::from("integration_value1")));
    assert_eq!(result2, Some(Bytes::from("value2")));

    // Test 5: Delete a key
    replicated_lsm
        .replicated_delete(Bytes::from("key2"))
        .await
        .expect("Delete should succeed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Test 6: Verify deletion
    let result = replicated_lsm.replicated_get(b"key2").await.unwrap();
    assert_eq!(result, None, "Deleted key should not exist");

    // Test 7: First key should still exist
    let result = replicated_lsm.replicated_get(b"integration_key1").await.unwrap();
    assert_eq!(result, Some(Bytes::from("integration_value1")));

    replicated_lsm.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_linearizable_reads_require_leadership() {
    let (replicated_lsm, _raft_dir, _lsm_dir) = create_single_node_cluster().await;

    // Don't start - node remains follower

    // Try to read (should fail - not leader)
    let result = replicated_lsm.replicated_get(b"any_key").await;

    assert!(result.is_err(), "Read should fail when not leader");
    assert!(
        matches!(result, Err(nori_raft::error::RaftError::NotLeader { .. })),
        "Should return NotLeader error"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bulk_operations() {
    let (replicated_lsm, _raft_dir, _lsm_dir) = create_single_node_cluster().await;

    replicated_lsm.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Write 100 key-value pairs
    for i in 0..100 {
        let key = Bytes::from(format!("bulk_key_{}", i));
        let value = Bytes::from(format!("bulk_value_{}", i));

        replicated_lsm
            .replicated_put(key, value, None)
            .await
            .expect(&format!("Put {} should succeed", i));
    }

    // Wait for all to apply
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Verify all keys exist
    for i in 0..100 {
        let key = format!("bulk_key_{}", i);
        let expected_value = format!("bulk_value_{}", i);

        let result = replicated_lsm
            .replicated_get(key.as_bytes())
            .await
            .expect("Get should succeed");

        assert_eq!(
            result,
            Some(Bytes::from(expected_value)),
            "Key {} should have correct value",
            i
        );
    }

    replicated_lsm.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_ttl_operations() {
    let (replicated_lsm, _raft_dir, _lsm_dir) = create_single_node_cluster().await;

    replicated_lsm.start().await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Write a key with TTL
    let key = Bytes::from("ttl_key");
    let value = Bytes::from("ttl_value");
    let ttl = Some(Duration::from_secs(1));

    replicated_lsm
        .replicated_put(key.clone(), value.clone(), ttl)
        .await
        .expect("Put with TTL should succeed");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // Key should exist initially
    let result = replicated_lsm.replicated_get(b"ttl_key").await.unwrap();
    assert_eq!(result, Some(Bytes::from("ttl_value")));

    // Wait for TTL to expire
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Key should be gone (TTL expired)
    let result = replicated_lsm.replicated_get(b"ttl_key").await.unwrap();
    assert_eq!(result, None, "Key should expire after TTL");

    replicated_lsm.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn test_persistence_across_restart() {
    let raft_dir = TempDir::new().unwrap();
    let lsm_dir = TempDir::new().unwrap();

    // First session: write some data
    {
        let node_id = NodeId::new("persist_node");

        let mut raft_config = RaftConfig::default();
        raft_config.election_timeout_min = Duration::from_millis(150);
        raft_config.election_timeout_max = Duration::from_millis(300);

        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = lsm_dir.path().to_path_buf();

        let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
            .await
            .unwrap();

        let transport: Arc<dyn nori_raft::transport::RaftTransport> =
            Arc::new(InMemoryTransport::new(node_id.clone(), HashMap::new()));

        let initial_config = ConfigEntry::Single(vec![node_id.clone()]);

        let replicated_lsm = ReplicatedLSM::new(
            node_id,
            raft_config,
            lsm_config,
            raft_log,
            transport,
            initial_config,
        )
        .await
        .unwrap();

        replicated_lsm.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Write data
        replicated_lsm
            .replicated_put(Bytes::from("persist_key"), Bytes::from("persist_value"), None)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify it exists
        let result = replicated_lsm.replicated_get(b"persist_key").await.unwrap();
        assert_eq!(result, Some(Bytes::from("persist_value")));

        replicated_lsm.shutdown().await.unwrap();
    }

    // Second session: reopen and verify data persisted
    {
        let node_id = NodeId::new("persist_node");

        let mut raft_config = RaftConfig::default();
        raft_config.election_timeout_min = Duration::from_millis(150);
        raft_config.election_timeout_max = Duration::from_millis(300);

        let mut lsm_config = ATLLConfig::default();
        lsm_config.data_dir = lsm_dir.path().to_path_buf();

        let (raft_log, _) = nori_raft::log::RaftLog::open(raft_dir.path())
            .await
            .unwrap();

        let transport: Arc<dyn nori_raft::transport::RaftTransport> =
            Arc::new(InMemoryTransport::new(node_id.clone(), HashMap::new()));

        let initial_config = ConfigEntry::Single(vec![node_id.clone()]);

        let replicated_lsm = ReplicatedLSM::new(
            node_id,
            raft_config,
            lsm_config,
            raft_log,
            transport,
            initial_config,
        )
        .await
        .unwrap();

        replicated_lsm.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Data should still exist after restart
        let result = replicated_lsm.replicated_get(b"persist_key").await.unwrap();
        assert_eq!(
            result,
            Some(Bytes::from("persist_value")),
            "Data should persist across restarts"
        );

        replicated_lsm.shutdown().await.unwrap();
    }
}
