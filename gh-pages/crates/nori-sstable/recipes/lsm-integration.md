---
layout: default
title: LSM Integration
parent: Recipes
grand_parent: nori-sstable
nav_order: 4
---

# LSM Integration
{: .no_toc }

Using nori-sstable as the storage layer for LSM-tree engines.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## LSM-Tree Overview

```
┌─────────────────────────────────────┐
│  MemTable (sorted in-memory)       │
│  - Fast writes (insert/update/del) │
│  - Limited size (64MB typical)     │
└──────────────┬──────────────────────┘
               │ Flush when full
               ↓
┌─────────────────────────────────────┐
│  Level 0 (unsorted SSTables)        │
│  - Multiple overlapping files      │
│  - Direct memtable flushes         │
└──────────────┬──────────────────────┘
               │ Compact when L0 > 4 files
               ↓
┌─────────────────────────────────────┐
│  Level 1+ (sorted, non-overlapping) │
│  - 10x size per level              │
│  - Range partitioned               │
└─────────────────────────────────────┘
```

---

## Memtable Flush

### Building L0 SSTable

```rust
use nori_sstable::{SSTableBuilder, SSTableConfig, Entry};
use std::collections::BTreeMap;

pub async fn flush_memtable(
    memtable: &BTreeMap<Bytes, Bytes>,
    output_path: PathBuf,
) -> Result<PathBuf> {
    let config = SSTableConfig {
        path: output_path.clone(),
        estimated_entries: memtable.len(),
        compression: Compression::Lz4,
        block_cache_mb: 64,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    for (key, value) in memtable {
        let entry = Entry::put(key, value);
        builder.add(&entry).await?;
    }

    let metadata = builder.finish().await?;
    println!("Flushed memtable to {} ({} MB)",
        output_path.display(),
        metadata.file_size / 1_000_000
    );

    Ok(output_path)
}
```

---

### L0 Characteristics

```
Level 0 properties:
  - Files can overlap (different memtable flush times)
  - No size limit per file (based on memtable size)
  - Requires checking ALL L0 files for a key

Example L0:
  l0_001.sst: keys [a, m]  (memtable flush #1)
  l0_002.sst: keys [d, z]  (memtable flush #2)
  l0_003.sst: keys [b, k]  (memtable flush #3)

Lookup for "f":
  Must check: l0_001, l0_002, l0_003 (all overlap "f")
```

---

## Compaction

### L0 → L1 Compaction

```rust
pub async fn compact_l0_to_l1(
    l0_files: Vec<PathBuf>,
    l1_output: PathBuf,
) -> Result<PathBuf> {
    // Open all L0 readers
    let mut readers = Vec::new();
    for path in &l0_files {
        let reader = SSTableReader::open(path.clone()).await?;
        readers.push(reader);
    }

    // Merge iterators (newest first for shadowing)
    let merged = merge_iterators(readers).await?;

    // Build L1 SSTable
    let config = SSTableConfig {
        path: l1_output.clone(),
        estimated_entries: estimate_merged_entries(&l0_files),
        compression: Compression::Lz4,
        ..Default::default()
    };

    let mut builder = SSTableBuilder::new(config).await?;

    while let Some(entry) = merged.try_next().await? {
        builder.add(&entry).await?;
    }

    builder.finish().await?;

    // Delete old L0 files
    for path in l0_files {
        std::fs::remove_file(path)?;
    }

    Ok(l1_output)
}
```

---

### Merge Iterator

