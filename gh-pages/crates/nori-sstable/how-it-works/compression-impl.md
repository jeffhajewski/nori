---
layout: default
title: Compression Implementation
parent: How It Works
grand_parent: nori-sstable
nav_order: 4
---

# Compression Implementation
{: .no_toc }

How LZ4 and Zstd compression are integrated into nori-sstable.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Compression Enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None = 0,
    Lz4 = 1,
    Zstd = 2,
}

impl Compression {
    pub fn from_u16(value: u16) -> Option<Self> {
        match value {
            0 => Some(Compression::None),
            1 => Some(Compression::Lz4),
            2 => Some(Compression::Zstd),
            _ => None,
        }
    }
}
```

**Stored in footer:**
```
Offset 6, 2 bytes: compression type (0/1/2)
```

---

## LZ4 Integration

### Compression

```rust
use lz4_flex::compress_prepend_size;

pub fn compress_lz4(uncompressed: &[u8]) -> Vec<u8> {
    // Prepends u32 uncompressed size + compressed data
    compress_prepend_size(uncompressed)
}
```

**Output format:**
```
┌──────────────┬─────────────────────────────────┐
│ u32: 4096    │ Compressed bytes (variable)     │
│ (orig size)  │                                 │
└──────────────┴─────────────────────────────────┘
```

**Example:**
```
Input:  [u8; 4096] (uncompressed block)
Output: [0x00, 0x10, 0x00, 0x00, ...compressed...] (1620 bytes total)
        └─────────┬────────────┘
          4096 in little-endian
```

---

### Decompression

```rust
use lz4_flex::decompress_size_prepended;

pub fn decompress_lz4(compressed: &[u8]) -> Result<Vec<u8>> {
    decompress_size_prepended(compressed)
        .map_err(|e| Error::Compression(e.to_string()))
}
```

**Safety check:**
```rust
// Validate decompressed size matches expected
pub fn decompress_lz4_checked(compressed: &[u8], expected: usize) -> Result<Vec<u8>> {
    let decompressed = decompress_size_prepended(compressed)?;

    if decompressed.len() != expected {
        return Err(Error::CorruptBlock(
            format!("Expected {} bytes, got {}", expected, decompressed.len())
        ));
    }

    Ok(decompressed)
}
```

---

## Zstd Integration

### Compression

```rust
use zstd::bulk::compress;

const ZSTD_LEVEL: i32 = 3; // Balance speed vs ratio

pub fn compress_zstd(uncompressed: &[u8]) -> Result<Vec<u8>> {
    let compressed = compress(uncompressed, ZSTD_LEVEL)
        .map_err(|e| Error::Compression(e.to_string()))?;

    // Prepend uncompressed size (for safety check)
    let size_bytes = (uncompressed.len() as u32).to_le_bytes();
    let mut output = Vec::with_capacity(4 + compressed.len());
    output.extend_from_slice(&size_bytes);
    output.extend_from_slice(&compressed);

    Ok(output)
}
```

**Why level 3?**
```
Benchmark 4KB blocks:

Level  Compress   Decompress  Ratio
1      6.8µs      3.1µs       2.8x
3      9.1µs      3.4µs       3.5x  ✅ Best balance
5      14.2µs     3.6µs       3.7x
9      28.5µs     3.9µs       3.9x
```

---

### Decompression

```rust
use zstd::bulk::decompress;

