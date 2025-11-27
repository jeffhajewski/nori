# Basic Usage

Complete examples for building and reading SSTables.

---

## Minimal Example

```rust
use nori_sstable::{Entry, SSTableBuilder, SSTableReader, SSTableConfig};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Build
    let config = SSTableConfig {
        path: PathBuf::from("data.sst"),
        estimated_entries: 100,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;
    builder.add(&Entry::put("alice", "engineer")).await?;
    builder.add(&Entry::put("bob", "designer")).await?;
    builder.add(&Entry::put("carol", "manager")).await?;
    builder.finish().await?;

    // Read
    let reader = Arc::new(SSTableReader::open(PathBuf::from("data.sst")).await?);
    if let Some(entry) = reader.get(b"bob").await? {
        println!("bob is a {}", String::from_utf8_lossy(&entry.value));
    }

    Ok(())
}
```

---

## Building with Sorted Data

```rust
use std::collections::BTreeMap;

let mut data = BTreeMap::new();
data.insert(b"user:1".to_vec(), b"alice".to_vec());
data.insert(b"user:2".to_vec(), b"bob".to_vec());
data.insert(b"user:3".to_vec(), b"carol".to_vec());

let mut builder = SSTableBuilder::new(config).await?;
for (key, value) in data {
    builder.add(&Entry::put(key, value)).await?;
}
let metadata = builder.finish().await?;

println!("Built SSTable:");
println!("  Entries: {}", metadata.entry_count);
println!("  Size: {} bytes", metadata.file_size);
println!("  Blocks: {}", metadata.block_count);
```

---

## Reading with Error Handling

```rust
match reader.get(b"nonexistent").await {
    Ok(Some(entry)) => {
        if entry.tombstone {
            println!("Key was deleted");
        } else {
            println!("Found: {:?}", entry.value);
        }
    }
    Ok(None) => println!("Key not found"),
    Err(e) => eprintln!("Error: {}", e),
}
```

---

## Range Scanning

```rust
use futures::TryStreamExt;

let mut iter = reader.iter_range(
    Bytes::from("user:1"),
    Bytes::from("user:9")
);

while let Some(entry) = iter.try_next().await? {
    println!("{:?} = {:?}",
        String::from_utf8_lossy(&entry.key),
        String::from_utf8_lossy(&entry.value)
    );
}
```

---

## With Compression

```rust
use nori_sstable::Compression;

let config = SSTableConfig {
    path: PathBuf::from("compressed.sst"),
    estimated_entries: 1000,
    compression: Compression::Lz4,
    ..Default::default()
};

let mut builder = SSTableBuilder::new(config).await?;
// Add entries...
builder.finish().await?;

// Reader automatically detects compression
let reader = Arc::new(SSTableReader::open("compressed.sst".into()).await?);
```

---

## Custom Cache Size

```rust
use nori_observe::NoopMeter;

// 256MB cache for hot workloads
let reader = SSTableReader::open_with_config(
    PathBuf::from("data.sst"),
    Arc::new(NoopMeter),
    256  // MB
).await?;
```

---

## Batch Building

```rust
let entries = vec![
    Entry::put("key1", "value1"),
    Entry::put("key2", "value2"),
    Entry::put("key3", "value3"),
];

let mut builder = SSTableBuilder::new(config).await?;
for entry in entries {
    builder.add(&entry).await?;
}
builder.finish().await?;
```

---

## Concurrent Reads

```rust
let reader = Arc::new(SSTableReader::open("data.sst".into()).await?);

let mut handles = vec![];
for i in 0..10 {
    let r = reader.clone();
    let handle = tokio::spawn(async move {
        let key = format!("key{}", i);
        r.get(key.as_bytes()).await
    });
    handles.push(handle);
}

for handle in handles {
    let result = handle.await??;
    println!("Result: {:?}", result);
}
```
