---
layout: default
title: Tuning Guide
parent: Performance
grand_parent: nori-sstable
nav_order: 2
---

# Tuning Guide
{: .no_toc }

Optimize nori-sstable for your specific workload.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Hot Workloads (High Read Rate)

**Goal:** Minimize p95/p99 read latency

**Configuration:**
```rust
SSTableConfig {
    compression: Compression::Lz4,  // Fast decompression
    block_cache_mb: 512,             // Large cache
    bloom_bits_per_key: 12,          // Lower FP rate
    ..Default::default()
}
```

**Rationale:**
- LZ4 decompresses in <1µs vs Zstd 3µs
- 512MB cache = high hit rate
- 12 bits/key = 0.3% FP vs 0.9%

**Expected:**
- p95: <1µs (cache hits)
- p99: ~100µs (cache misses)
- Hit rate: 90%+

---

## Cold Storage (Infrequent Access)

**Goal:** Maximize compression ratio

**Configuration:**
```rust
SSTableConfig {
    compression: Compression::Zstd,  // Best ratio
    block_cache_mb: 0,               // Disable cache
    bloom_bits_per_key: 10,          // Standard
    block_size: 8192,                // Larger blocks
    ..Default::default()
}
```

**Rationale:**
- Zstd: 3-5x compression
- No cache: Save RAM
- 8KB blocks: Better compression

**Savings:**
- 1TB → 250GB (75% savings)

---

## Write-Heavy Workloads

**Goal:** Maximize write throughput

**Configuration:**
```rust
SSTableConfig {
    compression: Compression::None,  // No compression overhead
    block_cache_mb: 64,              // Moderate cache
    bloom_bits_per_key: 10,
    ..Default::default()
}
```

**Or:**
```rust
SSTableConfig {
    compression: Compression::Lz4,  // Minimal overhead
    ..Default::default()
}
```

**Expected:**
- No compression: 700K entries/sec
- LZ4: 560K entries/sec (-20%)

---

## Memory-Constrained

**Goal:** Minimize memory usage

**Configuration:**
```rust
SSTableConfig {
    compression: Compression::Zstd,
    block_cache_mb: 16,             // Small cache
    bloom_bits_per_key: 8,          // Smaller bloom
    ..Default::default()
}
```

**Memory breakdown:**
```
100K entry SSTable:
  Bloom (8 bits/key): 100KB
  Index: ~200KB
  Cache (16MB): 16MB
  Total: ~16.3MB
```

---

## Range-Scan Heavy

**Goal:** Optimize sequential reads

**Configuration:**
```rust
SSTableConfig {
    block_size: 16384,              // Larger blocks
    compression: Compression::Lz4,
    block_cache_mb: 128,
    ..Default::default()
}
```

**Rationale:**
- Larger blocks = fewer block boundaries
- Better sequential throughput
- LZ4: Fast decompression for scans

---

## Monitoring & Adjustment

### Key Metrics

```rust
// Cache hit rate
let hit_rate = cache_hits / total_reads;
if hit_rate < 0.7 {
    // Increase block_cache_mb
}

// False positive rate
let fp_rate = false_positives / bloom_checks;
if fp_rate > 0.02 {
    // Increase bloom_bits_per_key
}

// Compression ratio
let ratio = uncompressed_size / compressed_size;
if ratio < 1.5 {
    // Consider Compression::None
}
```

---

## Quick Decision Table

| Workload | Compression | Cache | Bloom | Block Size |
|----------|-------------|-------|-------|------------|
| **Hot reads** | LZ4 | 256-512MB | 12 bits | 4KB |
| **Cold storage** | Zstd | 0MB | 10 bits | 8KB |
| **Write-heavy** | None/LZ4 | 64MB | 10 bits | 4KB |
| **Memory-limited** | Zstd | 16MB | 8 bits | 4KB |
| **Range scans** | LZ4 | 128MB | 10 bits | 16KB |
| **Development** | None | 64MB | 10 bits | 4KB |
