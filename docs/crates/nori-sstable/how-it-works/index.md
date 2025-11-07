---
layout: default
title: How It Works
nav_order: 4
has_children: true
parent: nori-sstable
grand_parent: Crates
---

# How It Works
{: .no_toc }

Deep dives into the file format, algorithms, and internal workings of nori-sstable.
{: .fs-6 .fw-300 }

---

## Overview

This section explains **how** nori-sstable implements SSTables. Each page covers a specific aspect of the implementation with diagrams, code examples, and performance analysis.

## Topics

### [File Format](file-format)
Complete specification of the `.sst` file format with byte-level layout.

### [Block Format](block-format)
How entries are encoded within 4KB blocks, including prefix compression.

### [Index Structure](index-structure)
The two-level index: block index and restart points within blocks.

### [Bloom Filter](bloom-filter-impl)
Implementation details of the bloom filter using xxHash64 and double hashing.

### [Compression](compression-impl)
How LZ4 and Zstd compression are applied at block granularity.

### [Caching](cache-impl)
LRU cache implementation, eviction policy, and integration with compression.

### [Iterator](iterator-impl)
How range scans work across blocks with efficient buffering.

---

## Learning Path

**Understand the file:**
Start with [File Format](file-format) to see the overall structure.

**Dive into blocks:**
Read [Block Format](block-format) for entry encoding details.

**Optimize reads:**
Check [Bloom Filter](bloom-filter-impl) and [Caching](cache-impl).

**Advanced topics:**
Explore [Compression](compression-impl) and [Iterator](iterator-impl).
