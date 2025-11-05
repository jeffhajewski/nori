---
layout: default
title: Home
nav_order: 1
description: "NoriKV Documentation - Sharded, Raft-replicated, log-structured key-value store"
permalink: /
---

# NoriKV Documentation

Welcome to the comprehensive documentation for **NoriKV** - a sharded, Raft-replicated, log-structured key-value store with first-class observability.

## What is NoriKV?

NoriKV is a distributed key-value store designed for:
- **High availability** through Raft consensus
- **Horizontal scalability** via consistent hashing and sharding
- **Performance** with LSM-tree storage engine
- **Observability** with first-class metrics and tracing
- **Multi-language support** with SDKs for Java, Go, TypeScript, and Python

## Quick Navigation

### 🚀 [Client SDKs](./sdks/)
Get started with NoriKV in your preferred language:
- **[Java SDK](./sdks/java/)** - Production-ready with 123 tests, comprehensive guides
- **[Go SDK](./sdks/go/)** - High-performance with zero-allocation routing
- **[TypeScript SDK](./sdks/typescript/)** - Full type safety, async/await
- **[Python SDK](./sdks/python/)** - Asyncio-based, type hints

### 📚 Architecture & Internals
Understand how NoriKV works:
- **Storage Engine** - LSM-tree with WAL, SSTables, and compaction
- **Consensus** - Raft protocol with read-index and leases
- **Membership** - SWIM-based failure detection
- **Sharding** - Jump consistent hashing with 1024 virtual shards

### 🔧 Operations
Deploy and manage NoriKV:
- **Installation** - Getting started guides
- **Configuration** - Tuning parameters
- **Monitoring** - Metrics and observability
- **Troubleshooting** - Common issues and solutions

## Features

### Storage & Performance
- **LSM-tree storage** with write-ahead log (WAL)
- **SSTable format** with bloom filters and compression
- **Automatic compaction** with configurable strategies
- **Snapshots** for fast recovery and backups

### Distributed Consensus
- **Raft consensus** for strong consistency
- **Read-index protocol** for linearizable reads
- **Leader leases** for fast reads without quorum
- **Joint consensus** for safe membership changes

### Client Features
- **Smart routing** - Direct requests to shard leaders
- **Automatic retries** with exponential backoff
- **Connection pooling** for efficient resource use
- **CAS operations** for optimistic concurrency control
- **TTL support** for automatic expiration
- **Consistency levels** - Choose between speed and consistency

## Getting Started

### Choose Your SDK

| Language | Status | Tests | Documentation |
|----------|--------|-------|---------------|
| [Java](./sdks/java/) |  Production | 123/123 | Excellent |
| [Go](./sdks/go/) |  Production | 102+ | Excellent |
| [TypeScript](./sdks/typescript/) |  Production | 100+ | Good |
| [Python](./sdks/python/) |  Production | 40 | Good |

### Installation

Choose your preferred language and follow the SDK-specific guides:

**Java (Maven)**
```xml
<dependency>
    <groupId>com.norikv</groupId>
    <artifactId>norikv-client</artifactId>
    <version>0.1.0</version>
</dependency>
```

**Go**
```bash
go get github.com/norikv/norikv-go
```

**TypeScript/JavaScript**
```bash
npm install @norikv/client
```

**Python**
```bash
pip install norikv
```

### Quick Example

**Java**
```java
try (NoriKVClient client = new NoriKVClient(config)) {
    Version version = client.put(key, value, null);
    GetResult result = client.get(key, null);
    client.delete(key, null);
}
```

**Go**
```go
client, _ := norikv.NewClient(ctx, config)
defer client.Close()

version, _ := client.Put(ctx, key, value, nil)
result, _ := client.Get(ctx, key, nil)
client.Delete(ctx, key, nil)
```

**TypeScript**
```typescript
const client = new NoriKVClient(config);
await client.connect();

await client.put(key, value);
const result = await client.get(key);
await client.delete(key);
```

**Python**
```python
async with NoriKVClient(config) as client:
    version = await client.put(key, value)
    result = await client.get(key)
    await client.delete(key)
```

## Documentation Structure

This documentation is organized into several sections:

- **[SDKs](./sdks/)** - Client library documentation for all languages
  - API references, architecture guides, troubleshooting
  - Advanced patterns and real-world examples

- **Architecture** - Internal design and implementation
  - Storage engine details
  - Consensus protocol
  - Membership and failure detection

- **Operations** - Deployment and management
  - Installation guides
  - Configuration reference
  - Monitoring and metrics
  - Troubleshooting

## Support & Contributing

- **GitHub Repository**: [github.com/j-haj/nori](https://github.com/j-haj/nori)
- **Issue Tracker**: [GitHub Issues](https://github.com/j-haj/nori/issues)
- **Contributing**: See [CONTRIBUTING.md](https://github.com/j-haj/nori/blob/main/CONTRIBUTING.md)

## License

NoriKV is distributed under the **MIT OR Apache-2.0** license.

---

<div class="callout note">
  <p><strong>Note:</strong> This documentation is for NoriKV version 0.1.x. For older versions, see the <a href="https://github.com/j-haj/nori/releases">releases page</a>.</p>
</div>
