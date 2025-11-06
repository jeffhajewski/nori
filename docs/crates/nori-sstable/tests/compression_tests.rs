use nori_sstable::{Compression, Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_compression_lz4_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("lz4.sst");

    // Build with LZ4 compression
    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        compression: Compression::Lz4,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add test data (should compress well - repetitive values)
    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = "test_value_that_should_compress_well_".repeat(10);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata = builder.finish().await.unwrap();
    assert_eq!(metadata.entry_count, 100);

    // Read and verify all entries
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Key not found: {}", key);

        let entry = entry.unwrap();
        assert_eq!(entry.key.as_ref(), key.as_bytes());
        assert_eq!(
            entry.value.as_ref(),
            "test_value_that_should_compress_well_"
                .repeat(10)
                .as_bytes()
        );
    }
}

#[tokio::test]
async fn test_compression_zstd_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("zstd.sst");

    // Build with Zstd compression
    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        compression: Compression::Zstd,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add test data
    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = format!("value_{:04}_{}", i, "x".repeat(100));
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata = builder.finish().await.unwrap();
    assert_eq!(metadata.entry_count, 100);

    // Read and verify
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap();
        assert!(entry.is_some(), "Key not found: {}", key);

        let entry = entry.unwrap();
        let expected_value = format!("value_{:04}_{}", i, "x".repeat(100));
        assert_eq!(entry.value.as_ref(), expected_value.as_bytes());
    }
}

#[tokio::test]
async fn test_compression_none_vs_lz4() {
    let dir = TempDir::new().unwrap();
    let path_none = dir.path().join("none.sst");
    let path_lz4 = dir.path().join("lz4.sst");

    // Build without compression
    let config_none = SSTableConfig {
        path: path_none.clone(),
        estimated_entries: 100,
        compression: Compression::None,
        ..Default::default()
    };

    let mut builder_none = SSTableBuilder::new(config_none).await.unwrap();

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = "test_value_".repeat(10);
        builder_none.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata_none = builder_none.finish().await.unwrap();

    // Build with LZ4 compression
    let config_lz4 = SSTableConfig {
        path: path_lz4.clone(),
        estimated_entries: 100,
        compression: Compression::Lz4,
        ..Default::default()
    };

    let mut builder_lz4 = SSTableBuilder::new(config_lz4).await.unwrap();

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = "test_value_".repeat(10);
        builder_lz4.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata_lz4 = builder_lz4.finish().await.unwrap();

    // Compressed file should be smaller
    assert!(
        metadata_lz4.file_size < metadata_none.file_size,
        "LZ4 compressed size {} should be smaller than uncompressed {}",
        metadata_lz4.file_size,
        metadata_none.file_size
    );

    println!(
        "Compression ratio: {:.2}x (uncompressed: {} bytes, compressed: {} bytes)",
        metadata_none.file_size as f64 / metadata_lz4.file_size as f64,
        metadata_none.file_size,
        metadata_lz4.file_size
    );

    // Verify both have same data
    let reader_none = Arc::new(SSTableReader::open(path_none).await.unwrap());
    let reader_lz4 = Arc::new(SSTableReader::open(path_lz4).await.unwrap());

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let entry_none = reader_none.get(key.as_bytes()).await.unwrap().unwrap();
        let entry_lz4 = reader_lz4.get(key.as_bytes()).await.unwrap().unwrap();

        assert_eq!(entry_none.key, entry_lz4.key);
        assert_eq!(entry_none.value, entry_lz4.value);
    }
}

#[tokio::test]
async fn test_compression_iterator() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("iter.sst");

    // Build with compression
    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 50,
        compression: Compression::Lz4,
        block_size: 512, // Small blocks to test multiple block decompression
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..50 {
        let key = format!("key_{:04}", i);
        let value = "x".repeat(100);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Iterate and verify
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());
    let mut iter = reader.iter();

    let mut count = 0;
    while let Some(entry) = iter.try_next().await.unwrap() {
        let expected_key = format!("key_{:04}", count);
        assert_eq!(entry.key.as_ref(), expected_key.as_bytes());
        assert_eq!(entry.value.len(), 100);
        count += 1;
    }

    assert_eq!(count, 50);
}

#[tokio::test]
async fn test_compression_with_cache() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("cache.sst");

    // Build with compression
    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        compression: Compression::Lz4,
        block_cache_mb: 10, // Enable cache
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..100 {
        let key = format!("key_{:04}", i);
        let value = format!("value_{:04}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read with cache enabled
    let reader = Arc::new(
        SSTableReader::open_with_config(path, Arc::new(nori_observe::NoopMeter), 10)
            .await
            .unwrap(),
    );

    // First read - cache miss
    let key = b"key_0050";
    let entry1 = reader.get(key).await.unwrap().unwrap();

    // Second read - cache hit (decompressed block is cached)
    let entry2 = reader.get(key).await.unwrap().unwrap();

    assert_eq!(entry1.key, entry2.key);
    assert_eq!(entry1.value, entry2.value);
}

#[tokio::test]
async fn test_compression_tombstones() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tombstones.sst");

    // Build with compression and tombstones
    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 20,
        compression: Compression::Lz4,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..10 {
        let key = format!("key_{:04}", i);
        builder.add(&Entry::put(key, "value")).await.unwrap();
    }

    for i in 10..20 {
        let key = format!("key_{:04}", i);
        builder.add(&Entry::delete(key)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read and verify
    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    for i in 0..10 {
        let key = format!("key_{:04}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap().unwrap();
        assert!(!entry.tombstone, "Expected live entry");
    }

    for i in 10..20 {
        let key = format!("key_{:04}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap().unwrap();
        assert!(entry.tombstone, "Expected tombstone");
    }
}

#[tokio::test]
async fn test_compression_empty_values() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 10,
        compression: Compression::Lz4,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..10 {
        let key = format!("key_{:04}", i);
        builder.add(&Entry::put(key, "")).await.unwrap();
    }

    builder.finish().await.unwrap();

    let reader = Arc::new(SSTableReader::open(path).await.unwrap());

    for i in 0..10 {
        let key = format!("key_{:04}", i);
        let entry = reader.get(key.as_bytes()).await.unwrap().unwrap();
        assert_eq!(entry.value.len(), 0, "Expected empty value");
    }
}