```rust
use futures::stream::{Stream, StreamExt};

struct MergeIterator {
    iterators: Vec<Pin<Box<dyn Stream<Item = Result<Entry>>>>>,
    heap: BinaryHeap<MergeEntry>,
}

impl MergeIterator {
    pub async fn new(readers: Vec<Arc<SSTableReader>>) -> Result<Self> {
        let mut iterators = Vec::new();
        let mut heap = BinaryHeap::new();

        for (idx, reader) in readers.into_iter().enumerate() {
            let mut iter = reader.iter_all();

            // Prime heap with first entry from each iterator
            if let Some(entry) = iter.try_next().await? {
                heap.push(MergeEntry {
                    entry,
                    iterator_idx: idx,
                    sequence: idx,  // Newer files win
                });
                iterators.push(Box::pin(iter) as _);
            }
        }

        Ok(Self { iterators, heap })
    }

    pub async fn next(&mut self) -> Result<Option<Entry>> {
        while let Some(merge_entry) = self.heap.pop() {
            let key = merge_entry.entry.key.clone();

            // Advance this iterator
            let idx = merge_entry.iterator_idx;
            if let Some(next_entry) = self.iterators[idx].try_next().await? {
                self.heap.push(MergeEntry {
                    entry: next_entry,
                    iterator_idx: idx,
                    sequence: merge_entry.sequence,
                });
            }

            // Skip duplicates (same key from older files)
            while let Some(top) = self.heap.peek() {
                if top.entry.key == key {
                    // Same key from older file, skip it
                    let old = self.heap.pop().unwrap();
                    if let Some(next) = self.iterators[old.iterator_idx].try_next().await? {
                        self.heap.push(MergeEntry {
                            entry: next,
                            iterator_idx: old.iterator_idx,
                            sequence: old.sequence,
                        });
                    }
                } else {
                    break;
                }
            }

            return Ok(Some(merge_entry.entry));
        }

        Ok(None)
    }
}
```

---

## Tiered Compaction

### Level Sizing

```
Level   Max Size    Files (100MB each)  Fanout
L0      400 MB      4                   N/A (special)
L1      400 MB      4                   10x
L2      4 GB        40                  10x
L3      40 GB       400                 10x
L4      400 GB      4,000               10x
```

---

### Range Partitioning (L1+)

```rust
pub async fn partition_level(
    input_files: Vec<PathBuf>,
    level: usize,
    partition_size_mb: usize,
) -> Result<Vec<PathBuf>> {
    let mut outputs = Vec::new();
    let mut current_builder: Option<SSTableBuilder> = None;
    let mut current_size = 0;

    let merged = merge_iterators(input_files).await?;

    while let Some(entry) = merged.try_next().await? {
        // Start new partition if needed
        if current_builder.is_none() || current_size >= partition_size_mb * 1_000_000 {
            if let Some(builder) = current_builder.take() {
                let path = builder.finish().await?.path;
                outputs.push(path);
            }

            let output_path = format!("l{}_part{:04}.sst", level, outputs.len());
            let config = SSTableConfig {
                path: output_path.into(),
                compression: Compression::Lz4,
                ..Default::default()
            };
            current_builder = Some(SSTableBuilder::new(config).await?);
            current_size = 0;
        }

        // Add entry to current partition
        if let Some(builder) = &mut current_builder {
            builder.add(&entry).await?;
            current_size += entry.key.len() + entry.value.len();
        }
    }

    // Finish last partition
    if let Some(builder) = current_builder {
        let path = builder.finish().await?.path;
        outputs.push(path);
    }

    Ok(outputs)
}
```

---

## Read Path

### Multi-Level Lookup

```rust
pub struct LSMReader {
    memtable: Arc<RwLock<BTreeMap<Bytes, Bytes>>>,
    l0_readers: Vec<Arc<SSTableReader>>,
    l1_readers: Vec<Arc<SSTableReader>>,
    l2_readers: Vec<Arc<SSTableReader>>,
}

impl LSMReader {
    pub async fn get(&self, key: &[u8]) -> Result<Option<Entry>> {
        // 1. Check memtable (newest data)
        {
            let memtable = self.memtable.read().await;
            if let Some(value) = memtable.get(key) {
                return Ok(Some(Entry::put(key, value)));
            }
        }

        // 2. Check L0 (must check ALL files, newest first)
        for reader in self.l0_readers.iter().rev() {
            if let Some(entry) = reader.get(key).await? {
                return Ok(Some(entry));
            }
        }

        // 3. Check L1+ (binary search for range, then check file)
        for reader in &self.l1_readers {
            if reader.key_range_contains(key) {
                if let Some(entry) = reader.get(key).await? {
                    return Ok(Some(entry));
                }
            }
        }

        // 4. Not found
        Ok(None)
    }
}
```

---

### Range Contains Check

```rust
impl SSTableReader {
    pub fn key_range_contains(&self, key: &[u8]) -> bool {
        // Check if key falls in [first_key, last_key] range
        key >= self.metadata.first_key.as_ref()
            && key <= self.metadata.last_key.as_ref()
    }
}
```

