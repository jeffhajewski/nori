---
layout: default
title: How It Works
parent: nori-sstable
has_children: true
nav_order: 5
---

# How It Works

Deep dive into the internal implementation of nori-sstable.

## Pages

- **[File Format](file-format/)** - Complete `.sst` file specification with footer, index, and bloom layout
- **[Block Format](block-format/)** - Entry encoding, prefix compression, and restart points
- **[Bloom Filter Implementation](bloom-filter-impl/)** - xxHash64, double hashing, and FP rate analysis
- **[Compression Implementation](compression-impl/)** - LZ4/Zstd integration and cache design
