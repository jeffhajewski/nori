---
layout: default
title: Key-Value Store
parent: Recipes
nav_order: 1
---

# Building a Key-Value Store
{: .no_toc }

Complete implementation of a durable key-value store using nori-wal.
{: .fs-6 .fw-300 }

## Table of contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Problem

You want to build a simple key-value store where:
- Data survives crashes and restarts
- Fast in-memory reads
- Durable writes
- Support for PUT and DELETE operations

## Solution

```rust
use nori_wal::{Wal, WalConfig, Record, Position};
use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;
use bytes::Bytes;

/// A simple key-value store with WAL durability.
pub struct KvStore {
    /// In-memory data structure for fast reads
    data: HashMap<Bytes, Bytes>,
    /// Write-ahead log for durability
    wal: Wal,
}

impl KvStore {
    /// Opens or creates a KV store at the given path.
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        // Configure WAL
        let config = WalConfig {
            dir: path.join("wal"),
            max_segment_size: 128 * 1024 * 1024,  // 128 MB segments
            fsync_policy: nori_wal::FsyncPolicy::Batch(
                std::time::Duration::from_millis(5)
            ),
            preallocate: true,
            node_id: 0,
        };

        // Open WAL (performs recovery automatically)
        let (wal, recovery_info) = Wal::open(config).await?;

        println!("Recovery complete:");
        println!("  Valid records: {}", recovery_info.valid_records);
        println!("  Segments scanned: {}", recovery_info.segments_scanned);

        if recovery_info.corruption_detected {
            println!("  WARNING: {} bytes truncated due to corruption",
                recovery_info.bytes_truncated);
        }

        // Rebuild in-memory state from WAL
        let mut data = HashMap::new();
        let mut reader = wal.read_from(Position { segment_id: 0, offset: 0 }).await?;

        while let Some((record, _position)) = reader.next_record().await? {
            if record.tombstone {
                // DELETE operation
                data.remove(&record.key);
            } else {
                // PUT operation
                data.insert(record.key, record.value);
            }
        }

        println!("Rebuilt {} keys from WAL", data.len());

        Ok(Self { data, wal })
    }

    /// Gets a value by key.
    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.data.get(key).map(|v| v.as_ref())
    }

    /// Puts a key-value pair.
    pub async fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
        // 1. Write to WAL first (write-ahead!)
        let record = Record::put(key, value);
        self.wal.append(&record).await?;

        // 2. Update in-memory data
        self.data.insert(Bytes::copy_from_slice(key), Bytes::copy_from_slice(value));

        Ok(())
    }

    /// Deletes a key.
    pub async fn delete(&mut self, key: &[u8]) -> Result<()> {
        // 1. Write tombstone to WAL
        let record = Record::delete(key);
        self.wal.append(&record).await?;

        // 2. Remove from in-memory data
        self.data.remove(key);

        Ok(())
    }

    /// Syncs WAL to disk.
    pub async fn sync(&self) -> Result<()> {
        self.wal.sync().await?;
        Ok(())
    }

    /// Returns the number of keys in the store.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns an iterator over all keys.
    pub fn keys(&self) -> impl Iterator<Item = &[u8]> {
        self.data.keys().map(|k| k.as_ref())
    }

    /// Gracefully closes the store.
    pub async fn close(self) -> Result<()> {
        self.wal.close().await?;
        Ok(())
    }
}

// Example usage
#[tokio::main]
async fn main() -> Result<()> {
    // Open store (creates if doesn't exist)
    let mut store = KvStore::open("./my_kv_store").await?;

    // Write some data
    store.put(b"user:1", b"Alice").await?;
    store.put(b"user:2", b"Bob").await?;
    store.put(b"user:3", b"Charlie").await?;

    // Read data
    if let Some(value) = store.get(b"user:1") {
        println!("user:1 = {:?}", std::str::from_utf8(value)?);
    }

    // Delete a key
    store.delete(b"user:2").await?;

    // Sync to ensure durability
    store.sync().await?;

    println!("Store has {} keys", store.len());

    // Close gracefully
    store.close().await?;

    Ok(())
}
```

