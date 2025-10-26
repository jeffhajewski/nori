---
layout: default
title: Block Format
parent: How It Works
grand_parent: nori-sstable
nav_order: 2
---

# Block Format
{: .no_toc }

Deep dive into how entries are encoded within blocks.
{: .fs-6 .fw-300 }

## Table of Contents
{: .no_toc .text-delta }

1. TOC
{:toc}

---

## Block Structure

```
┌────────────────────────────────────────────┐
│  Entries (prefix-compressed)               │
│   [Entry 0]  ← Restart point               │
│   [Entry 1]  (shared prefix with 0)        │
│   [Entry 2]  (shared prefix with 1)        │
│   ...                                       │
│   [Entry 16] ← Restart point               │
│   [Entry 17] (shared prefix with 16)       │
│   ...                                       │
│   [Entry K]                                 │
├────────────────────────────────────────────┤
│  Restart Points Array                      │
│   [0x0000: u32]  (offset of entry 0)      │
│   [0x0120: u32]  (offset of entry 16)     │
│   [0x0240: u32]  (offset of entry 32)     │
│   ...                                       │
├────────────────────────────────────────────┤
│  num_restarts: u32                         │
└────────────────────────────────────────────┘
```

**Key properties:**
- Entries use prefix compression
- Restart points every 16 entries
- Footer contains restart array + count

---

## Entry Encoding

### Full Entry (Restart Point)

```
┌──────────────┬──────────────┬──────────────┬──────────────┬──────────────┬──────────────┐
│ shared: u32  │ unshared: u32│ value_len:u32│ key_delta    │ value        │ tombstone:u8 │
│ (always 0)   │ (full len)   │              │ (full key)   │              │              │
└──────────────┴──────────────┴──────────────┴──────────────┴──────────────┴──────────────┘
```

**Example:**
```
Entry: key="user:alice", value="engineer", tombstone=false

Bytes:
  0x00 0x00 0x00 0x00   (shared=0)
  0x00 0x00 0x00 0x0A   (unshared=10)
  0x00 0x00 0x00 0x08   (value_len=8)
  "user:alice"          (10 bytes)
  "engineer"            (8 bytes)
  0x00                  (tombstone=false)
```

---

### Compressed Entry

```
┌──────────────┬──────────────┬──────────────┬──────────────┬──────────────┬──────────────┐
│ shared: u32  │ unshared: u32│ value_len:u32│ key_delta    │ value        │ tombstone:u8 │
│ (prefix len) │ (suffix len) │              │ (only suffix)│              │              │
└──────────────┴──────────────┴──────────────┴──────────────┴──────────────┴──────────────┘
```

**Example:**
```
Previous key: "user:alice"
Current key:  "user:bob"

Common prefix: "user:" (5 bytes)
Unique suffix: "bob" (3 bytes)

Bytes:
  0x00 0x00 0x00 0x05   (shared=5)
  0x00 0x00 0x00 0x03   (unshared=3)
  0x00 0x00 0x00 0x08   (value_len=8)
  "bob"                 (3 bytes, not 9!)
  "designer"            (8 bytes)
  0x00                  (tombstone=false)

Savings: 6 bytes per entry × 50 entries = 300 bytes per block!
```

---

## Restart Points

### Why Restart Points?

**Problem:** Prefix compression creates dependency chain.

```
Entry 0: "user:alice" (full)
Entry 1: "user:bob"   (shared=5, needs Entry 0)
Entry 2: "user:carol" (shared=5, needs Entry 1)
...
Entry 50: "user:zoe"  (shared=5, needs Entry 49)
```

To read Entry 50, you must decode **all 50 entries**!

---

### The Solution

**Restart every 16 entries:**

```
Entry 0:  "user:alice"  ← Restart point
Entry 1:  "user:bob"    (shared with 0)
...
Entry 15: "user:peter"  (shared with 14)
Entry 16: "user:quinn"  ← Restart point (full key)
Entry 17: "user:rob"    (shared with 16)
...
```

**Benefit:**
- To read Entry 50, decode from Entry 48 (nearest restart)
- Max decoding: 16 entries instead of 50!

---

## Block Footer

### Restart Array Format

```
Footer:
┌────────────────────────────────────┐
│ Restart offsets (4 bytes each)    │
│   offset_0:  0x0000                │
│   offset_16: 0x0220                │
│   offset_32: 0x0440                │
│   offset_48: 0x0660                │
├────────────────────────────────────┤
│ num_restarts: u32 = 4              │
└────────────────────────────────────┘
```

