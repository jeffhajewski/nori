---
layout: default
title: Recipes
nav_order: 6
has_children: true
parent: nori-sstable
grand_parent: Crates
---

# Recipes
{: .no_toc }

Common patterns and practical examples for using nori-sstable.
{: .fs-6 .fw-300 }

---

## Overview

This section provides practical, copy-paste-ready examples for common SSTable use cases.

## Recipes

### [Basic Usage](basic-usage)
Building and reading your first SSTable with complete error handling.

### [Hot Workloads](hot-workloads)
Optimizing for high-throughput, cache-friendly workloads.

### [Cold Storage](cold-storage)
Configuring SSTables for archival data with maximum compression.

### [Iterator Patterns](iterator-patterns)
Efficient range scans, filtering, and merge patterns.

### [Batch Building](batch-building)
Building multiple SSTables efficiently from large datasets.

### [Integration with LSM](lsm-integration)
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
