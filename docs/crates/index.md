---
layout: default
title: Crates
nav_order: 2
has_children: true
---

# Crates
{: .no_toc }

NoriKV is built from composable, production-ready crates that can be used individually or together.
{: .fs-6 .fw-300 }

---

## Published Crates

Each crate solves a specific problem and can be used standalone in your Rust projects.

### Storage Layer

| Crate | Purpose | Status |
|-------|---------|--------|
| **[nori-wal](nori-wal/)** | Write-ahead log with recovery | ✅ Production-ready |
| **[nori-sstable](nori-sstable/)** | Immutable sorted string tables | ✅ Production-ready |
| **[nori-lsm](nori-lsm/)** | LSM storage engine | 🚧 In development |

### Consensus & Membership

| Crate | Purpose | Status |
|-------|---------|--------|
| **[nori-raft](nori-raft/)** | Raft consensus algorithm | 🚧 In development |
| **[nori-swim](nori-swim/)** | SWIM membership protocol | 🚧 In development |

### Observability

| Crate | Purpose | Status |
|-------|---------|--------|
| **[nori-observe](nori-observe/)** | Vendor-neutral observability ABI | ✅ Ready |
| **nori-observe-prom** | Prometheus exporter | 🚧 Planned |
| **nori-observe-otlp** | OTLP exporter | 🚧 Planned |

---

## Internal Crates

These crates are used by the NoriKV server but not published separately:

- **norikv-types** - Shared types, IDs, error codes
- **norikv-placement** - Sharding and replica placement
- **norikv-transport-grpc** - gRPC transport adapter
- **norikv-testkit** - Chaos testing and linearizability checking

---

## How Crates Fit Together

```
┌─────────────────────────────────────────────────┐
│  NoriKV Server (DI composition)                 │
├─────────────────────────────────────────────────┤
│  Adapters: LSM, Raft, SWIM, gRPC, HTTP         │
├─────────────────────────────────────────────────┤
│  Ports: Storage, ReplicatedLog, Membership,    │
│         Transport, Router traits                │
├─────────────────────────────────────────────────┤
│  Domain: Types, IDs, Versions, Errors          │
└─────────────────────────────────────────────────┘
```

**Key principle:** Each crate is independently usable. You can use just `nori-wal` in your project, or combine `nori-wal` + `nori-sstable` + `nori-lsm` for a complete storage engine.

---

## Using Crates Individually

### Example: Just the WAL

```rust
[dependencies]
nori-wal = "0.1"

use nori_wal::{Wal, WalConfig};
// Use as append-only log
```

### Example: Full Storage Stack

```rust
[dependencies]
nori-lsm = "0.1"  // Includes WAL + SSTable

use nori_lsm::LsmEngine;
// Complete key-value storage
```

---

## Documentation Navigation

Click on any crate above to see its complete documentation:

- **Getting Started** - Installation and quickstart
- **API Reference** - Complete API documentation
- **Core Concepts** - Understanding the fundamentals
- **Performance** - Benchmarks and tuning
- **How It Works** - Internal implementation details
- **Recipes** - Common usage patterns

---

## Next Steps

**New to NoriKV?**
Start with [nori-wal](nori-wal/) to understand the foundation, then explore [nori-sstable](nori-sstable/) for immutable storage.

**Building a storage engine?**
Check out the [Architecture](../architecture/) section to see how components fit together.

**Need observability?**
See [nori-observe](nori-observe/) for vendor-neutral metrics and events.
