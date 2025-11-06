---
layout: default
title: Design Decisions
nav_order: 7
has_children: true
parent: nori-wal
grand_parent: Crates
---

# Design Decisions

Why nori-wal is built the way it is.
{: .fs-6 .fw-300 }

This section documents the key architectural and implementation decisions made in nori-wal, including the rationale behind each choice and alternative approaches that were considered.

## Overview

nori-wal was designed with several core principles:

1. **Production-Ready** - Comprehensive error handling, recovery, and observability
2. **Performance** - Optimized for modern hardware with careful benchmarking
3. **Simplicity** - Clear APIs, predictable behavior, minimal surprises
4. **Flexibility** - Configurable trade-offs between durability and performance
5. **Standalone** - Usable as a library without dependencies on larger systems

## Key Decisions

### [Append-Only Architecture](append-only)
Why the WAL is strictly append-only and never modifies existing data.

### [Prefix-Valid Recovery](recovery-strategy)
Why we truncate at the first corruption rather than trying to recover more data.

### [Segment-Based Storage](segmentation)
Why the WAL is split into multiple files instead of one large file.

### [CRC32C Checksums](checksums)
Why we use CRC32C for data integrity and where it's computed.

### [Varint Encoding](varint-encoding)
Why record lengths use variable-length encoding instead of fixed sizes.

### [Fsync Policies](fsync-policies)
How we balance durability and performance with configurable fsync behavior.

### [Compression Support](compression)
Why compression is optional and how it's integrated into the record format.

### [Zero-Copy Design](zero-copy)
Where we use zero-copy techniques and where we don't.

### [Observability First](observability)
Why metrics and events are built into the core rather than added later.

## Decision-Making Framework

When making design decisions for nori-wal, we prioritize:

**1. Correctness First**
- Never sacrifice data integrity for performance
- Clear error handling over silent failures
- Predictable behavior over clever optimizations

**2. Production Reality**
- Handle crashes, corruption, and operator mistakes
- Provide observability for debugging
- Document failure modes explicitly

**3. Performance Where It Matters**
- Optimize the hot path (append, sync)
- Accept slower cold paths (recovery, startup)
- Measure before optimizing

**4. Simple Mental Model**
- Append-only semantics are easy to reason about
- Explicit configuration over magic defaults
- Predictable failure modes

## Trade-offs

Every design decision involves trade-offs. We document both what we gained and what we gave up:

| Decision | Gained | Cost |
|----------|--------|------|
| Append-only | Simplicity, crash safety | No in-place updates |
| Segments | Bounded recovery time | Multiple file handles |
| CRC32C | Corruption detection | 4 bytes per record |
| Varint | Space efficiency | Parsing overhead |
| Batched fsync | 100x write throughput | Possible data loss window |

## Non-Goals

It's also important to document what nori-wal is **not** designed to do:

**Not a Database**
- No query language or indexes
- No transactions across multiple records
- No schema or type system

**Not a Message Queue**
- No consumer group management
- No message ordering guarantees beyond append order
- No built-in retention policies

**Not a Distributed System**
- No replication (use nori-raft on top)
- No sharding (use norikv-placement)
- No consensus (use nori-raft)

nori-wal is a building block, not a complete system.

## Evolution of Design

Some decisions were made early and have proven stable. Others evolved based on experience:

**Stable Since v0.1:**
- Append-only architecture
- Prefix-valid recovery
- CRC32C checksums
- Segment-based storage

**Added Based on Feedback:**
- Compression support (v0.2)
- Batch append API (v0.3)
- TTL metadata (v0.4)
- File pre-allocation (v0.5)

**Future Considerations:**
- Record-level encryption
- Alternative checksum algorithms (XXH3)
- Async fsync with io_uring on Linux

## Further Reading

Each subsection goes into detail on a specific design decision. Read them in any order based on your interests.
