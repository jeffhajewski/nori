//! LSM Correctness Tests (#6-9)
//!
//! Core correctness property tests for LSM engine:
//! 6. Compaction Correctness - Preserves all visible data across compactions
//! 7. Snapshot Consistency - (Not yet implemented in LSM)
//! 8. Crash Recovery - WAL replay restores exact pre-crash state
//! 9. TTL Expiration - Expired entries become invisible at expiration time
//!
//! These tests verify the fundamental correctness guarantees of the LSM implementation.

use bytes::Bytes;
use nori_lsm::{ATLLConfig, CompactionMode, LsmEngine};
use std::collections::HashMap;
use std::time::Duration;
use tempfile::TempDir;

// ============================================================================
// Test #6: Compaction Correctness
// ============================================================================

#[tokio::test]
async fn test_compaction_correctness_preserves_data() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    let engine = LsmEngine::open(config.clone()).await.unwrap();

    // Phase 1: Write initial dataset
    let mut expected_state = HashMap::new();
    for i in 0..100 {
        let key = Bytes::from(format!("key_{:03}", i));
        let value = Bytes::from(format!("value_v1_{}", i));
        engine.put(key.clone(), value.clone(), None).await.unwrap();
        expected_state.insert(key, value);
    }

    // Phase 2: Update some keys
    for i in 0..50 {
        let key = Bytes::from(format!("key_{:03}", i));
        let value = Bytes::from(format!("value_v2_{}", i));
        engine.put(key.clone(), value.clone(), None).await.unwrap();
        expected_state.insert(key, value);
    }

    // Phase 3: Delete some keys
    for i in 25..40 {
        let key = Bytes::from(format!("key_{:03}", i));
        engine.delete(&key).await.unwrap();
        expected_state.remove(&key);
    }

    // Phase 4: Flush and compact
    engine.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    engine
        .compact_range(Some(b"key_000"), Some(b"key_999"), CompactionMode::Eager)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Phase 5: Verify all expected data still exists
    for (key, expected_value) in &expected_state {
        match engine.get(key).await.unwrap() {
            Some(actual_value) => {
                assert_eq!(actual_value, *expected_value, "Value mismatch for key {:?}", key);
            }
            None => {
                panic!("Key {:?} missing after compaction", key);
            }
        }
    }

    // Phase 6: Verify deleted keys remain deleted
    for i in 25..40 {
        let key = Bytes::from(format!("key_{:03}", i));
        assert!(engine.get(&key).await.unwrap().is_none(), "Deleted key {:?} should not exist", key);
    }

    engine.shutdown().await.unwrap();
}

// ============================================================================
// Test #8: Crash Recovery
// ============================================================================

#[tokio::test]
async fn test_crash_recovery_wal_replay() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    // Phase 1: Write data without flushing
    let mut expected_state = HashMap::new();

    {
        let engine = LsmEngine::open(config.clone()).await.unwrap();

        for i in 0..50 {
            let key = Bytes::from(format!("recovery_key_{}", i));
            let value = Bytes::from(format!("recovery_value_{}", i));
            engine.put(key.clone(), value.clone(), None).await.unwrap();
            expected_state.insert(key, value);
        }

        // Simulate crash by dropping engine without shutdown
    }

    // Phase 2: Reopen engine (should replay WAL)
    let engine = LsmEngine::open(config.clone()).await.unwrap();

    // Phase 3: Verify all data was recovered
    for (key, expected_value) in &expected_state {
        match engine.get(key).await.unwrap() {
            Some(actual_value) => {
                assert_eq!(actual_value, *expected_value, "Value mismatch after recovery for key {:?}", key);
            }
            None => {
                panic!("Key {:?} missing after WAL recovery", key);
            }
        }
    }

    engine.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_crash_recovery_with_deletes() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    let mut expected_state = HashMap::new();

    // Phase 1: Write and delete without flushing
    {
        let engine = LsmEngine::open(config.clone()).await.unwrap();

        for i in 0..30 {
            let key = Bytes::from(format!("crash_key_{}", i));
            let value = Bytes::from(format!("value_{}", i));
            engine.put(key.clone(), value.clone(), None).await.unwrap();
            expected_state.insert(key, value);
        }

        for i in 0..15 {
            let key = Bytes::from(format!("crash_key_{}", i));
            engine.delete(&key).await.unwrap();
            expected_state.remove(&key);
        }
    }

    // Phase 2: Reopen and verify
    let engine = LsmEngine::open(config).await.unwrap();

    // Verify remaining keys exist
    for (key, expected_value) in &expected_state {
        let actual_value = engine.get(key).await.unwrap();
        assert!(actual_value.is_some(), "Key {:?} should exist after recovery", key);
        assert_eq!(actual_value.unwrap(), *expected_value, "Value mismatch for key {:?}", key);
    }

    // Verify deleted keys are gone
    for i in 0..15 {
        let key = Bytes::from(format!("crash_key_{}", i));
        assert!(engine.get(&key).await.unwrap().is_none(), "Deleted key {:?} should not exist after recovery", key);
    }

    engine.shutdown().await.unwrap();
}

// ============================================================================
// Test #9: TTL Expiration
// ============================================================================

#[tokio::test]
async fn test_ttl_expiration() {
    let temp_dir = TempDir::new().unwrap();
    let mut config = ATLLConfig::default();
    config.data_dir = temp_dir.path().to_path_buf();

    let engine = LsmEngine::open(config).await.unwrap();

    // Phase 1: Write keys with short TTL
    let short_ttl = Duration::from_millis(500);
    for i in 0..10 {
        let key = Bytes::from(format!("ttl_short_{}", i));
        let value = Bytes::from(format!("expires_soon_{}", i));
        engine.put(key, value, Some(short_ttl)).await.unwrap();
    }

    // Phase 2: Write keys with long TTL
    let long_ttl = Duration::from_secs(10);
    for i in 0..10 {
        let key = Bytes::from(format!("ttl_long_{}", i));
        let value = Bytes::from(format!("expires_later_{}", i));
        engine.put(key, value, Some(long_ttl)).await.unwrap();
    }

    // Phase 3: Verify all keys exist initially
    for i in 0..10 {
        let short_key = Bytes::from(format!("ttl_short_{}", i));
        let long_key = Bytes::from(format!("ttl_long_{}", i));
        assert!(engine.get(&short_key).await.unwrap().is_some(), "Short TTL key should exist initially");
        assert!(engine.get(&long_key).await.unwrap().is_some(), "Long TTL key should exist initially");
    }

    // Phase 4: Wait for short TTL to expire
    tokio::time::sleep(Duration::from_millis(600)).await;

    // Phase 5: Verify short TTL keys are gone, long TTL keys remain
    for i in 0..10 {
        let short_key = Bytes::from(format!("ttl_short_{}", i));
        let long_key = Bytes::from(format!("ttl_long_{}", i));

        let short_value = engine.get(&short_key).await.unwrap();
        assert!(short_value.is_none(), "Short TTL key should be expired");

        let long_value = engine.get(&long_key).await.unwrap();
        assert!(long_value.is_some(), "Long TTL key should still exist");
    }

    engine.shutdown().await.unwrap();
}
