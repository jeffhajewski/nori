# Compression Fundamentals

Why and how nori-sstable compresses data at block granularity.

---

## Why Compress?

### The Storage Cost Problem

```
1 million entries * 1KB/entry = 1GB uncompressed

Storage costs:
  SSD: $0.10/GB/month = $0.10/month
  S3: $0.023/GB/month = $0.023/month

Over 1 year: $1.20 (SSD) or $0.28 (S3)

With 2.5x compression → 400MB:
Over 1 year: $0.48 (SSD) or $0.11 (S3)

Savings: 60% storage cost
```

**For large datasets (TB scale), compression saves thousands of dollars/year.**

---

### The I/O Speed Benefit

```
Read 1GB from SSD:
  Uncompressed: 1GB / 3 GB/s = 333ms

  Compressed (2.5x):
    Read: 400MB / 3 GB/s = 133ms
    Decompress: 400MB / 3.9 GB/s = 102ms
    Total: 235ms

Speedup: 333ms / 235ms = 1.4x faster!
```

**Key insight:** If decompression is faster than disk, compression actually **speeds up** reads!

---

## Block-Level Compression

### Why Not Compress the Whole File?

```
Whole-file compression:
  ┌─────────────────────────┐
  │ Compressed SSTable      │
  │ (entire 1GB compressed) │
  └─────────────────────────┘

To read 1 key:
  1. Decompress entire 1GB → RAM
  2. Find key
  Problem: Need to decompress everything!
```

### Why Not Compress Per-Entry?

```
Per-entry compression:
  Entry 1: compress("user:alice", "data...") → 50 bytes
  Entry 2: compress("user:bob", "data...")   → 48 bytes
  ...

Problems:
  - Poor ratio (no cross-entry patterns)
  - Overhead per entry (header, dictionary)
  - More complex
```

### Block-Level: The Sweet Spot

```
Block-level compression:
  Block 0 (4KB):
    Entry 1: "user:alice" → "data..."
    Entry 2: "user:bob"   → "data..."
    ...
    Entry 50: "user:zach" → "data..."

  Compress entire block → ~1.6KB

To read 1 key:
  1. Read compressed block (1.6KB from disk)
  2. Decompress block (4KB → RAM in 1µs)
  3. Find key

Benefits:
  - Good ratio (compress ~50 entries together)
  - Fast (decompress only needed block)
  - Simple (block = unit of compression)
```

---

## Compression Algorithms

### LZ4 (Recommended Default)

**Properties:**
- Compression: 2-3x typical
- Compression speed: ~750 MB/s
- **Decompression speed: ~3,900 MB/s** (blazingly fast!)
- CPU cost: Minimal (<10% on reads)

**Use case:** General purpose, hot data, real-time systems

```rust
SSTableConfig {
    compression: Compression::Lz4,
    ..Default::default()
}
```

---

### Zstd (Higher Ratio)

**Properties:**
- Compression: 3-5x typical
- Compression speed: ~450 MB/s
- Decompression speed: ~1,200 MB/s
- CPU cost: Medium (~20-30% on reads)

**Use case:** Cold storage, archival data, when storage cost >> CPU cost

```rust
SSTableConfig {
    compression: Compression::Zstd,
    ..Default::default()
}
```

---

### None (No Compression)

**Properties:**
- Compression: 1x (no savings)
- Speed: Zero CPU overhead
- Best for: Already-compressed data (images, videos)

**Use case:** Development, benchmarking, pre-compressed data

```rust
SSTableConfig {
    compression: Compression::None,
    ..Default::default()
}
```

---

## Compression Ratio Analysis

### Text Data

```
Block contents (4KB):
  "user:alice" → "{'name':'Alice','age':25,'city':'NY'}"
  "user:bob"   → "{'name':'Bob','age':30,'city':'SF'}"
  ...

Patterns:
  - Repeated strings: "user:", "name", "age", "city"
  - JSON structure: "{'" "':'", "'}"
  - Common values: city names

LZ4 compression: 4096 bytes → 1400 bytes (2.9x)
Zstd compression: 4096 bytes → 1100 bytes (3.7x)
```

---

### Time-Series Data

```
Block contents (4KB):
  "metric:cpu.usage:1678900000" → "45.2"
  "metric:cpu.usage:1678900001" → "45.3"
  "metric:cpu.usage:1678900002" → "45.1"
  ...

Patterns:
  - Highly repeated prefixes: "metric:cpu.usage:"
  - Sequential timestamps: 1678900000, 1678900001, ...
  - Similar float values: 45.x

LZ4 compression: 4096 bytes → 500 bytes (8.2x!)
Zstd compression: 4096 bytes → 350 bytes (11.7x!)
```