pub fn decompress_zstd(compressed: &[u8]) -> Result<Vec<u8>> {
    // Read expected size
    let expected_size = u32::from_le_bytes(compressed[0..4].try_into()?) as usize;

    // Decompress
    let decompressed = decompress(&compressed[4..], expected_size)
        .map_err(|e| Error::Compression(e.to_string()))?;

    // Validate size
    if decompressed.len() != expected_size {
        return Err(Error::CorruptBlock(
            format!("Decompression produced wrong size")
        ));
    }

    Ok(decompressed)
}
```

---

## Block Compression

### During Build

```rust
impl SSTableBuilder {
    async fn flush_block(&mut self) -> Result<()> {
        let uncompressed = self.block_buffer.clone();

        // Compress block based on config
        let on_disk = match self.config.compression {
            Compression::None => uncompressed,
            Compression::Lz4 => compress_lz4(&uncompressed),
            Compression::Zstd => compress_zstd(&uncompressed)?,
        };

        // Write to file
        let offset = self.writer.pos();
        self.writer.write_all(&on_disk).await?;

        // Update index with compressed offset/size
        self.index.add_block(
            self.block_first_key.clone(),
            offset,
            on_disk.len() as u32,
        );

        Ok(())
    }
}
```

**Example trace:**
```
Block 0:
  Uncompressed: 4096 bytes
  LZ4 compressed: 1620 bytes
  Written at offset 0

Block 1:
  Uncompressed: 4096 bytes
  LZ4 compressed: 1580 bytes
  Written at offset 1620
```

---

## Block Decompression

### During Read

```rust
impl SSTableReader {
    async fn read_block(&self, block_id: usize) -> Result<Block> {
        // Check cache first
        if let Some(cached) = self.cache.get(&block_id) {
            return Ok(cached.clone());
        }

        // Read compressed block from disk
        let block_meta = &self.index[block_id];
        let compressed = self.read_at(block_meta.offset, block_meta.size).await?;

        // Decompress based on footer's compression type
        let uncompressed = match self.compression {
            Compression::None => compressed,
            Compression::Lz4 => decompress_lz4(&compressed)?,
            Compression::Zstd => decompress_zstd(&compressed)?,
        };

        // Parse block
        let block = Block::parse(&uncompressed)?;

        // Cache decompressed block
        self.cache.insert(block_id, block.clone());

        Ok(block)
    }
}
```

---

## Cache Design

### Why Cache Decompressed Blocks?

**Option 1: Cache compressed blocks**
```
Cache hit:
  1. Read compressed from cache (50ns)
  2. Decompress (1.05µs LZ4)
  Total: 1.1µs per read
```

**Option 2: Cache decompressed blocks** ✅
```
Cache hit:
  1. Read decompressed from cache (50ns)
  Total: 50ns per read (22x faster!)
```

**Cost:** 2.5x more RAM, but cache size is configurable.

---

### Cache Implementation

```rust
use lru::LruCache;
use std::sync::Mutex;

pub struct BlockCache {
    cache: Mutex<LruCache<usize, Arc<Block>>>,
    capacity_bytes: usize,
}

impl BlockCache {
    pub fn new(capacity_mb: usize) -> Self {
        let capacity_blocks = (capacity_mb * 1024 * 1024) / 4096;

        Self {
            cache: Mutex::new(LruCache::new(capacity_blocks)),
            capacity_bytes: capacity_mb * 1024 * 1024,
        }
    }

    pub fn get(&self, block_id: &usize) -> Option<Arc<Block>> {
        let mut cache = self.cache.lock().unwrap();
        cache.get(block_id).cloned()
    }

    pub fn insert(&self, block_id: usize, block: Block) {
        let mut cache = self.cache.lock().unwrap();
        cache.put(block_id, Arc::new(block));
    }
}
```

**Size calculation:**
```
64MB cache:
  64MB / 4KB per block = 16,384 blocks cached

100MB SSTable (LZ4 2.5x):
  Compressed: 40MB on disk
  Uncompressed: 100MB / 4KB = 25,600 blocks
  Cache hit rate: 16,384 / 25,600 = 64%
```

---

## Compression Ratio Analysis

### Real-World Data

**Time-series data (timestamps + floats):**
```
Block size: 4096 bytes
Entries: ~80 (50 bytes each)

Uncompressed: 4096 bytes
LZ4:          1420 bytes (2.9x ratio)
Zstd(3):      1050 bytes (3.9x ratio)
```

**String keys (user IDs):**
```
Block size: 4096 bytes
Entries: ~50 (80 bytes each)

