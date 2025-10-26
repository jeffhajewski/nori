---
layout: default
title: Design Decisions
nav_order: 7
has_children: true
parent: nori-sstable
grand_parent: Crates
---

# Design Decisions
{: .no_toc }

Rationale behind key design choices in nori-sstable.
{: .fs-6 .fw-300 }

---

## Overview

This section explains **why** nori-sstable works the way it does. Each design decision represents a conscious trade-off, balancing performance, simplicity, and production-readiness.

## Key Decisions

### [Block-Based Organization](block-based)
Why we use 4KB blocks instead of variable-length entries.

### [Immutability](immutability-decision)
Why SSTables are write-once, never modified.

### [Compression at Block Level](compression-strategy)
Why we compress blocks individually rather than the whole file.

### [Bloom Filter Strategy](bloom-filters)
Why xxHash64 with double hashing, and how we size filters.

### [Prefix Compression](prefix-compression)
How we reduce key size within blocks while maintaining random access.

### [Cache Design](caching-strategy)
Why LRU at block granularity, and how we handle compressed blocks.

---

## Design Philosophy

nori-sstable follows these principles:

1. **Simplicity over features** - Do one thing well
2. **Performance by default** - Fast path should be the easy path
3. **Observable** - Instrument everything for debugging
4. **Crash-safe** - No corruption, ever
5. **Composable** - Works standalone or in LSM engines
