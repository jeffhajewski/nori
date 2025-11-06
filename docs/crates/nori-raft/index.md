---
layout: default
title: nori-raft
parent: Crates
nav_order: 4
---

# nori-raft

Raft consensus algorithm with read-index optimization and leader leases.

## Overview

`nori-raft` implements the Raft distributed consensus protocol for building fault-tolerant replicated state machines. It provides strong consistency guarantees with configurable read optimizations.

## Planned Features

- **Leader election** with randomized timeouts
- **Log replication** with at-rest quorum guarantees
- **Read-index protocol** for linearizable reads without log appends
- **Leader leases** for local reads with bounded staleness
- **Joint consensus** for safe membership changes
- **Snapshot support** for log compaction and fast recovery
- **Pre-vote** to prevent disruptive elections

## Status

🚧 **In Development** - Core implementation in progress

## Design Goals

- **Modularity** - Separate transport, storage, and state machine concerns
- **Observability** - First-class metrics and event tracing via `nori-observe`
- **Testing** - Property-based tests and linearizability checking
- **Performance** - Sub-millisecond leader election, batched log appends

## Architecture

Follows the Raft paper with optimizations from:
- "In Search of an Understandable Consensus Algorithm" (Ongaro & Ousterhout, 2014)
- etcd Raft implementation patterns
- CockroachDB read-index and lease techniques

## Installation

```toml
[dependencies]
nori-raft = "0.1"  # Not yet published
```

## References

- [Raft Paper](https://raft.github.io/raft.pdf)
- [Raft Visualization](http://thesecretlivesofdata.com/raft/)

## License

MIT OR Apache-2.0
