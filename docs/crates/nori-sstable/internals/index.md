# Internals

Deep implementation details for nori-sstable contributors and advanced users.

---

## Overview

This section covers the internal implementation of nori-sstable. These pages are useful for:
- Contributors wanting to understand the codebase
- Advanced users optimizing for specific workloads
- Anyone curious about how SSTables work under the hood

## Topics

### File Descriptor Management
How we manage open file handles and avoid running out of FDs.

### Block Cache Implementation
LRU cache internals, eviction policy, and thread safety.

### Bloom Filter Construction
How bloom filters are built during SSTable creation.

### Iterator Implementation
How we implement efficient range scans across blocks.

---

**Note:** This section is under development. Check back soon for detailed internal documentation.
