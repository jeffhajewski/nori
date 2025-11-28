# Recipes

Common patterns and practical examples for using nori-sstable.

---

## Overview

This section provides practical, copy-paste-ready examples for common SSTable use cases.

## Recipes

### [Basic Usage](basic-usage.md)
Building and reading your first SSTable with complete error handling.

### [Hot Workloads](hot-workloads.md)
Optimizing for high-throughput, cache-friendly workloads.

### [Cold Storage](cold-storage.md)
Configuring SSTables for archival data with maximum compression.

### [Iterator Patterns](iterator-patterns.md)
Efficient range scans, filtering, and merge patterns.

### [Batch Building](batch-building.md)
Building multiple SSTables efficiently from large datasets.

### [Integration with LSM](lsm-integration.md)
Using nori-sstable as part of an LSM-tree storage engine.

---

## Quick Examples

### Minimal Example
```rust
let mut builder = SSTableBuilder::new(config).await?;
builder.add(&Entry::put("key", "value")).await?;
builder.finish().await?;
```

### With Compression
```rust
let config = SSTableConfig {
    compression: Compression::Lz4,
    ..Default::default()
};
```

### With Custom Cache
```rust
let reader = SSTableReader::open_with_config(
    path,
    Arc::new(NoopMeter),
    256  // 256MB cache
).await?;
```