## How It Works

### 1. Initialization

```rust
let (wal, recovery_info) = Wal::open(config).await?;
```

The WAL is opened and automatically performs recovery:
- Scans all segment files
- Validates CRC for each record
- Truncates any corruption
- Returns recovery statistics

### 2. State Reconstruction

```rust
let mut reader = wal.read_from(Position::start()).await?;

while let Some((record, _)) = reader.next_record().await? {
    if record.tombstone {
        data.remove(&record.key);
    } else {
        data.insert(record.key, record.value);
    }
}
```

We replay the entire WAL to rebuild the in-memory HashMap:
- PUT records insert/update keys
- DELETE records (tombstones) remove keys
- Last write wins for duplicate keys

### 3. Write Path

```rust
pub async fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
    // 1. WAL first
    let record = Record::put(key, value);
    self.wal.append(&record).await?;

    // 2. Then in-memory
    self.data.insert(...);

    Ok(())
}
```

**Critical:** WAL write happens before in-memory update. If we crash after WAL write but before in-memory update, recovery will replay the WAL and the data will be correct.

### 4. Read Path

```rust
pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
    self.data.get(key).map(|v| v.as_ref())
}
```

Reads are pure in-memory lookups (fast!). The WAL is only for durability, not for reads.

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_basic_operations() {
        let dir = TempDir::new().unwrap();
        let mut store = KvStore::open(dir.path()).await.unwrap();

        // PUT
        store.put(b"key1", b"value1").await.unwrap();
        assert_eq!(store.get(b"key1"), Some(&b"value1"[..]));

        // UPDATE
        store.put(b"key1", b"value2").await.unwrap();
        assert_eq!(store.get(b"key1"), Some(&b"value2"[..]));

        // DELETE
        store.delete(b"key1").await.unwrap();
        assert_eq!(store.get(b"key1"), None);
    }

    #[tokio::test]
    async fn test_recovery() {
        let dir = TempDir::new().unwrap();

        // Write data
        {
            let mut store = KvStore::open(dir.path()).await.unwrap();
            store.put(b"key1", b"value1").await.unwrap();
            store.put(b"key2", b"value2").await.unwrap();
            store.sync().await.unwrap();
            // Drop without close (simulates crash)
        }

        // Reopen and verify data
        {
            let store = KvStore::open(dir.path()).await.unwrap();
            assert_eq!(store.get(b"key1"), Some(&b"value1"[..]));
            assert_eq!(store.get(b"key2"), Some(&b"value2"[..]));
            assert_eq!(store.len(), 2);
        }
    }

    #[tokio::test]
    async fn test_delete_recovery() {
        let dir = TempDir::new().unwrap();

        {
            let mut store = KvStore::open(dir.path()).await.unwrap();
            store.put(b"key1", b"value1").await.unwrap();
            store.delete(b"key1").await.unwrap();
            store.sync().await.unwrap();
        }

        {
            let store = KvStore::open(dir.path()).await.unwrap();
            assert_eq!(store.get(b"key1"), None);
            assert_eq!(store.len(), 0);
        }
    }
}
```

## Production Considerations

### 1. Compaction

Over time, the WAL will grow with duplicate keys and tombstones:

```
WAL: [put(k1,v1), put(k2,v2), put(k1,v3), delete(k2)]
     ↓
