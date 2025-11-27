# Block-Based Storage

Why SSTables organize data into fixed-size blocks and how this design enables efficient I/O and caching.

---

## The Core Design

**SSTables divide data into fixed-size blocks** (typically 4KB). Each block contains multiple key-value entries, compressed independently, and serves as the atomic unit of I/O and caching.

```
SSTable File:
┌──────────────┐
│  Block 0     │ ← 4KB (compressed)
│  Block 1     │ ← 4KB (compressed)
│  Block 2     │ ← 4KB (compressed)
│     ...      │
│  Block N     │ ← 4KB (compressed)
├──────────────┤
│  Index       │ ← Maps keys → block numbers
│  Bloom       │
│  Footer      │
└──────────────┘
```

---

## Why Blocks?

### Alternative 1: Variable-Length Entries (No Blocks)

```
File: [entry1][entry2][entry3]...[entryN]
```

**Problems:**
- Read key 1000 → Must scan from beginning
- Random access requires full file index (memory overhead)
- Cache entire file or nothing (no granularity)

### Alternative 2: One Block Per Entry

```
File: [block:entry1][block:entry2]...[block:entryN]
```

**Problems:**
- Massive overhead (footer per entry)
- Poor compression (can't compress across entries)
- Index size explodes (one entry per key)

### Solution: Fixed-Size Blocks

```
Block: [entry1][entry2]...[entryK] ← ~4KB total
```

**Benefits:**
- Random access via block index (log B lookups)
- Cache at block granularity (balance memory vs I/O)
- Good compression (compress ~100 entries together)
- Reasonable index size (one entry per block)

---

## Block Size Trade-Offs

### Small Blocks (1KB-2KB)

**Advantages:**
- Fine-grained caching (less wasted memory)
- Lower read amplification per key
- Faster decompression per block

**Disadvantages:**
- More index entries (larger index)
- Poor compression ratio (less data to compress together)
- More overhead (footer, checksum per block)

### Large Blocks (16KB-64KB)

**Advantages:**
- Better compression ratio (more context)
- Smaller index (fewer blocks)
- Less overhead per byte

**Disadvantages:**
- Coarse-grained caching (waste memory on unused entries)
- Higher read amplification (read 64KB to get 1 key)
- Slower decompression (more data to decompress)

### The Sweet Spot: 4KB

**nori-sstable uses 4KB blocks by default:**

```rust
pub const DEFAULT_BLOCK_SIZE: usize = 4096;
```

**Why 4KB?**
- Matches OS page size (aligned I/O)
- Contains ~50-200 entries (good compression)
- Small enough for efficient caching
- Large enough to amortize overhead

**Performance:**
- Compression ratio: 2-3x (LZ4)
- Decompress time: < 1µs (4KB @ 3.9 GB/s)
- Cache overhead: Acceptable for 64MB cache (~16K blocks)

---

## Block Internal Structure

### Entry Layout Within Block

```
Block (4KB):
┌────────────────────────────────────────┐
│ Restart Points Array                   │
│   restart[0] = offset 0                │
│   restart[1] = offset 512              │
│   restart[2] = offset 1024             │
│   ...                                  │
├────────────────────────────────────────┤
│ Entries (prefix-compressed)            │
│                                        │
│  Entry 0: shared=0, unshared=10, val=5 │
│           key="user:alice", val="..."  │
│                                        │
│  Entry 1: shared=5, unshared=3, val=5  │
│           key="bob" (→ "user:bob")     │
│                                        │
│  Entry 2: shared=5, unshared=7, val=5  │
│           key="charlie" (→ "user:charlie")│
│  ...                                   │
│                                        │
│  Entry 16: shared=0, unshared=11, val=5│ ← Restart point
│            key="user:david", val="..." │
│  ...                                   │
└────────────────────────────────────────┘
```

### Restart Points

**Purpose:** Enable binary search within compressed blocks.

```rust
// Every 16 entries = restart point (full key)
pub const RESTART_INTERVAL: usize = 16;
```

**Why needed?**
- Entries are prefix-compressed
- To decode entry N, need entry N-1
- Restart points have full keys (no dependency)
- Binary search jumps to restart points

**Trade-off:**
- More restarts = faster search, worse compression
- Fewer restarts = better compression, slower search
- Default 16 = good balance

---

## Block-Level I/O

### Reading a Single Key

```
1. User: sst.get(b"user:bob")
2. Check bloom filter (~67ns)
3. Search index → Block 5 contains range ["user:alice" ... "user:david"]
4. Read block 5 (4KB)
   - Check cache first
   - If miss: read from disk
5. Decompress block 5 (<1µs)
6. Binary search within block
7. Return value
```

**Key insight:** Block is the atomic unit of I/O. Even if you need 1 entry, you read the whole 4KB block.

### Read Amplification

```
Key size: 20 bytes
Value size: 100 bytes
Entry size: ~120 bytes

Useful data: 120 bytes
Actual read: 4096 bytes

Read amplification: 4096 / 120 = 34x
```

**Mitigation:** Caching amortizes this over multiple reads.

---

## Block-Level Caching

### Why Cache Blocks (Not Entries)?

```rust
// Good: Cache at block granularity
cache.insert(block_id, decompressed_block);

// Bad: Cache individual entries
cache.insert(key, value);  // ❌
```

**Reasons:**

1. **Spatial locality**: If you read "user:bob", likely to read "user:alice" next
2. **Decompression cost**: Decompress once, use many times
3. **Memory efficiency**: 4KB block contains ~50-200 entries
4. **Simpler eviction**: LRU at block level

### Cache Hit Patterns

```
Cold cache:
  get("key1") → cache miss → read block 0 → decompress
  get("key2") → cache miss → read block 0 again → decompress

Warm cache:
  get("key1") → cache miss → read block 0 → decompress → cache
  get("key2") → cache HIT → return from cache (~100ns)
  get("key3") → cache HIT (if in same block)
```

**80/20 rule:** 20% of blocks contain 80% of keys accessed.

---

## Block-Level Compression

### Why Compress Per-Block?

```
Alternative 1: Compress whole file
  - Read 1 key → decompress entire file (slow!)

Alternative 2: Compress per-entry
  - Poor ratio (no cross-entry patterns)
  - More overhead

Block-level:
  - Read 1 key → decompress 4KB (fast)
  - Good ratio (compress ~100 entries together)
```

### Compression + Caching Synergy

```
Disk: Block 5 (compressed, 1.2KB LZ4)
        ↓ read from disk
RAM: Block 5 (decompressed, 4KB in cache)
        ↓ many reads
Users get 4KB of decompressed data at <100ns latency
```

**Key insight:** Cache stores *decompressed* blocks. Decompress once, serve many times.

### Example: LZ4 on 4KB Block

```
Uncompressed block: 4096 bytes
  Entry 1: "user:alice" → "data..."
  Entry 2: "user:bob"   → "data..."
  ...
  Entry 50: "user:zach" → "data..."

LZ4 compressed: ~1400 bytes (2.9x ratio)
  - Common prefix "user:" compressed
  - Repeated value patterns compressed
  - Dictionary built across all entries

Decompress time: ~1µs (4KB @ 3.9 GB/s)
```

---

## Index Structure

### Block Index

```
Index (in SSTable footer region):
┌────────────────────────────────────┐
│ Block 0: first_key="aaa", offset=0    │
│ Block 1: first_key="bbb", offset=4096 │
│ Block 2: first_key="ccc", offset=8192 │
│ ...                                │
│ Block N: first_key="zzz", offset=...  │
└────────────────────────────────────┘
```

**Search algorithm:**
```rust
fn find_block(key: &[u8]) -> BlockId {
    // Binary search in index
    index.binary_search(|block| block.first_key.cmp(key))
}
```

**Complexity:** O(log B) where B = number of blocks

**Example:**
- 1MB SSTable / 4KB blocks = 256 blocks
- Binary search: log2(256) = 8 comparisons

### Comparison: Entry-Level Index

```
Alternative: Index every key
  - 100K entries → 100K index entries
  - Index size: ~10MB (key + offset per entry)
  - Memory overhead: 10x data size!

Block-level index:
  - 100K entries / 50 per block = 2000 blocks
  - Index size: ~200KB
  - Memory overhead: 2% of data size ✓
```

---

## Real-World Example

### Scenario: 1GB SSTable

```
Configuration:
  - Block size: 4KB
  - Average entry: 120 bytes
  - Entries per block: ~34
  - Compression (LZ4): 2.5x

Math:
  - Uncompressed blocks: 1GB / 4KB = 256K blocks
  - Compressed on disk: 1GB / 2.5 = 400MB
  - Index size: 256K * 32 bytes = 8MB
  - Total file size: 400MB + 8MB = 408MB
```

### Read Performance

```
Point lookup (cache miss):
  1. Bloom filter check: ~67ns
  2. Index binary search: log2(256K) = 18 comparisons (~500ns)
  3. Read compressed block from SSD: ~100µs
  4. Decompress 4KB block: ~1µs
  5. Binary search in block: ~200ns
  Total: ~102µs

Point lookup (cache hit):
  1. Bloom filter: ~67ns
  2. Index search: ~500ns
  3. Cache lookup: ~100ns
  4. Binary search in cached block: ~200ns
  Total: ~900ns (100x faster!)
```

---

## Design Alternatives Considered

### 1. Variable-Size Blocks

**Idea:** Pack blocks until ~4KB, but allow variance.

**Pros:**
- No entry splitting across blocks
- Potentially better compression

**Cons:**
- Complex index (size per block)
- Unpredictable cache sizing
- Harder to align with OS pages

**Decision:** Fixed 4KB for simplicity and predictability.

---

### 2. Nested Blocks (2-Level)

**Idea:** 64KB "mega-blocks" containing 16x 4KB sub-blocks.

**Pros:**
- Even better compression (more context)
- Smaller index (fewer mega-blocks)

**Cons:**
- Higher read amplification
- More complex implementation
- 64KB cache entries wasteful

**Decision:** Single-level 4KB blocks for simplicity.

---

### 3. Adaptive Block Sizing

**Idea:** Small blocks for hot data, large for cold.

**Pros:**
- Optimize for access patterns

**Cons:**
- Very complex
- Unknown access patterns at write time
- Index becomes complex

**Decision:** Fixed 4KB, let caching handle hot/cold.

---

## Performance Impact

### Benchmark: Block Size Comparison

| Block Size | Index Size | Compression Ratio | Cache Hit Latency | Miss Latency |
|------------|------------|-------------------|-------------------|--------------|
| **1KB**    | 32MB       | 2.0x              | 400ns             | 80µs         |
| **4KB**    | 8MB        | 2.5x              | 900ns             | 102µs        |
| **16KB**   | 2MB        | 3.0x              | 2.5µs             | 150µs        |
| **64KB**   | 512KB      | 3.5x              | 8µs               | 280µs        |

**Optimal:** 4KB balances all factors.

---

## Implementation Details

### Block Building

```rust
pub struct BlockBuilder {
    buffer: Vec<u8>,
    restarts: Vec<u32>,
    counter: usize,
    last_key: Bytes,
}

impl BlockBuilder {
    pub fn add(&mut self, entry: &Entry) {
        if self.counter % RESTART_INTERVAL == 0 {
            // Restart point: full key
            self.restarts.push(self.buffer.len() as u32);
            self.write_entry(entry, 0); // shared_len = 0
        } else {
            // Compressed entry
            let shared = common_prefix_len(&self.last_key, &entry.key);
            self.write_entry(entry, shared);
        }
        self.counter += 1;
        self.last_key = entry.key.clone();
    }

    pub fn finish(self) -> Vec<u8> {
        let mut block = self.buffer;
        // Append restart points
        for offset in self.restarts {
            block.extend_from_slice(&offset.to_le_bytes());
        }
        block.extend_from_slice(&(self.restarts.len() as u32).to_le_bytes());
        block
    }
}
```

---

## Best Practices

### 1. Use Default 4KB

```rust
// Recommended
SSTableConfig {
    block_size: 4096,  // Default
    ..Default::default()
}

// Only change if:
// - Ultra-small keys/values (try 2KB)
// - Ultra-large values (try 8KB)
// - Profiling shows bottleneck
```

### 2. Size Cache Based on Working Set

```
Working set = hot blocks * 4KB
Cache size ≈ 1.5x working set

Example:
  10K hot keys / 50 per block = 200 hot blocks
  200 blocks * 4KB = 800KB working set
  Cache size: 1-2MB sufficient
```

### 3. Monitor Block Cache Hit Rate

```rust
let hit_rate = cache_hits / (cache_hits + cache_misses);

// Target: >80% for hot workloads
if hit_rate < 0.8 {
    // Increase cache size
}
```

---

## Summary

**Block-based storage is fundamental to SSTable design:**

✅ **Efficient I/O** - 4KB aligned with OS pages
✅ **Good compression** - Compress ~50-200 entries together
✅ **Practical caching** - Cache at block granularity
✅ **Fast lookups** - Small block index (O(log B))
✅ **Simple design** - Fixed size, no complex logic

**Key insight:** Blocks are the **quantum of I/O**. Choose block size to balance compression ratio, cache efficiency, and read amplification.

---

## Next Steps

**Understand prefix compression:**
See [How It Works: Block Format](../how-it-works/block-format) for entry encoding details.

**Learn about caching:**
Check [Caching](../caching) for how the LRU cache works with blocks.

**Optimize compression:**
Read [Compression](../compression) for how blocks are compressed.

**See the index:**
Explore [How It Works: Index Structure](../how-it-works/index-structure) for block indexing details.
