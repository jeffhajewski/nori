# Compression Strategy

Why nori-sstable compresses at block granularity with LZ4/Zstd.

---

## Decision

1. **Compress at block level** (not file or entry level)
2. **Default to LZ4** for general use
3. **Offer Zstd** for cold storage
4. **Cache decompressed blocks** in RAM

---

## Why Block-Level Compression?

### File-Level: Too Coarse
```
Compressed file (1GB → 400MB)
To read 1 key: Decompress entire 1GB 
```

### Entry-Level: Too Fine
```
Each entry compressed individually
Poor ratio (no cross-entry patterns) 
High overhead (header per entry) 
```

### Block-Level: Just Right
```
4KB block (~50 entries) → compress together
Good ratio (patterns across entries) Yes
Fast decompression (1µs for 4KB) Yes
```

---

## Why LZ4 as Default?

**Speed is critical for hot data:**

```
LZ4:
  Compress: 750 MB/s
  Decompress: 3,900 MB/s (3.9 GB/s!)
  Ratio: 2-3x

Zstd:
  Compress: 450 MB/s
  Decompress: 1,200 MB/s
  Ratio: 3-5x
```

**Key insight:** LZ4 decompresses so fast (1µs per 4KB) that it's essentially free compared to cache lookup (100ns).

**Math:**
```
Cache hit: 100ns
LZ4 decompress: 1,000ns (1µs)
Disk read: 100,000ns (100µs)

Decompress cost: 1% of disk read
```

---

## When to Use Zstd?

**Cold storage where ratio > speed:**

```rust
SSTableConfig {
    compression: Compression::Zstd,
    block_cache_mb: 0, // Disable cache
    ..Default::default()
}
```

**Use cases:**
- Archival data (accessed < 1/day)
- Backup snapshots
- Log retention
- Cost-sensitive storage (S3 glacier)

---

## Cache Decompressed Blocks

**Critical design:**

```
Disk: Compressed (1.6KB LZ4)
  ↓ read
RAM: Decompressed (4KB in cache)
  ↓ many reads
Users get cached data at 100ns
```

**Why decompressed cache:**
- Decompress once, serve many times
- 80% cache hit rate typical
- LZ4 fast enough for 20% misses

**Alternative (compressed cache):**
- Save memory (2.5x less cache size)
- But decompress on every cache hit! 
- 1µs * millions of requests = seconds wasted

---

## Compression Ratio vs Speed

| Algorithm | Ratio | Decompress | Use Case |
|-----------|-------|------------|----------|
| **None** | 1.0x | Instant | Development, pre-compressed data |
| **LZ4** | 2-3x | 3.9 GB/s | **Default (hot data)** |
| **LZ4HC** | 2.5-3.5x | 3.9 GB/s | Willing to trade write speed |
| **Zstd L3** | 3-4x | 1.2 GB/s | Balanced |
| **Zstd L9** | 4-5x | 1.2 GB/s | **Cold storage** |
| **Zstd L19** | 4.5-5.5x | 1.2 GB/s | Archival (slow writes OK) |

---

## Real-World Performance

### Test Data: JSON User Records

```
4KB block, 34 entries:
  {"user":"alice","age":25,"city":"NY"}
  {"user":"bob","age":30,"city":"SF"}
  ...

Uncompressed: 4096 bytes

LZ4:       1420 bytes (2.9x) - 1.0µs decompress
Zstd (L3): 1100 bytes (3.7x) - 3.3µs decompress
Zstd (L9): 950 bytes (4.3x) - 3.3µs decompress
```

**Decision:** LZ4's 2.9x with 1µs beats Zstd's 4.3x with 3.3µs for hot workloads.

---

## Summary

 Block-level compression (4KB granularity)
 LZ4 default (speed matters for hot data)
 Zstd for cold (ratio matters, speed doesn't)
 Cache decompressed (decompress once, serve many)

**Key trade-off:** LZ4 gives 70% of Zstd's compression at 3x the speed.