---

### Random Binary Data

```
Block contents (4KB):
  Random bytes: [0xA3, 0x2F, 0x91, ...]

Patterns: None (random)

LZ4 compression: 4096 bytes → 4100 bytes (no compression, added header)
Zstd compression: 4096 bytes → 4120 bytes (no compression)

Auto-detect: Skip compression for incompressible blocks
```

---

## Decompression Performance

### LZ4 Benchmark

```
Decompress 4KB block:
  Time: ~1µs
  Throughput: 4KB / 1µs = 3.9 GB/s

CPU cycles: ~4,000 cycles @ 4GHz
  - Tiny fraction of CPU time
  - L3 cache miss costs more (200ns = 800 cycles)

Conclusion: LZ4 decompression is essentially free!
```

### Zstd Benchmark

```
Decompress 4KB block:
  Time: ~3.3µs
  Throughput: 4KB / 3.3µs = 1.2 GB/s

CPU cycles: ~13,000 cycles @ 4GHz
  - Still fast, but 3x slower than LZ4
  - Noticeable at high request rates

Trade-off: 30% better compression, 3x slower decompression
```

---

## Compression + Caching Synergy

### Without Caching

```
Read same key 100 times:
  1. Read compressed block from disk (100µs)
  2. Decompress (1µs)
  3. Find key in block
  4. Repeat 100 times

Total: 100 * 101µs = 10.1ms
```

### With Caching (Decompressed Blocks)

```
Read same key 100 times:
  First read:
    1. Read compressed block (100µs)
    2. Decompress (1µs)
    3. Cache decompressed block
    4. Find key

  Next 99 reads:
    1. Lookup in cache (100ns)
    2. Find key in cached block
    Total: 99 * 100ns = 9.9µs

Total: 101µs + 9.9µs = 111µs (90x faster!)
```

**Key insight:** Compress on disk, cache decompressed in RAM. Best of both worlds!

---

## Implementation Details

### Compression During Write

```rust
impl BlockBuilder {
    pub fn finish(self, compression: Compression) -> Vec<u8> {
        let uncompressed = self.buffer;

        match compression {
            Compression::None => uncompressed,

            Compression::Lz4 => {
                lz4::compress(&uncompressed)
            },

            Compression::Zstd => {
                zstd::compress(&uncompressed, level=3)
            },
        }
    }
}
```

### Decompression During Read

```rust
impl SSTableReader {
    async fn read_block(&self, block_id: usize) -> Result<Block> {
        // 1. Check cache (stores decompressed blocks)
        if let Some(block) = self.cache.get(block_id) {
            return Ok(block);
        }

        // 2. Read compressed block from disk
        let compressed = self.read_compressed_block(block_id).await?;

        // 3. Decompress
        let decompressed = match self.compression {
            Compression::None => compressed,
            Compression::Lz4 => lz4::decompress(&compressed)?,
            Compression::Zstd => zstd::decompress(&compressed)?,
        };

        // 4. Cache decompressed block
        let block = Block::parse(decompressed)?;
        self.cache.insert(block_id, block.clone());

        Ok(block)
    }
}
```

---

## Choosing a Compression Algorithm

### Decision Tree

```
Is data hot (frequently accessed)?
├─ Yes → Use LZ4
│   - Fast decompression (3.9 GB/s)
│   - Cache hit rate high anyway
│   - Want minimal CPU overhead
│
└─ No → Use Zstd
    - Better compression (3-5x vs 2-3x)
    - Slower decompression acceptable for cold data
    - Storage cost matters more than CPU
```

### Workload-Specific Recommendations

| Workload | Algorithm | Rationale |
|----------|-----------|-----------|
| **Real-time queries** | LZ4 | Ultra-fast decompression |
| **Hot key-value** | LZ4 | Cache hit rate high, speed matters |
| **Cold storage** | Zstd | Maximize storage savings |
| **Archival** | Zstd (level 9) | Best compression, slow reads OK |
| **Pre-compressed** | None | Already compressed (images, video) |
| **Development** | None | Fast iteration, no compression overhead |

---

## Advanced: Compression Levels

### Zstd Levels

```rust
// Default
zstd::compress(data, level=3)  // Fast, good ratio

// Higher compression (slower)
zstd::compress(data, level=9)  // Better ratio, 2x slower
zstd::compress(data, level=19) // Best ratio, 10x slower

Comparison:
  Level 3:  3.5x ratio, 450 MB/s compress, 1.2 GB/s decompress
  Level 9:  4.0x ratio, 180 MB/s compress, 1.2 GB/s decompress
  Level 19: 4.5x ratio, 50 MB/s compress, 1.2 GB/s decompress
```

