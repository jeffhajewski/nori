# How It Works

Deep dives into the file format, algorithms, and internal workings of nori-sstable.

---

## Overview

This section explains **how** nori-sstable implements SSTables. Each page covers a specific aspect of the implementation with diagrams, code examples, and performance analysis.

## Topics

### [File Format](file-format.md)
Complete specification of the `.sst` file format with byte-level layout.

### [Block Format](block-format.md)
How entries are encoded within 4KB blocks, including prefix compression.

### [Index Structure](index-structure.md)
The two-level index: block index and restart points within blocks.

### [Bloom Filter](bloom-filter-impl.md)
Implementation details of the bloom filter using xxHash64 and double hashing.

### [Compression](compression-impl.md)
How LZ4 and Zstd compression are applied at block granularity.

### [Caching](cache-impl.md)
LRU cache implementation, eviction policy, and integration with compression.

### [Iterator](iterator-impl.md)
How range scans work across blocks with efficient buffering.

---

## Learning Path

**Understand the file:**
Start with [File Format](file-format.md) to see the overall structure.

**Dive into blocks:**
Read [Block Format](block-format.md) for entry encoding details.

**Optimize reads:**
Check [Bloom Filter](bloom-filter-impl.md) and [Caching](cache-impl.md).

**Advanced topics:**
Explore [Compression](compression-impl.md) and [Iterator](iterator-impl.md).
