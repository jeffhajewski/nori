use nori_sstable::{Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_large_sstable() {
    // Build SSTable with 1M+ entries
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.sst");

    let num_entries = 1_000_000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add 1M entries
    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = "x".repeat(100);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata = builder.finish().await.unwrap();
    assert_eq!(metadata.entry_count, num_entries as u64);

    // Verify all entries are readable
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    // Sample every 10,000th key
    for i in (0..num_entries).step_by(10_000) {
        let key = format!("key_{:08}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Key not found: {}", key);
        assert_eq!(entry.unwrap().value.len(), 100);
    }

    // Test iterator over full table
    let mut iter = reader.clone().iter();
    let mut count = 0;
    let mut last_key = Vec::new();

    while let Some(entry) = iter.try_next().await.unwrap() {
        // Verify sorted order
        assert!(
            entry.key.as_ref() > last_key.as_slice() || last_key.is_empty(),
            "Keys not sorted"
        );
        last_key = entry.key.to_vec();
        count += 1;
    }

    assert_eq!(count, num_entries);
}

#[tokio::test]
async fn test_concurrent_reads() {
    // Test multiple readers on same SSTable (Arc safety)
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("concurrent.sst");

    let num_entries = 10_000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = format!("value_{:08}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    // Spawn multiple concurrent read tasks
    let mut handles = vec![];

    for task_id in 0..10 {
        let reader_clone = reader.clone();
        let handle = tokio::spawn(async move {
            // Each task reads different keys
            for i in (task_id * 1000..(task_id + 1) * 1000).step_by(10) {
                let key = format!("key_{:08}", i);
                let entry = reader_clone.get(key.as_bytes()).await.unwrap();
                assert!(entry.is_some(), "Task {}: key not found: {}", task_id, key);
            }
        });
        handles.push(handle);
    }

    // Wait for all tasks
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_many_blocks() {
    // Force 1000+ blocks with small block size
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("many_blocks.sst");

    let num_entries = 10_000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        block_size: 256, // Very small to force many blocks
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = "x".repeat(100);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Verify we have many blocks
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());
    assert!(
        reader.block_count() > 1000,
        "Expected >1000 blocks, got {}",
        reader.block_count()
    );

    // Verify index correctness by reading keys across blocks
    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Key not found: {}", key);
    }
}

#[tokio::test]
async fn test_max_bloom_size() {
    // Test bloom filter with very large cardinality
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large_bloom.sst");

    let num_entries = 100_000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        bloom_bits_per_key: 10,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = "value";
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    // Verify bloom filter works for hits
    let mut hits = 0;
    for i in (0..num_entries).step_by(1000) {
        let key = format!("key_{:08}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        if entry.is_some() {
            hits += 1;
        }
    }

    assert_eq!(hits, 100, "All sampled keys should be found");

    // Verify bloom filter rejects most misses
    let mut misses = 0;
    let mut false_positives = 0;

    for i in num_entries..(num_entries + 10_000) {
        let key = format!("key_{:08}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();

        if entry.is_none() {
            misses += 1;
        } else {
            false_positives += 1;
        }
    }

    // FP rate should be around 0.9% (90 out of 10000)
    // Allow up to 2% for variance
    assert!(
        false_positives <= 200,
        "Too many false positives: {} / 10000",
        false_positives
    );
    assert!(misses >= 9800, "Expected most lookups to miss");
}

#[tokio::test]
async fn test_seek_after_boundary() {
    // Test iterator seek across block boundaries
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("seek.sst");

    let num_entries = 1000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        block_size: 512, // Create multiple blocks
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let value = "x".repeat(50);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());
    assert!(
        reader.block_count() > 5,
        "Need multiple blocks for this test"
    );

    // Seek to various positions
    for seek_pos in [0, 100, 250, 500, 750, 999] {
        let mut iter = reader.clone().iter();
        let target = format!("key_{:08}", seek_pos);

        iter.seek(target.as_bytes()).await.unwrap();

        // Next entry should be >= target
        if let Some(entry) = iter.try_next().await.unwrap() {
            assert!(
                entry.key.as_ref() >= target.as_bytes(),
                "Seek failed: got {:?}, expected >= {:?}",
                entry.key,
                target
            );
        }
    }
}

#[tokio::test]
async fn test_all_tombstones() {
    // SSTable with all tombstones
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tombstones.sst");

    let num_entries = 1000;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        builder.add(&Entry::delete(key)).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    // Verify all are tombstones
    for i in 0..num_entries {
        let key = format!("key_{:08}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Tombstone not found: {}", key);
        assert!(
            entry.unwrap().tombstone,
            "Expected tombstone for key: {}",
            key
        );
    }
}

#[tokio::test]
async fn test_very_large_keys_and_values() {
    // Test with 10KB keys and 100KB values
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large_kv.sst");

    let num_entries = 100;

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: num_entries,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..num_entries {
        let key = format!("key_{:08}_{}", i, "x".repeat(10 * 1024));
        let value = "x".repeat(100 * 1024);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    // Verify all entries readable
    for i in 0..num_entries {
        let key = format!("key_{:08}_{}", i, "x".repeat(10 * 1024));
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Large key not found: {}", i);
        assert_eq!(entry.unwrap().value.len(), 100 * 1024);
    }
}

#[tokio::test]
async fn test_sequential_build_and_read() {
    // Build and immediately read in a loop
    for iteration in 0..10 {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(format!("seq_{}.sst", iteration));

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 100,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        for i in 0..100 {
            let key = format!("key_{:08}", i);
            let value = format!("value_{}_{}", iteration, i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        builder.finish().await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());

        // Verify all keys
        for i in 0..100 {
            let key = format!("key_{:08}", i);
            let entry = reader.get(key.as_bytes()).await.unwrap();
            assert!(entry.is_some(), "Iteration {}: key not found", iteration);
        }
    }
}
