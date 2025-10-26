---
layout: default
title: Immutability
parent: Core Concepts
grand_parent: nori-sstable
nav_order: 2
---

# Immutability
{: .no_toc }

Why SSTables are write-once, immutable files and the profound benefits this provides.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## The Core Principle

**Once an SSTable is written, it never changes.**

This single design decision cascades into numerous benefits:
- Lock-free concurrent reads
- Simple caching semantics
- Crash-safe by design
- Perfect snapshot isolation
- Zero write amplification on reads

---

## Why Immutability?

### Problem: Mutable Files Require Coordination

In a traditional B-tree database:

```
Writer updates page 42
    ↓
1. Read page 42 from disk
2. Modify in memory
3. Write back to disk
4. Flush to ensure durability
```

**Issues:**
- Readers need locks (can't read page being written)
- Cache invalidation (did page change?)
- Crash during write = corrupted page
- Write amplification (read-modify-write)

### Solution: Write New, Never Modify

In an SSTable-based system:

```
New data arrives
    ↓
1. Accumulate in memtable (RAM)
2. When full, write NEW SSTable
3. Old SSTables unchanged
4. Background: merge old SSTables → new SSTable
```

**Benefits:**
- Readers never block (old files still valid)
- Cache forever (data never changes)
- Crash = incomplete file, just delete it
- Pure sequential writes

---

## Concurrency Benefits

### Lock-Free Reads

```rust
// Thread 1
let reader1 = Arc::new(SSTableReader::open("v1.sst").await?);
reader1.get(b"key").await?;

// Thread 2
let reader2 = reader1.clone();  // Just clone Arc
reader2.get(b"other_key").await?;  // No locks!

// Thread 3
let reader3 = reader1.clone();
reader3.iter().await?;  // Still no locks!
```

**No mutexes, no RwLocks, no coordination overhead.**

### Simple Atomic Updates

```rust
// Update "database" by atomically swapping file list
struct DB {
    sstables: Arc<RwLock<Vec<Arc<SSTableReader>>>>,
}

impl DB {
    async fn compact(&self) {
        // 1. Create new merged SSTable
        let new_sst = compact([old1, old2, old3]).await?;

        // 2. Atomically update list
        let mut tables = self.sstables.write();
        tables.retain(|sst| sst.path != old1.path && ...);
        tables.push(Arc::new(new_sst));

        // 3. Old readers still see old files (Arc keeps them alive)
        // 4. New readers see new files
    }
}
```

---

## Caching Benefits

### Forever Caching

```rust
// Cache a block from SSTable
cache.insert(
    key: (sst_id, block_index),
    value: decompressed_block,
    ttl: FOREVER  // Block will NEVER change!
);
```

**No cache invalidation logic needed.** Block 15 from `file_42.sst` will always contain the same data.

### Multi-Level Caching

```
L1: Process-local LRU cache (64MB)
    ↓ miss
L2: Shared mmap cache (OS page cache)
    ↓ miss
L3: Disk
```

All levels can cache aggressively because **data never changes**.

---

## Crash Safety

### Atomic File Creation

```
Write SSTable:
1. Write data blocks    → temp_file.sst.tmp
2. Write index          → temp_file.sst.tmp
3. Write bloom filter   → temp_file.sst.tmp
4. Write footer + CRC   → temp_file.sst.tmp
5. fsync()
6. rename() to final.sst  ← Atomic!
```

**If crash occurs:**
- Before rename: temp file deleted, no corruption
- After rename: file complete and valid

### Validation on Open

```rust
impl SSTableReader {
    pub async fn open(path: PathBuf) -> Result<Self> {
        let file = File::open(path).await?;

        // Read footer
        let footer = read_footer(&file).await?;

        // Validate CRC
        if !footer.validate_crc() {
            return Err(Error::CorruptedFile);
        }

        // File is valid!
        Ok(Self { file, footer, ... })
    }
}
```

**If corruption detected, just delete the file. No recovery needed.**

---

## Snapshot Isolation (MVCC)

### Perfect Snapshots

```rust
struct Snapshot {
    sstables: Vec<Arc<SSTableReader>>,  // Immutable!
    timestamp: u64,
}

// Create snapshot
let snap = db.snapshot();

// Later... database changed, but snapshot unchanged
snap.get(b"key").await?;  // Sees data as of snapshot time
```

**SSTables are immutable → snapshot is just a list of Arc references.**

### Time-Travel Queries

```rust
// Read data as of 1 hour ago
let snapshot = db.snapshot_at(now - 3600);
snapshot.scan(b"user:", b"user:~").await?;
```

Possible because old SSTables are kept until no longer referenced.

---

## Deletion Handling: Tombstones

### Problem: How to Delete if Immutable?

**Solution:** Write a **tombstone** (delete marker)

```rust
// Delete "user:42"
memtable.insert(b"user:42", Entry::tombstone());

// When memtable flushes
sstable.write(Entry {
    key: b"user:42",
    value: b"",
    tombstone: true,  // ← Marker
});
```

### Tombstone Lifecycle

```
1. Delete request → Tombstone in memtable
2. Flush → Tombstone in L0 SSTable
3. Read "user:42" → Sees tombstone first (newest) → Return None
4. Compaction → Merges tombstone with older live value → Removed
```

**Tombstone shadows all older versions during reads.**

---

## Compaction: Making Immutability Practical

### The Accumulation Problem

```
Day 1: SST1 (1MB)
Day 2: SST1, SST2 (2MB total)
Day 3: SST1, SST2, SST3 (3MB total)
...
Day 100: SST1...SST100 (100MB total)
```

**Problem:** Reads must check 100 files!

### Solution: Background Compaction

```rust
// Merge multiple SSTables into one
async fn compact(inputs: Vec<SSTableReader>) -> SSTable {
    let mut builder = SSTableBuilder::new(config).await?;

    // Merge-sort all inputs
    let mut heap = MergeHeap::new();
    for sst in inputs {
        heap.push(sst.iter());
    }

    while let Some((key, value, tombstone)) = heap.pop() {
        // Take newest version of each key
        if !already_seen(key) {
            builder.add(&Entry { key, value, tombstone }).await?;
        }
    }

    builder.finish().await
}
```

**Creates new SSTable with merged data. Old SSTables deleted once unreferenced.**

---

## Update Handling

### Problem: How to Update a Key?

**Solution:** Write the new value. Newest version wins.

```
Time 0: Write "user:42" = "alice" → SST1
Time 1: Write "user:42" = "bob"   → SST2

Read "user:42":
  Check SST2 (newer) → "bob" ✓ (return this)
  Check SST1 (older) → "alice" (ignored)
```

### Space Amplification

```
Same key updated N times = N copies on disk
    ↓
Compaction merges → Keep only newest
    ↓
Space amplification = (data_on_disk / live_data)
  Typical: 1.1-1.5x with regular compaction
```

---

## Comparison: Mutable vs Immutable

| Aspect | Mutable (B-tree) | Immutable (SSTable) |
|--------|------------------|---------------------|
| **Concurrency** | Locks required | Lock-free reads |
| **Caching** | Invalidation logic | Cache forever |
| **Crash recovery** | Complex (WAL replay) | Simple (validate CRC) |
| **Snapshots** | Copy-on-write | Free (just Arc refs) |
| **Write pattern** | Random (slow) | Sequential (fast) |
| **Space overhead** | Low | Medium (until compaction) |
| **Compaction** | Not needed | Required |

---

## Trade-Offs

### Advantages ✅

- **Write throughput:** 10-100x faster than random writes
- **Concurrency:** Lock-free, scales to many readers
- **Crash safety:** Atomic, no corruption possible
- **Snapshots:** Zero-cost MVCC
- **Caching:** Aggressive caching with no invalidation

### Disadvantages ❌

- **Space amplification:** Old versions accumulate until compaction
- **Read amplification:** Must check multiple files
- **Compaction cost:** Background I/O and CPU
- **Latency spikes:** During compaction (can be rate-limited)

---

## Best Practices

### 1. Regular Compaction

```rust
// Schedule compaction to keep read amplification low
tokio::spawn(async {
    loop {
        tokio::time::sleep(Duration::from_secs(3600)).await;
        db.compact().await?;
    }
});
```

### 2. Reference Counting for Deletion

```rust
// Only delete SSTables when all readers done
struct SSTable {
    reader: Arc<SSTableReader>,  // Ref-counted
}

// When compaction creates new file:
// 1. Add new SSTable to list
// 2. Remove old SSTables from list
// 3. Old files deleted when Arc count → 0
```

### 3. Bounded Retention

```rust
// Don't keep infinite history
db.compact_with_policy(CompactionPolicy {
    max_versions: 1,  // Keep only latest version
    tombstone_ttl: Duration::from_days(7),  // Drop old tombstones
});
```

---

## Real-World Analogy

Think of **version control** (like Git):

- Each commit is **immutable**
- New changes = new commit (don't modify old ones)
- To see current state, merge all commits
- Periodically, you rebase/squash to clean up history
- Old commits stay around until unreferenced

**SSTables are like commits for your database.**

---

## Next Steps

**Learn about organization:**
See [Block-Based Storage](block-based-storage) for how immutable data is structured.

**Understand deletion:**
Read about [Tombstones](../design-decisions/tombstones) in the design decisions.

**See compaction details:**
Check out [Compaction Strategy](../how-it-works/compaction) for how we manage immutability at scale.

**Explore implementation:**
Jump to [File Format](../how-it-works/file-format) to see how immutability is enforced.
