---
layout: default
title: Core Concepts
nav_order: 2
has_children: true
parent: nori-sstable
grand_parent: Crates
---

# Core Concepts
{: .no_toc }

Fundamental concepts behind SSTables and how they enable fast, scalable storage.
{: .fs-6 .fw-300 }

---

## Overview

SSTables (Sorted String Tables) are a foundational data structure in modern storage systems. Understanding the core concepts behind SSTables helps you use them effectively and understand the tradeoffs they make.

## Key Concepts

### [What is an SSTable?](what-is-sstable)
Learn what SSTables are, why they exist, and how they fit into LSM-tree storage engines.

### [Immutability](immutability)
Why SSTables are write-once, immutable files and the benefits this provides.

### [Block-Based Storage](block-based-storage)
How data is organized into 4KB blocks for efficient I/O and caching.

### [Bloom Filters](bloom-filters)
Probabilistic data structures that prevent unnecessary disk reads.

### [Compression](compression-fundamentals)
How block-level compression reduces storage costs with minimal performance impact.

### [When to Use SSTables](when-to-use)
Understand the use cases where SSTables excel and where they don't.

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