**Optimization:**
```
L1 with 10 files (each 100MB, disjoint ranges):
  l1_part0000.sst: [a, j]
  l1_part0001.sst: [k, s]
  l1_part0002.sst: [t, z]

Lookup "m":
  Check range [a,j]: No
  Check range [k,s]: Yes → bloom + index search
  Check range [t,z]: No

Files checked: 1 (instead of 10!)
```

---

## Tombstone Handling

### Deletion During Compaction

```rust
pub async fn compact_with_tombstones(
    input_files: Vec<PathBuf>,
    level: usize,
) -> Result<PathBuf> {
    let merged = merge_iterators(input_files).await?;
    let mut builder = SSTableBuilder::new(config).await?;

    while let Some(entry) = merged.try_next().await? {
        if entry.tombstone {
            // Only keep tombstone if lower levels might have this key
            if has_lower_levels(level) {
                builder.add(&entry).await?;
            }
            // Otherwise, drop it (compaction GC)
        } else {
            builder.add(&entry).await?;
        }
    }

    builder.finish().await?
}

fn has_lower_levels(level: usize) -> bool {
    // If L4 (last level), no lower levels exist
    level < 4
}
```

**Example:**
```
L1 compaction:
  Entry: key="user:123", tombstone=true
  Action: Keep (L2+ might have old value)

L4 compaction (last level):
  Entry: key="user:123", tombstone=true
  Action: Drop (no lower levels, GC complete)
```

---

## Write Amplification

### Measuring Write Amp

```
Original write: 100 MB memtable flush

L0 flush: 100 MB written (1x)
L0→L1 compact: 400 MB L0 + 400 MB L1 = 800 MB read, 400 MB written (4x)
L1→L2 compact: 400 MB L1 + 4 GB L2 = 4.4 GB read, 4 GB written (40x)

Total writes: 100 + 400 + 4000 = 4,500 MB
Write amplification: 4,500 / 100 = 45x
```

---

### Reducing Write Amp

**Increase L0 threshold:**
```rust
// Compact L0→L1 when 8 files (instead of 4)
const L0_COMPACTION_THRESHOLD: usize = 8;

// Write amp improvement:
// Old (4 files): 45x
// New (8 files): 22x (2x better!)

// Trade-off: More L0 files → slower reads
```

**Increase level fanout:**
```rust
// 20x fanout instead of 10x
const LEVEL_FANOUT: usize = 20;

// L1: 400 MB
// L2: 8 GB (instead of 4 GB)
// L3: 160 GB (instead of 40 GB)

// Fewer compactions = lower write amp
```

---

## Configuration Per Level

```rust
pub fn config_for_level(level: usize) -> SSTableConfig {
    match level {
        0 => SSTableConfig {
            compression: Compression::Lz4,
            block_cache_mb: 128,  // Hot data
            bloom_bits_per_key: 12,
            ..Default::default()
        },
        1..=2 => SSTableConfig {
            compression: Compression::Lz4,
            block_cache_mb: 64,  // Warm data
            bloom_bits_per_key: 10,
            ..Default::default()
        },
        _ => SSTableConfig {
            compression: Compression::Zstd,
            block_cache_mb: 0,  // Cold data
            bloom_bits_per_key: 8,
            block_size: 8192,  // Larger blocks
            ..Default::default()
        },
    }
}
```

---

## Summary

**LSM integration checklist:**
- ✅ Flush sorted memtable to L0 SSTable
- ✅ Merge L0 files during compaction (handle overlaps)
- ✅ Partition L1+ into non-overlapping ranges
- ✅ Multi-level lookup (memtable → L0 → L1+)
- ✅ Tombstone GC at last level
- ✅ Per-level configuration (compression, cache, bloom)

**Performance tips:**
- Use bloom filters at all levels (9x fewer disk reads)
- Cache hot levels (L0/L1) more aggressively
- Increase L0 threshold to reduce write amp
- Use Zstd compression for deep levels (save disk space)

**nori-sstable advantages:**
- Immutable files → lock-free reads during compaction
- Built-in bloom filters → fast negative lookups
- Flexible compression → optimize per level
- Efficient range scans → fast compaction