**Reading algorithm:**
```rust
// Find restart point for key "user:frank"
let footer_offset = block.len() - 4;
let num_restarts = u32::from_le_bytes(&block[footer_offset..]);

// Binary search restart keys
let restart_array_offset = footer_offset - (num_restarts * 4);
let restart_idx = binary_search_restarts(key, restart_array_offset);

// Decode from restart point
let start_offset = read_u32(&block[restart_array_offset + restart_idx*4]);
decode_from(start_offset, key);
```

---

## Compression

### Block-Level Compression

```
Uncompressed block (4KB):
┌────────────────────────────────────┐
│ Entries + Footer                   │  4096 bytes
└────────────────────────────────────┘
            ↓ LZ4 compression
┌────────────────────────────────────┐
│ Compressed data                    │  ~1600 bytes (2.5x ratio)
└────────────────────────────────────┘
            ↓ Add header
┌──────────────┬─────────────────────┐
│ u32: 1600    │ Compressed bytes    │
│ (size)       │                     │
└──────────────┴─────────────────────┘
```

**On-disk format:**
```rust
[compressed_size: u32][compressed_data: [u8; compressed_size]]
```

**Reading:**
```rust
let compressed_size = u32::from_le_bytes(&disk_block[0..4]);
let compressed_data = &disk_block[4..4+compressed_size];
let uncompressed = lz4::decompress(compressed_data, 4096)?;
// Now parse uncompressed block
```

---

## Cache Behavior

### Blocks in Cache

```
Cache stores DECOMPRESSED blocks:

┌─────────────────────────────────────┐
│ LRU Cache (256MB)                   │
│                                     │
│  block_0: [u8; 4096]  ← uncompressed│
│  block_5: [u8; 4096]                │
│  block_12: [u8; 4096]               │
│  ...                                │
└─────────────────────────────────────┘
```

**Why decompressed?**
- Decompress once, serve many reads
- 1µs decompression → 100ns cache hit (100x faster!)

---

## Real Block Example

### Small Block (20 entries)

```
Entry 0 (restart):
  key="2024-01-01T00:00:00", value="100.5", tombstone=false
  shared=0, unshared=19, value_len=5

Entry 1:
  key="2024-01-01T00:01:00", value="101.2", tombstone=false
  shared=16, unshared=3, value_len=5
  (only store "1:00")

Entry 2:
  key="2024-01-01T00:02:00", value="99.8", tombstone=false
  shared=16, unshared=3, value_len=4
  (only store "2:00")

...

Entry 16 (restart):
  key="2024-01-01T01:00:00", value="105.3", tombstone=false
  shared=0, unshared=19, value_len=5

Footer:
  Restarts: [0x0000, 0x0220]
  num_restarts: 2
```

**Size calculation:**
```
Without compression:
  20 entries × 50 bytes avg = 1000 bytes
  Restart array: 2 × 4 = 8 bytes
  Footer: 4 bytes
  Total: 1012 bytes

With prefix compression:
  Entry 0: 50 bytes (full)
  Entries 1-15: 30 bytes each (shared prefix)
  Entry 16: 50 bytes (full)
  Entries 17-19: 30 bytes each
  Total: 100 + 450 + 50 + 90 + 12 = 702 bytes

With LZ4:
  ~320 bytes (2.2x ratio on already compressed data!)
```

---

## Performance Characteristics

### Decoding Speed

```
Decode single entry at restart point:
  Time: 45ns (read 3 u32s + copy bytes)

Decode single entry with compression:
  Time: ~80ns (+ 35ns for prefix reconstruction)

Decode from mid-block (worst case):
  Time: 16 × 80ns = 1.28µs (decode 16 entries)

Block decompression (LZ4):
  Time: 1.05µs (entire 4KB block)
```

**Key insight:** Decompressing entire block (1µs) is faster than decoding 16 entries (1.28µs)!

---

## Summary

**Block layout:**
- Prefix-compressed entries for space efficiency
- Restart points every 16 entries for fast random access
- Footer with restart array for binary search
- Optional LZ4/Zstd compression on entire block

**Trade-offs:**
- Space: Prefix compression saves 30-50%
- Speed: Max 16 entry decodes for any lookup
- Cache: Store decompressed blocks for 100x speedup
