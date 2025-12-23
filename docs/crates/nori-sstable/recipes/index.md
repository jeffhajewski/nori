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