In-memory: {k1: v3}
```

The WAL has 4 records, but only 1 key in memory. You need periodic compaction:

```rust
impl KvStore {
    /// Compacts the WAL by rewriting only current state.
    pub async fn compact(&mut self) -> Result<()> {
        // 1. Create new WAL directory
        let new_dir = self.wal.config().dir.parent().unwrap().join("wal_new");
        let config = WalConfig {
            dir: new_dir.clone(),
            ..self.wal.config().clone()
        };

        let (new_wal, _) = Wal::open(config).await?;

        // 2. Write current state to new WAL
        for (key, value) in &self.data {
            let record = Record::put(key, value);
            new_wal.append(&record).await?;
        }
        new_wal.sync().await?;

        // 3. Swap WALs
        let old_dir = self.wal.config().dir.clone();
        self.wal = new_wal;

        // 4. Delete old WAL
        tokio::fs::remove_dir_all(&old_dir).await?;
        tokio::fs::rename(&new_dir, &old_dir).await?;

        Ok(())
    }
}
```

Run compaction periodically or when WAL size exceeds threshold.

### 2. Memory Management

HashMap can grow large. Consider:

```rust
// Limit max size
const MAX_KEYS: usize = 1_000_000;

pub async fn put(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
    if self.data.len() >= MAX_KEYS && !self.data.contains_key(key) {
        return Err(anyhow::anyhow!("Store is full"));
    }
    // ...
}
```

Or use an eviction policy (LRU, LFU).

### 3. Batching for Performance

Batch multiple operations before syncing:

```rust
// Batch write
for (key, value) in batch {
    store.put(key, value).await?;
}
store.sync().await?;  // Single fsync for all
```

This is much faster than syncing after each operation.

### 4. Monitoring

Track key metrics:

```rust
// Keys in memory
metrics.gauge("kv.keys", store.len());

// WAL segments
metrics.gauge("kv.wal_segments", count_segments()?);

// WAL size
metrics.gauge("kv.wal_bytes", total_wal_size()?);
```

Alert when:
- WAL grows too large (need compaction)
- Recovery takes too long
- Corruption detected

### 5. Concurrent Access

Current implementation is single-threaded. For concurrent access:

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ConcurrentKvStore {
    data: Arc<RwLock<HashMap<Bytes, Bytes>>>,
    wal: Arc<Mutex<Wal>>,
}

impl ConcurrentKvStore {
    pub async fn get(&self, key: &[u8]) -> Option<Bytes> {
        let data = self.data.read().await;
        data.get(key).cloned()
    }

    pub async fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let record = Record::put(key, value);

        // WAL write (exclusive)
        let mut wal = self.wal.lock().await;
        wal.append(&record).await?;
        drop(wal);

        // In-memory update (write lock)
        let mut data = self.data.write().await;
        data.insert(Bytes::copy_from_slice(key), Bytes::copy_from_slice(value));

        Ok(())
    }
}
```

## Enhancements

### Add TTL Support

```rust
use std::time::{SystemTime, Duration};

struct TtlEntry {
    value: Bytes,
    expires_at: SystemTime,
}

pub async fn put_with_ttl(
    &mut self,
    key: &[u8],
    value: &[u8],
    ttl: Duration
) -> Result<()> {
    let record = Record::put_with_ttl(key, value, ttl);
    self.wal.append(&record).await?;

    let expires_at = SystemTime::now() + ttl;
    self.data.insert(key, TtlEntry { value, expires_at });

    Ok(())
}
```

### Add Range Queries

Use `BTreeMap` instead of `HashMap`:

```rust
use std::collections::BTreeMap;

pub fn range(&self, start: &[u8], end: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    self.data.range(start..end)
        .map(|(k, v)| (k.as_ref(), v.as_ref()))
}
```

### Add Compression

```rust
use nori_wal::Compression;

pub async fn put_compressed(&mut self, key: &[u8], value: &[u8]) -> Result<()> {
    let record = Record::put(key, value)
        .with_compression(Compression::Lz4);

    self.wal.append(&record).await?;
    self.data.insert(key, value);

    Ok(())
}
```

## Conclusion

This recipe demonstrates:
- Using WAL for durability
- Rebuilding state from WAL on recovery
- Write-ahead logging pattern
- Testing crash recovery

The KV store is production-ready for single-node use cases. For distributed systems, add replication using the [Replication recipe](replication).
