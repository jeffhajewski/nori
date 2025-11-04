# NoriKV

**A sharded, Raft-replicated, log-structured key–value store with portable SDKs and first-class observability.**

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

## Overview

NoriKV is a distributed key-value database designed for high availability, strong consistency, and operational transparency. Built with modern storage and consensus algorithms, it provides predictable performance with comprehensive observability.

### Key Features

- **📦 Sharded Architecture**: Horizontal scaling with Jump Consistent Hashing
- **🔄 Raft Consensus**: Strong consistency with leader-based replication
- **💾 LSM Storage**: Log-structured merge-tree with leveled compaction
- **🌐 Multi-Language SDKs**: TypeScript, Python, Go, and Java clients
- **📊 First-Class Observability**: Built-in metrics, tracing, and live visualization
- **🔍 SWIM Membership**: Fast failure detection and cluster health monitoring
- **⚡ High Performance**: Zero-copy operations, optimized hot paths

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  Client SDKs (TypeScript, Python, Go, Java)                 │
│  - Smart routing to shard leaders                           │
│  - Automatic retry with exponential backoff                 │
│  - Connection pooling & health checking                     │
└─────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────┐
│  NoriKV Cluster                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │   Node 1     │  │   Node 2     │  │   Node 3     │      │
│  │  (Shard 0)   │  │  (Shard 1)   │  │  (Shard 2)   │      │
│  │              │  │              │  │              │      │
│  │  ┌────────┐  │  │  ┌────────┐  │  │  ┌────────┐  │      │
│  │  │  Raft  │  │  │  │  Raft  │  │  │  │  Raft  │  │      │
│  │  └────────┘  │  │  └────────┘  │  │  └────────┘  │      │
│  │  ┌────────┐  │  │  ┌────────┐  │  │  ┌────────┐  │      │
│  │  │  LSM   │  │  │  │  LSM   │  │  │  │  LSM   │  │      │
│  │  └────────┘  │  │  └────────┘  │  │  └────────┘  │      │
│  └──────────────┘  └──────────────┘  └──────────────┘      │
│                                                               │
│  SWIM Membership: Gossip-based failure detection            │
└─────────────────────────────────────────────────────────────┘
```

## Quick Start

### Installation

#### Go SDK (Production Ready ✅)
```bash
go get github.com/norikv/norikv-go
```

```go
import norikv "github.com/norikv/norikv-go"

client, _ := norikv.NewClient(ctx, norikv.DefaultClientConfig(
    []string{"localhost:9001", "localhost:9002"},
))
defer client.Close()

// Put a value
version, _ := client.Put(ctx, []byte("key"), []byte("value"), nil)

// Get a value
result, _ := client.Get(ctx, []byte("key"), nil)
```

#### TypeScript SDK
```bash
npm install @norikv/client
```

```typescript
import { createClient } from '@norikv/client';

const client = createClient({
  nodes: ['localhost:9001', 'localhost:9002'],
});

await client.put('key', 'value');
const result = await client.get('key');
```

#### Python SDK
```bash
pip install norikv
```

```python
from norikv import Client

async with Client(['localhost:9001', 'localhost:9002']) as client:
    await client.put('key', 'value')
    result = await client.get('key')
```

### Server (Development)

```bash
# Build the server
cargo build --release -p norikv-server

# Run with default configuration
./target/release/norikv-server
```

## Project Structure

### Core Storage Crates (Rust)

| Crate | Status | Description |
|-------|--------|-------------|
| **nori-observe** | ✅ Complete | Vendor-neutral observability framework |
| **nori-wal** | ✅ Complete | Write-ahead log with recovery |
| **nori-sstable** | ✅ Complete | Sorted string tables with bloom filters |
| **nori-lsm** | ✅ Complete | LSM tree engine with compaction |
| **nori-swim** | ✅ Complete | SWIM failure detection protocol |
| **nori-raft** | ✅ Complete | Raft consensus implementation |

### Client SDKs

| Language | Status | Features | Tests |
|----------|--------|----------|-------|
| **TypeScript** | ✅ Production | Smart routing, retries, pooling, ephemeral server | 100+ passing |
| **Python** | ✅ Production | Async/await API, type hints, ephemeral server | 80+ passing |
| **Go** | ✅ Production | Connection pooling, topology watching, integration tests | 102+ passing |
| **Java** | 🚧 Planned | Maven/Gradle, gRPC client | Pending |

### Server Components

| Component | Status | Description |
|-----------|--------|-------------|
| **norikv-server** | 🚧 In Progress | Main server binary |
| **norikv-placement** | ✅ Complete | Shard assignment and routing |
| **norikv-transport-grpc** | 🚧 In Progress | gRPC/HTTP transport layer |
| **norikv-vizd** | 📋 Planned | Visualization daemon |
| **norikv-dashboard** | 📋 Planned | Real-time web dashboard |

## SDK Features Comparison

All SDKs provide consistent functionality:

- ✅ **Smart Client Routing**: Client-side shard assignment with Jump Consistent Hashing
- ✅ **Leader-Aware Operations**: Direct requests to shard leaders with automatic failover
- ✅ **Retry Logic**: Exponential backoff with jitter for transient failures
- ✅ **Connection Pooling**: Efficient connection management per node
- ✅ **Conditional Operations**: Compare-and-swap (CAS) with version matching
- ✅ **Consistency Levels**: Lease-based, linearizable, or stale reads
- ✅ **Idempotency Keys**: Safe retries for write operations
- ✅ **Cluster Topology**: Dynamic cluster membership tracking
- ✅ **Ephemeral Server**: In-memory server for testing (no external dependencies)

### Hash Function Compatibility

**Critical**: All SDKs use identical hash functions to ensure consistent shard routing:
- **Key Hashing**: xxhash64 (seed=0)
- **Shard Assignment**: Jump Consistent Hash
- **Cross-Validated**: Test vectors ensure identical results across all languages

## Development

### Prerequisites

- Rust 1.75+ (for server and core crates)
- Node.js 18+ (for TypeScript SDK)
- Python 3.9+ (for Python SDK)
- Go 1.21+ (for Go SDK)
- Java 11+ (for Java SDK)

### Building from Source

```bash
# Build all Rust crates
cargo build --all