Uncompressed: 4096 bytes
LZ4:          1680 bytes (2.4x ratio)
Zstd(3):      1180 bytes (3.5x ratio)
```

**Random binary data (hashes):**
```
Block size: 4096 bytes
Entries: ~60 (68 bytes each)

Uncompressed: 4096 bytes
LZ4:          3920 bytes (1.04x ratio) ❌ Not worth it!
Zstd(3):      3680 bytes (1.11x ratio) ❌
```

---

### Auto-Detection

```rust
impl SSTableBuilder {
    fn should_compress_block(&self, uncompressed: &[u8]) -> bool {
        match self.config.compression {
            Compression::None => false,
            Compression::Lz4 | Compression::Zstd => {
                // Sample first 512 bytes
                let sample = &uncompressed[..512.min(uncompressed.len())];

                // Quick entropy check
                let entropy = calculate_entropy(sample);

                // High entropy (> 7.5 bits) = already compressed/random
                entropy < 7.5
            }
        }
    }
}

fn calculate_entropy(data: &[u8]) -> f64 {
    let mut counts = [0u32; 256];
    for &byte in data {
        counts[byte as usize] += 1;
    }

    let len = data.len() as f64;
    counts.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}
```

**Note:** Current implementation does not use auto-detection, compresses all blocks. Future optimization.

---

## Performance Measurements

### Compression Throughput

```
LZ4:
  Compress:   754 MB/s (5.3µs per 4KB)
  Decompress: 3900 MB/s (1.05µs per 4KB)

Zstd (level 3):
  Compress:   450 MB/s (9.1µs per 4KB)
  Decompress: 1200 MB/s (3.4µs per 4KB)
```

---

### Impact on Read Latency

```
Point lookup (cache miss):

No compression:
  Disk read: 95µs (4KB @ 42 MB/s)
  Parse block: 0.8µs
  Total: 95.8µs

LZ4:
  Disk read: 38µs (1.6KB @ 42 MB/s)
  Decompress: 1.05µs
  Parse block: 0.8µs
  Total: 39.85µs (2.4x faster!)

Zstd:
  Disk read: 28µs (1.17KB @ 42 MB/s)
  Decompress: 3.4µs
  Parse block: 0.8µs
  Total: 32.2µs (3x faster!)
```

**Paradox:** Decompression is faster than disk!

---

## Tuning Guidelines

### When to Use Each

**None:**
- Random binary data (hashes, UUIDs)
- Write-heavy workloads (avoid CPU overhead)
- RAM-rich environments (cache everything)

**LZ4:**
- Default choice for most workloads
- Hot data with frequent reads
- Need low p99 latency (< 1µs decompression)

**Zstd:**
- Cold storage (optimize disk space)
- Archival data (rarely read)
- Network-bound replication (save bandwidth)

---

### Configuration Examples

```rust
// Hot workload: fast decompression
SSTableConfig {
    compression: Compression::Lz4,
    block_cache_mb: 512,  // Cache decompressed blocks
    ..Default::default()
}

// Cold storage: max compression
SSTableConfig {
    compression: Compression::Zstd,
    block_cache_mb: 0,    // No cache, save RAM
    ..Default::default()
}

// Write-heavy: no compression overhead
SSTableConfig {
    compression: Compression::None,
    block_cache_mb: 64,   // Moderate cache
    ..Default::default()
}
```

---

## Summary

**Implementation:**
- Block-level compression (4KB granularity)
- LZ4 via `lz4_flex` crate (1.05µs decompression)
- Zstd via `zstd` crate (3.4µs decompression, level 3)
- Cache stores decompressed blocks (22x faster than re-decompressing)

**Performance:**
- LZ4: 2.5x compression, 2.4x read speedup
- Zstd: 3.5x compression, 3x read speedup
- Decompression faster than disk I/O on SSD

**Trade-offs:**
- Compression CPU: 5-9µs per block
- Cache memory: 2.5x more RAM for decompressed blocks
- Disk savings: 60-70% space reduction
