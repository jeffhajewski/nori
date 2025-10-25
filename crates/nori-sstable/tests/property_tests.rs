use nori_sstable::{Entry, SSTableBuilder, SSTableConfig, SSTableReader};
use proptest::prelude::*;
use std::collections::BTreeMap;
use std::sync::Arc;
use tempfile::TempDir;

// Strategy: Generate arbitrary key/value pairs
fn arb_key() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 1..100)
}

fn arb_value() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..1000)
}

fn arb_entry() -> impl Strategy<Value = (Vec<u8>, Option<Vec<u8>>)> {
    (arb_key(), prop::option::of(arb_value()))
}

fn arb_entries() -> impl Strategy<Value = Vec<(Vec<u8>, Option<Vec<u8>>)>> {
    prop::collection::vec(arb_entry(), 1..100)
}

#[test]
fn test_property_roundtrip() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(mut entries in arb_entries())| {
        rt.block_on(async {
            // Sort entries by key (SSTable requirement)
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries.dedup_by(|a, b| a.0 == b.0); // Remove duplicates

            if entries.is_empty() {
                return Ok(());
            }

            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            // Build SSTable
            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: entries.len(),
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();

            for (key, value) in &entries {
                let entry = match value {
                    Some(v) => Entry::put(key.clone(), v.clone()),
                    None => Entry::delete(key.clone()),
                };
                builder.add(&entry).await.unwrap();
            }

            builder.finish().await.unwrap();

            // Read back
            let reader = Arc::new(SSTableReader::open(path).await.unwrap());

            // Verify each entry
            for (key, value) in &entries {
                let found = reader.get(key).await.unwrap();

                match value {
                    Some(v) => {
                        prop_assert!(found.is_some(), "Key {:?} not found", key);
                        let entry = found.unwrap();
                        prop_assert!(!entry.tombstone, "Expected value, got tombstone");
                        prop_assert_eq!(&entry.value[..], &v[..]);
                    }
                    None => {
                        prop_assert!(found.is_some(), "Tombstone for key {:?} not found", key);
                        let entry = found.unwrap();
                        prop_assert!(entry.tombstone, "Expected tombstone, got value");
                    }
                }
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_sorted_order() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(mut entries in arb_entries())| {
        rt.block_on(async {
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            entries.dedup_by(|a, b| a.0 == b.0);

            if entries.is_empty() {
                return Ok(());
            }

            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: entries.len(),
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();

            for (key, value) in &entries {
                let entry = match value {
                    Some(v) => Entry::put(key.clone(), v.clone()),
                    None => Entry::delete(key.clone()),
                };
                builder.add(&entry).await.unwrap();
            }

            builder.finish().await.unwrap();

            // Iterate and verify sorted order
            let reader = Arc::new(SSTableReader::open(path).await.unwrap());
            let mut iter = reader.iter();

            let mut last_key: Option<Vec<u8>> = None;

            while let Some(entry) = iter.try_next().await.unwrap() {
                if let Some(ref prev) = last_key {
                    prop_assert!(entry.key.as_ref() > prev.as_slice(),
                        "Keys not in sorted order: {:?} > {:?}", prev, entry.key);
                }
                last_key = Some(entry.key.to_vec());
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_tombstone_preservation() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(keys in prop::collection::vec(arb_key(), 1..50))| {
        rt.block_on(async {
            let mut unique_keys: Vec<_> = keys.into_iter().collect();
            unique_keys.sort();
            unique_keys.dedup();

            if unique_keys.is_empty() {
                return Ok(());
            }

            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: unique_keys.len(),
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();

            // Add all keys as tombstones
            for key in &unique_keys {
                builder.add(&Entry::delete(key.clone())).await.unwrap();
            }

            builder.finish().await.unwrap();

            // Verify all are tombstones
            let reader = Arc::new(SSTableReader::open(path).await.unwrap());

            for key in &unique_keys {
                let entry = reader.get(key).await.unwrap();
                prop_assert!(entry.is_some(), "Tombstone not found for key {:?}", key);
                prop_assert!(entry.unwrap().tombstone, "Expected tombstone for key {:?}", key);
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_range_scan_correctness() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(
        entries in prop::collection::vec((arb_key(), arb_value()), 10..50),
        range_start in 0usize..25,
        range_len in 1usize..20,
    )| {
        rt.block_on(async {
            // Build sorted map
            let mut map: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
            for (k, v) in entries {
                map.insert(k, v);
            }

            let all_keys: Vec<_> = map.keys().cloned().collect();

            if all_keys.len() < 2 {
                return Ok(());
            }

            let start_idx = range_start.min(all_keys.len() - 1);
            let end_idx = (start_idx + range_len).min(all_keys.len());

            let start_key = &all_keys[start_idx];
            let end_key = &all_keys[end_idx - 1];

            // Build SSTable
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: map.len(),
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();

            for (k, v) in &map {
                builder.add(&Entry::put(k.clone(), v.clone())).await.unwrap();
            }

            builder.finish().await.unwrap();

            // Range scan
            let reader = Arc::new(SSTableReader::open(path).await.unwrap());
            let mut iter = reader.iter_range(start_key.clone().into(), end_key.clone().into());

            let mut scanned = Vec::new();
            while let Some(entry) = iter.try_next().await.unwrap() {
                scanned.push(entry.key.to_vec());
            }

            // Verify range
            for key in &scanned {
                prop_assert!(key >= start_key, "Key {:?} < start {:?}", key, start_key);
                prop_assert!(key < end_key, "Key {:?} >= end {:?}", key, end_key);
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_bloom_filter_soundness() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(entries in prop::collection::vec(arb_key(), 10..100))| {
        rt.block_on(async {
            let mut unique_keys: Vec<_> = entries.into_iter().collect();
            unique_keys.sort();
            unique_keys.dedup();

            if unique_keys.is_empty() {
                return Ok(());
            }

            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: unique_keys.len(),
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();

            for key in &unique_keys {
                builder.add(&Entry::put(key.clone(), "value")).await.unwrap();
            }

            builder.finish().await.unwrap();

            let reader = Arc::new(SSTableReader::open(path).await.unwrap());

            // Bloom filter must not have false negatives
            for key in &unique_keys {
                let result = reader.get(key).await.unwrap();
                prop_assert!(result.is_some(),
                    "Bloom filter false negative for key {:?}", key);
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_block_boundary_handling() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(num_entries in 50usize..200)| {
        rt.block_on(async {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("test.sst");

            // Use small block size to force many blocks
            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: num_entries,
                block_size: 256, // Small to create boundaries
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

            // Verify all keys are findable (including those at block boundaries)
            for i in 0..num_entries {
                let key = format!("key_{:08}", i);
                let entry = reader.get(key.as_bytes()).await.unwrap();
                prop_assert!(entry.is_some(), "Key not found: {}", key);
            }

            Ok(())
        })?;
    });
}

#[test]
fn test_property_empty_edge_cases() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        // Empty SSTable
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.sst");

        let config = SSTableConfig {
            path: path.clone(),
            estimated_entries: 0,
            ..Default::default()
        };

        let builder = SSTableBuilder::new(config).await.unwrap();
        builder.finish().await.unwrap();

        let reader = Arc::new(SSTableReader::open(path).await.unwrap());

        // Any lookup should return None
        assert!(reader.get(b"any_key").await.unwrap().is_none());

        // Iterator should be empty
        let mut iter = reader.iter();
        assert!(iter.try_next().await.unwrap().is_none());
    });
}

#[test]
fn test_property_single_entry() {
    let rt = tokio::runtime::Runtime::new().unwrap();

    proptest!(|(key in arb_key(), value in arb_value())| {
        rt.block_on(async {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("single.sst");

            let config = SSTableConfig {
                path: path.clone(),
                estimated_entries: 1,
                ..Default::default()
            };

            let mut builder = SSTableBuilder::new(config).await.unwrap();
            builder.add(&Entry::put(key.clone(), value.clone())).await.unwrap();
            builder.finish().await.unwrap();

            let reader = Arc::new(SSTableReader::open(path).await.unwrap());

            // Verify entry
            let entry = reader.get(&key).await.unwrap();
            prop_assert!(entry.is_some());
            prop_assert_eq!(&entry.unwrap().value[..], &value[..]);

            // Verify iterator
            let mut iter = reader.iter();
            let first = iter.try_next().await.unwrap();
            prop_assert!(first.is_some());
            prop_assert_eq!(&first.unwrap().key[..], &key[..]);

            let second = iter.try_next().await.unwrap();
            prop_assert!(second.is_none());

            Ok(())
        })?;
    });
}
