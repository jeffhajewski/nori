---
layout: default
title: nori-swim
parent: Crates
nav_order: 5
---

# nori-swim

SWIM-based failure detection and membership protocol.

## Overview

`nori-swim` implements a scalable gossip-based membership protocol for distributed systems. It provides eventually-consistent cluster state with probabilistic failure detection.

## Planned Features

- **Gossip-based dissemination** with O(log N) message complexity
- **Probabilistic failure detection** with configurable suspicion timeout
- **Indirect probing** for accurate failure detection through network partitions
- **Piggyback updates** for efficient state synchronization
- **Event stream** for membership change notifications

## Status

🚧 **In Development** - Design complete, implementation pending

## Design Goals

- **Scalability** - Support clusters of 100+ nodes with constant overhead per node
- **Accuracy** - Minimize false positives through indirect probes
- **Integration** - Trigger Raft reconfiguration on membership changes
- **Observability** - Emit membership events for monitoring

## Architecture

Based on:
- "SWIM: Scalable Weakly-consistent Infection-style Process Group Membership Protocol" (Das et al., 2002)
- HashiCorp Serf implementation patterns

## Protocol Phases

1. **Ping** - Direct health check to random member
2. **Ping-Req** - Indirect check if direct ping fails
3. **Suspect** - Mark member as suspected, request confirmation
4. **Alive/Dead** - Confirm state based on indirect probes

## Installation

```toml
[dependencies]
nori-swim = "0.1"  # Not yet published
```

## References

- [SWIM Paper](http://www.cs.cornell.edu/projects/Quicksilver/public_pdfs/SWIM.pdf)
- [Serf by HashiCorp](https://www.serf.io/)

## License

MIT OR Apache-2.0
