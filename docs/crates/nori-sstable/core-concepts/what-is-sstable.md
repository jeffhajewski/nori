# What is an SSTable?

Understanding the fundamental concept behind Sorted String Tables.

---

## Definition

An **SSTable** (Sorted String Table) is an **immutable, on-disk data structure** that stores a sorted collection of key-value pairs. SSTables are a core component of LSM-tree (Log-Structured Merge-tree) storage engines, used in databases like LevelDB, RocksDB, Cassandra, and now NoriKV.

### Key Properties

1. **Sorted**: Keys are stored in lexicographic order
2. **Immutable**: Once written, the file never changes
3. **Self-contained**: Includes data, index, bloom filter, and metadata
4. **Block-based**: Data organized into fixed-size blocks (typically 4KB)

---

## Why SSTables Exist

### The Problem: Random Writes are Slow

Traditional B-tree databases perform random writes directly to disk:
- Each write requires reading a page, modifying it, writing it back
- Causes disk head seeks (slow on HDDs)
- Even on SSDs, random writes have higher write amplification

### The Solution: Sequential Writes

SSTables enable a **write-optimized** storage approach:

1. **Writes go to memory first** (memtable)
2. When memory fills, **flush sorted data** to a new SSTable
3. All writes to an SSTable are **sequential** (fast!)
4. Reads check multiple SSTables and merge results

**Result:** 10-100x faster write throughput

---

## How SSTables Work

### Write Path

```
User writes → Memtable (in-memory sorted tree)
                  ↓ (when full)
              SSTableBuilder
                  ↓
    ┌─────────────┴─────────────┐
    │  1. Sort all entries      │
    │  2. Build blocks (4KB)    │
    │  3. Build bloom filter    │
    │  4. Build index           │
    │  5. Write footer          │
    └───────────────────────────┘
                  ↓
         Immutable SSTable file
```

### Read Path

```
User reads key "user:42"
    ↓
Check bloom filter (67ns)
    ↓ (probably exists)
Search index (binary search)
    ↓ (find block 15)
Read block 15 from disk or cache
    ↓ (decompress if needed)
Binary search within block
    ↓
Return value
```

**Typical read latency:** ~5-10µs (cache hit), ~100µs-1ms (disk read)

---

## SSTable File Structure

```
┌────────────────────────────────────┐
│ Data Blocks (sorted key-value)    │
│                                    │
│  Block 0: [aaa...azz] (4KB)       │
│  Block 1: [baa...bzz] (4KB)       │
│  Block 2: [caa...czz] (4KB)       │
│  ...                               │
│  Block N: [yaa...zzz] (4KB)       │
│                                    │
├────────────────────────────────────┤
│ Index (points to blocks)           │
│  "aaa" → Block 0, offset 0         │
│  "baa" → Block 1, offset 4096      │
│  "caa" → Block 2, offset 8192      │
│  ...                               │
├────────────────────────────────────┤
│ Bloom Filter                       │
│  (probabilistic set membership)    │
├────────────────────────────────────┤
│ Footer (64 bytes)                  │
│  - Magic number (validation)       │
│  - Index offset/size               │
│  - Bloom filter offset/size        │
│  - Compression type                │
│  - CRC32C checksum                 │
└────────────────────────────────────┘
```

**Size:** Typically 1MB-256MB per SSTable

---

## SSTables in LSM-Trees

SSTables power LSM-tree storage engines by enabling:

### Leveled Compaction

```
Level 0: [SST1] [SST2] [SST3]  ← Recent, may overlap
            ↓ compact
Level 1: [SST4 ─────] [SST5 ─────]  ← No overlaps, 10x size
            ↓ compact
Level 2: [SST6 ──────────────]  ← No overlaps, 100x size
```

- **Level 0:** Fresh SSTables from memtable flushes
- **Level 1+:** Merged, non-overlapping SSTables
- **Compaction:** Background process that merges SSTables

### Point Reads

```rust
// Check newest to oldest
if let Some(val) = memtable.get(key) { return val; }
for sst in level_0.iter() {
    if sst.bloom_contains(key) && sst.get(key).is_some() {
        return val;
    }
}
for sst in level_1.iter() { /* ... */ }
// ...
```