**Note:** Decompression speed is **same** regardless of level! Higher level only affects write-time compression.

### LZ4 Levels

LZ4 has only one level (fast mode). For better ratio, use LZ4HC:

```rust
lz4_hc::compress(data, level=9)  // 2.5-3x ratio, slower compression
```

**Trade-off:** LZ4HC compresses slower (200 MB/s vs 750 MB/s) but decompresses at same speed (3.9 GB/s).

---

## Storage Savings Calculation

### Example: 100GB SSTable

```
Uncompressed: 100GB

With LZ4 (2.5x):
  Disk: 40GB
  Savings: 60GB
  At $0.10/GB/month: $6/month saved

With Zstd (3.5x):
  Disk: 28.6GB
  Savings: 71.4GB
  At $0.10/GB/month: $7.14/month saved

Extra savings with Zstd: $1.14/month
CPU cost: 3x slower decompression (acceptable for cold data)
```

**Decision:** For cold storage, Zstd's extra 1.4x compression justifies the CPU cost.

---

## Compression and Prefix Compression

### Two Types of Compression

```
1. Prefix compression (within block):
   "user:alice" → store full key
   "user:bob"   → store "bob" (shares "user:")
   "user:carol" → store "carol"

2. Block compression (entire block):
   Take prefix-compressed block → compress with LZ4/Zstd
```

**Combined effect:**

```
Original block: 4KB (no prefix compression, no block compression)
With prefix compression: 2.8KB (1.4x from prefix compression)
With prefix + LZ4: 1.1KB (2.5x from LZ4 on top)

Total: 4KB → 1.1KB (3.6x combined!)
```

---

## Monitoring Compression

### Metrics to Track

```rust
pub struct CompressionStats {
    pub blocks_compressed: u64,
    pub bytes_uncompressed: u64,
    pub bytes_compressed: u64,
    pub compression_ratio: f64,
    pub decompression_time_ns: u64,
}

// Example values
CompressionStats {
    blocks_compressed: 10_000,
    bytes_uncompressed: 40_960_000,  // 10K * 4KB
    bytes_compressed: 16_384_000,    // Compressed size
    compression_ratio: 2.5,
    decompression_time_ns: 10_000_000, // 10ms total (1µs avg)
}
```

### Red Flags

```
compression_ratio < 1.2
  → Data is incompressible (already compressed?)
  → Consider Compression::None

decompression_time > 5µs per block
  → CPU bottleneck
  → Consider switching Zstd → LZ4

bytes_compressed > bytes_uncompressed
  → Compression adding overhead
  → Disable compression
```

---

## Best Practices

### 1. Use LZ4 by Default

```rust
// For most workloads
SSTableConfig {
    compression: Compression::Lz4,
    ..Default::default()
}
```

### 2. Use Zstd for Cold Storage

```rust
// For archival data
SSTableConfig {
    compression: Compression::Zstd,
    block_cache_mb: 0,  // Disable cache (cold data)
    ..Default::default()
}
```

### 3. Disable for Pre-Compressed Data

```rust
// For images, videos, already-compressed files
SSTableConfig {
    compression: Compression::None,
    ..Default::default()
}
```

### 4. Monitor Compression Ratio

```rust
let ratio = bytes_uncompressed / bytes_compressed;

if ratio < 1.5 {
    // Compression not effective, consider disabling
    log::warn!("Low compression ratio: {}", ratio);
}
```

---

## Summary

**Compression is essential for SSTables:**

 **Storage savings** - 2-5x reduction in disk usage
 **Faster I/O** - Read less from disk (if decompression faster than disk)
 **Block-granular** - Decompress only needed blocks
 **Cache-friendly** - Cache decompressed blocks for speed
 **Configurable** - LZ4 for hot, Zstd for cold, None for development

**Key insight:** Modern compression (especially LZ4) is so fast that it often speeds up reads by reducing I/O, not just saving storage.

---

## Next Steps

**Deep dive on algorithms:**
See [Compression Guide](../compression.md) for detailed LZ4/Zstd comparison and configuration.

**Understand design choices:**
Check [Design Decisions: Compression Strategy](../design-decisions/compression-strategy.md) for compression design rationale.

**Learn caching interaction:**
Read [Caching](../caching.md) for how compression + cache work together.

**Optimize for your workload:**
Explore [Performance: Tuning](../performance/tuning.md) for compression configuration guidance.
