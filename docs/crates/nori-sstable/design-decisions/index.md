# Design Decisions

Rationale behind key design choices in nori-sstable.

---

## Overview

This section explains **why** nori-sstable works the way it does. Each design decision represents a conscious trade-off, balancing performance, simplicity, and production-readiness.

## Key Decisions

### [Block-Based Organization](block-based)
Why we use fixed-size 4KB blocks: OS page alignment, compression sweet spot, cache efficiency, and performance validation.

### [Compression Strategy](compression-strategy)
Why block-level compression with LZ4 default: decompression speed, cache interaction, and cold storage with Zstd.

### [Bloom Filter Strategy](bloom-strategy)
Why 10 bits/key with xxHash64 double hashing: false positive rate analysis, whole-file bloom, and loaded-on-open design.

---

## Design Philosophy

nori-sstable follows these principles:

1. **Simplicity over features** - Do one thing well
2. **Performance by default** - Fast path should be the easy path
3. **Observable** - Instrument everything for debugging
4. **Crash-safe** - No corruption, ever
5. **Composable** - Works standalone or in LSM engines
