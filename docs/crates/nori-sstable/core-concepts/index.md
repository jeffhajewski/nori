# Core Concepts

Fundamental concepts behind SSTables and how they enable fast, scalable storage.

---

## Overview

SSTables (Sorted String Tables) are a foundational data structure in modern storage systems. Understanding the core concepts behind SSTables helps you use them effectively and understand the tradeoffs they make.

## Key Concepts

### [What is an SSTable?](what-is-sstable)
Learn what SSTables are, why they exist, how they work, and how they fit into LSM-tree storage engines.

### [Immutability](immutability)
Why SSTables are write-once, immutable files and the profound benefits this provides: lock-free reads, simple caching, crash safety, and zero-cost snapshots.

### [Block-Based Storage](block-based-storage)
How data is organized into fixed-size 4KB blocks for efficient I/O, caching, and compression. Understand why blocks are the quantum of I/O in SSTables.

### [Bloom Filters](bloom-filters)
Probabilistic data structures that prevent unnecessary disk reads. Learn how ~67ns checks save 100µs disk I/O and why false positives are acceptable.

### [Compression Fundamentals](compression-fundamentals)
How block-level compression reduces storage costs 2-5x with minimal performance impact. Understand LZ4 vs Zstd trade-offs and when compression actually speeds up reads.

### [When to Use SSTables](when-to-use)
Understand the use cases where SSTables excel (write-heavy, time-series, hot keys) and where they don't (ultra-low latency, heavy updates, tiny datasets).

---

## Learning Path

**New to SSTables?**
Start with [What is an SSTable?](what-is-sstable) to understand the basics.

**Want to understand the design?**
Read about [Immutability](immutability) and [Block-Based Storage](block-based-storage).

**Need performance insights?**
Check out [Bloom Filters](bloom-filters) and [Compression](compression-fundamentals).

**Ready to use them?**
Jump to [When to Use SSTables](when-to-use) for practical guidance.
