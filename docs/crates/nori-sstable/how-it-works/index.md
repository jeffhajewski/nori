# How It Works

Deep dives into the file format, algorithms, and internal workings of nori-sstable.

---

## Overview

This section explains **how** nori-sstable implements SSTables. Each page covers a specific aspect of the implementation with diagrams, code examples, and performance analysis.

## Topics

### [File Format](file-format.md)
Complete specification of the `.sst` file format with byte-level layout, including:
- Block encoding and prefix compression
- Two-level index structure
- Bloom filter implementation using xxHash64
- LZ4 and Zstd compression at block granularity
- LRU cache integration

---

## Learning Path

**Start here:**
Read [File Format](file-format.md) for a complete understanding of the SSTable structure.

**Related documentation:**
- [Compression Guide](../compression.md) - Compression algorithms and tradeoffs
- [Caching Guide](../caching.md) - Cache tuning and performance
- [Core Concepts](../core-concepts/index.md) - Conceptual overview