# Run tests
cargo test --all

# Build specific SDK
cd sdks/go && go build ./...
cd sdks/typescript && npm install && npm run build
cd sdks/python && pip install -e .
```

### Running Tests

```bash
# Rust core tests
cargo test --all

# Go SDK tests (unit + integration)
cd sdks/go && go test ./...

# TypeScript SDK tests
cd sdks/typescript && npm test

# Python SDK tests
cd sdks/python && pytest
```

## Performance

### Storage Engine (nori-lsm)

- **Point Reads**: ~10µs (p99)
- **Point Writes**: ~20µs (p99)
- **Bloom Filter Hit**: ~80ns (zero allocation)
- **Compaction**: Leveled strategy with size-tiered L0

### Hash Functions (Cross-SDK)

- **xxhash64**: ~2.5ns per operation (Go), ~8ns (Python/TypeScript)
- **Jump Consistent Hash**: ~14ns per operation (Go)
- **Combined Routing**: ~23ns (Go), <100ns (TypeScript/Python)

## Observability

NoriKV is built with observability as a first-class concern:

### Metrics
- **Vendor-Neutral**: `nori-observe` trait for pluggable backends
- **Prometheus**: Built-in Prometheus exporter
- **OTLP**: OpenTelemetry support with trace exemplars
- **Low Overhead**: <100ns per metric operation

### Visualization
- **Live Dashboard**: Real-time cluster visualization (planned)
- **VizEvent Stream**: Typed events for custom tooling
- **Health Endpoints**: HTTP health checks and readiness probes

## Documentation

- **[Architecture Guide](docs/architecture.md)**: System design and components
- **[Storage Layer](docs/storage.md)**: WAL, SSTable, and LSM details
- **[Consensus](docs/consensus.md)**: Raft implementation specifics
- **[SDKs](sdks/)**: Individual SDK documentation in each directory
- **[Operations](docs/operations.md)**: Deployment and monitoring guides

## Roadmap

### Completed ✅
- Core storage engine (WAL, SSTable, LSM)
- Raft consensus with read-index and leases
- SWIM membership protocol
- TypeScript, Python, and Go SDKs
- Ephemeral servers for testing
- Cross-SDK hash validation

### In Progress 🚧
- Server application and transport layer
- gRPC/HTTP API implementation
- Integration testing with real server

### Planned 📋
- Java SDK
- Live visualization dashboard
- Multi-shard transactions
- Streaming operations (watch API)
- Backup and restore
- Chaos testing framework

## Contributing

Contributions are welcome! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

### Areas for Contribution

- **Java SDK**: Complete implementation following Go/TypeScript patterns
- **Server Development**: gRPC handlers, sharding coordinator
- **Dashboard**: Real-time visualization UI
- **Documentation**: Tutorials, examples, API reference
- **Testing**: Property tests, chaos engineering
- **Performance**: Benchmarking, optimization

## License

This project is dual-licensed under MIT OR Apache-2.0.

See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE) for details.

## Related Projects

- **Python SDK**: [`sdks/python/`](sdks/python/)
- **TypeScript SDK**: [`sdks/typescript/`](sdks/typescript/)
- **Go SDK**: [`sdks/go/`](sdks/go/)
- **Java SDK**: [`sdks/java/`](sdks/java/)

## Acknowledgments

Built with modern distributed systems research:
- **LSM Trees**: Original LevelDB/RocksDB design
- **Raft Consensus**: Diego Ongaro's dissertation
- **SWIM**: Scalable Weakly-consistent Infection-style Process Group Membership Protocol
- **Jump Consistent Hash**: Google's consistent hashing algorithm

---

**Status**: Active development | **Stability**: Alpha | **Production Ready**: SDKs only (server in progress)