### Range Scans

```rust
// Merge iterators from all levels
let mut heap = MergeHeap::new();
heap.push(memtable.iter());
for sst in all_sstables {
    heap.push(sst.iter_range(start, end));
}

while let Some((key, value)) = heap.pop() {
    // Return newest version of each key
    yield (key, value);
}
```

---

## Comparison to B-Trees

| Aspect | SSTable (LSM) | B-Tree |
|--------|---------------|--------|
| **Write pattern** | Sequential | Random |
| **Write throughput** | High (10-100K/sec) | Medium (1-10K/sec) |
| **Read throughput** | Medium (need to check multiple files) | High (single lookup) |
| **Space amplification** | Medium (1.1-1.5x) | Low (1.0-1.1x) |
| **Compaction** | Background cost | None |
| **Use case** | Write-heavy, append-mostly | Read-heavy, updates |

**Key insight:** SSTables trade read complexity for write performance.

---

## Real-World Analogies

### Library Card Catalog

Think of each SSTable as a **card catalog drawer**:

- Each drawer contains cards (key-value pairs) in **alphabetical order**
- When you add new books, you create a **new drawer** (don't modify old ones)
- Periodically, you **merge drawers** to consolidate and remove duplicates
- To find a book, you check drawers **newest to oldest**

### Git Commits

SSTables are like **git commits**:

- Each commit (SSTable) is **immutable**
- New commits **don't change old ones**
- Occasionally, you **rebase/compact** to clean up history
- To get current state, you **merge** all commits

---

## Benefits of Immutability

SSTables are **write-once, never modified**. This provides:

### 1. Simple Concurrency

```rust
// Multiple readers, no locks needed!
let reader = Arc::new(SSTableReader::open("data.sst").await?);

tokio::spawn({
    let r = reader.clone();
    async move { r.get(b"key1").await }
});
tokio::spawn({
    let r = reader.clone();
    async move { r.get(b"key2").await }
});
```

No locks, no coordination, no reader-writer conflicts.

### 2. Easy Caching

```
Block 15 in cache? Yes
├─ It will NEVER change
└─ Cache it forever (until evicted)
```

### 3. Simple Crash Recovery

```
SSTable file exists?
├─ Yes Footer checksum valid → File is good
└─ No Checksum invalid → Discard (write failed mid-creation)
```

### 4. Snapshot Isolation

```
Snapshot = "Read from SSTable set at time T"
├─ SSTables never change
├─ Just keep references alive
└─ Perfect MVCC (multi-version concurrency control)
```

---

## Common Misconceptions

###  "SSTables are slow for reads"

**Reality:** With bloom filters and caching, SSTables can achieve **< 10µs** reads for hot data. Bloom filters skip 99%+ of unnecessary disk reads.

###  "SSTables waste space with duplicates"

**Reality:** Compaction merges SSTables and removes old versions. Space amplification is typically only **1.1-1.5x**.

###  "Compaction causes unpredictable latency"

**Reality:** Modern LSM engines use **rate limiting** and **tiered compaction** to keep p99 latency under SLO.

---

## When to Use SSTables

###  Great Fit

- **Write-heavy workloads** (logging, time-series, events)
- **Append-mostly data** (few updates/deletes)
- **Range scans** (sequential iteration)
- **Hot key patterns** (caching helps a lot)
- **Large datasets** that don't fit in memory

###  Not the Right Tool

- **Ultra-low latency** (< 1µs) requirements
- **Random updates** to same keys (causes many tombstones)
- **Tiny datasets** (< 1MB) where overhead dominates
- **No compaction budget** (need background I/O for merges)

---

## Next Steps

**Understand immutability:**
Read about [Immutability](immutability.md) and why it's central to SSTable design.

**Learn about organization:**
See [Block-Based Storage](block-based-storage.md) for how data is structured.

**Optimize reads:**
Check out [Bloom Filters](bloom-filters.md) to understand how we avoid disk I/O.

**Dive into implementation:**
Jump to [How It Works](../how-it-works/index.md) for file format details.
