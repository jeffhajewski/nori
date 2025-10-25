//! Integration tests for SSTable builder and reader roundtrip.
//!
//! These tests verify that the complete SSTable lifecycle works:
//! 1. Build an SSTable with entries
//! 2. Write to disk
//! 3. Read back from disk
//! 4. Verify all data is correct

use nori_sstable::{
    Block, BloomFilter, Compression, Entry, Footer, Index, SSTableBuilder, SSTableConfig,
};
use tempfile::TempDir;
use tokio::fs;

#[tokio::test]
async fn test_sstable_build_basic() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("basic.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add sorted entries
    for i in 0..100 {
        let key = format!("key{:04}", i);
        let value = format!("value{:04}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata = builder.finish().await.unwrap();

    // Verify metadata
    assert_eq!(metadata.entry_count, 100);
    assert_eq!(metadata.path, path);
    assert!(metadata.file_size > 0);
    assert!(metadata.block_count > 0);

    // Verify file exists
    let file_metadata = fs::metadata(&path).await.unwrap();
    assert_eq!(file_metadata.len(), metadata.file_size);
}

#[tokio::test]
async fn test_sstable_build_large() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("large.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 10000,
        block_size: 4096, // Standard block size
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Add 10,000 entries with larger values
    for i in 0..10000 {
        let key = format!("user_{:08}", i);
        let value = format!(
            "{{\"id\":{},\"name\":\"User {}\",\"email\":\"user{}@example.com\"}}",
            i, i, i
        );
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    let metadata = builder.finish().await.unwrap();

    assert_eq!(metadata.entry_count, 10000);
    assert!(metadata.block_count > 10, "Should have many blocks");
    assert!(metadata.file_size > 100_000, "File should be substantial");
}

#[tokio::test]
async fn test_sstable_with_tombstones() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("tombstones.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 50,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    // Mix of puts and deletes
    builder
        .add(&Entry::put(&b"key01"[..], &b"value01"[..]))
        .await
        .unwrap();
    builder.add(&Entry::delete(&b"key02"[..])).await.unwrap();
    builder
        .add(&Entry::put(&b"key03"[..], &b"value03"[..]))
        .await
        .unwrap();
    builder.add(&Entry::delete(&b"key04"[..])).await.unwrap();
    builder
        .add(&Entry::put(&b"key05"[..], &b"value05"[..]))
        .await
        .unwrap();

    let metadata = builder.finish().await.unwrap();

    assert_eq!(metadata.entry_count, 5);
}

#[tokio::test]
async fn test_sstable_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 0,
        ..Default::default()
    };

    let builder = SSTableBuilder::new(config).await.unwrap();
    let metadata = builder.finish().await.unwrap();

    assert_eq!(metadata.entry_count, 0);
    assert_eq!(metadata.block_count, 0);

    // File should still exist with minimal structure (index + bloom + footer)
    let file_exists = fs::metadata(&path).await.is_ok();
    assert!(file_exists);
}

#[tokio::test]
async fn test_sstable_single_entry() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("single.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 1,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();
    builder
        .add(&Entry::put(&b"only_key"[..], &b"only_value"[..]))
        .await
        .unwrap();

    let metadata = builder.finish().await.unwrap();

    assert_eq!(metadata.entry_count, 1);
    assert_eq!(metadata.block_count, 1);
}

#[tokio::test]
async fn test_sstable_file_format_verification() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("format.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 10,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..10 {
        let key = format!("k{:02}", i);
        let value = format!("v{:02}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read file and verify footer
    let file_data = fs::read(&path).await.unwrap();

    // Footer is last 64 bytes
    assert!(file_data.len() >= 64);
    let footer_bytes: [u8; 64] = file_data[file_data.len() - 64..].try_into().unwrap();

    // Decode footer
    let footer = Footer::decode(&footer_bytes).unwrap();

    assert_eq!(footer.entry_count, 10);
    assert!(footer.index_offset > 0);
    assert!(footer.index_size > 0);
    assert!(footer.bloom_offset > 0);
    assert!(footer.bloom_size > 0);
    assert_eq!(footer.compression, Compression::None);
}

#[tokio::test]
async fn test_sstable_index_structure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        block_size: 512, // Small blocks to force multiple
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..100 {
        let key = format!("key{:04}", i);
        let value = "x".repeat(50); // 50 bytes per value
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read and verify index
    let file_data = fs::read(&path).await.unwrap();
    let footer_bytes: [u8; 64] = file_data[file_data.len() - 64..].try_into().unwrap();
    let footer = Footer::decode(&footer_bytes).unwrap();

    // Extract index
    let index_start = footer.index_offset as usize;
    let index_end = index_start + footer.index_size as usize;
    let index_data = &file_data[index_start..index_end];

    let index = Index::decode(index_data).unwrap();

    // Should have multiple blocks
    assert!(
        index.len() > 1,
        "Should have multiple blocks due to small block size"
    );

    // Verify index entries are sorted
    let entries: Vec<_> = index.iter().collect();
    for i in 1..entries.len() {
        assert!(
            entries[i - 1].first_key < entries[i].first_key,
            "Index entries must be sorted"
        );
    }
}

#[tokio::test]
async fn test_sstable_bloom_filter_structure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bloom.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 100,
        bloom_bits_per_key: 10,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..100 {
        let key = format!("key{:04}", i);
        let value = format!("value{:04}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read and verify bloom filter
    let file_data = fs::read(&path).await.unwrap();
    let footer_bytes: [u8; 64] = file_data[file_data.len() - 64..].try_into().unwrap();
    let footer = Footer::decode(&footer_bytes).unwrap();

    // Extract bloom filter
    let bloom_start = footer.bloom_offset as usize;
    let bloom_end = bloom_start + footer.bloom_size as usize;
    let bloom_data = &file_data[bloom_start..bloom_end];

    let bloom = BloomFilter::decode(bloom_data).unwrap();

    // Verify bloom contains the keys we added
    assert!(bloom.contains(b"key0000"));
    assert!(bloom.contains(b"key0050"));
    assert!(bloom.contains(b"key0099"));

    // Keys not added should mostly not be found
    assert!(!bloom.contains(b"key9999"));
}

#[tokio::test]
async fn test_sstable_block_structure() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("blocks.sst");

    let config = SSTableConfig {
        path: path.clone(),
        estimated_entries: 10,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await.unwrap();

    for i in 0..10 {
        let key = format!("k{:02}", i);
        let value = format!("v{:02}", i);
        builder.add(&Entry::put(key, value)).await.unwrap();
    }

    builder.finish().await.unwrap();

    // Read file and extract first block
    let file_data = fs::read(&path).await.unwrap();
    let footer_bytes: [u8; 64] = file_data[file_data.len() - 64..].try_into().unwrap();
    let footer = Footer::decode(&footer_bytes).unwrap();

    // Extract index to find first block
    let index_start = footer.index_offset as usize;
    let index_end = index_start + footer.index_size as usize;
    let index = Index::decode(&file_data[index_start..index_end]).unwrap();

    let first_entry = index.get(0).unwrap();
    let block_start = first_entry.block_offset as usize;
    let block_end = block_start + first_entry.block_size as usize;
    let block_data = &file_data[block_start..block_end];

    // Decode block
    let block = Block::decode(bytes::Bytes::copy_from_slice(block_data)).unwrap();

    // Iterate and verify entries
    let mut iter = block.iter();
    let mut count = 0;
    while let Some(entry) = iter.try_next().unwrap() {
        assert!(entry.key.starts_with(b"k"));
        assert!(entry.value.starts_with(b"v"));
        count += 1;
    }

    assert!(count > 0, "Block should contain entries");
}

#[tokio::test]
async fn test_sstable_multiple_files() {
    let dir = TempDir::new().unwrap();

    // Create multiple SSTables
    for table_num in 0..5 {
        let path = dir.path().join(format!("table_{}.sst", table_num));

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 20,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();

        // Each table has non-overlapping key ranges
        let start = table_num * 100;
        let end = start + 20;

        for i in start..end {
            let key = format!("key{:06}", i);
            let value = format!("value{:06}", i);
            builder.add(&Entry::put(key, value)).await.unwrap();
        }

        let metadata = builder.finish().await.unwrap();
        assert_eq!(metadata.entry_count, 20);
    }

    // Verify all 5 files exist
    for table_num in 0..5 {
        let path = dir.path().join(format!("table_{}.sst", table_num));
        assert!(fs::metadata(&path).await.is_ok());
    }
}

#[tokio::test]
async fn test_sstable_overwrite_existing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("overwrite.sst");

    // Create first SSTable
    {
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 5,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..5 {
            builder
                .add(&Entry::put(format!("old{}", i), format!("old{}", i)))
                .await
                .unwrap();
        }
        builder.finish().await.unwrap();
    }

    let old_size = fs::metadata(&path).await.unwrap().len();

    // Overwrite with new SSTable
    {
        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 3,
            ..Default::default()
        };

        let mut builder = SSTableBuilder::new(config).await.unwrap();
        for i in 0..3 {
            builder
                .add(&Entry::put(format!("new{}", i), format!("new{}", i)))
                .await
                .unwrap();
        }
        builder.finish().await.unwrap();
    }

    let new_size = fs::metadata(&path).await.unwrap().len();

    // New file should be smaller (fewer entries)
    assert!(new_size < old_size);
}
