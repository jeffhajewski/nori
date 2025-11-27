# NoriKV Client SDKs

Official client libraries for accessing NoriKV in your preferred programming language.

## Available SDKs

All SDKs provide the same core functionality with language-specific idioms and optimizations.

| SDK | Status | Tests | Documentation | Best For |
|-----|--------|-------|---------------|----------|
| [Java](./java/) |  Production Ready | 123/123 (100%) | Excellent | Enterprise apps, Android |
| [Go](./go/) |  Production Ready | 102+ passing | Excellent | High-performance services |
| [TypeScript](./typescript/) |  Production Ready | 100+ passing | Good | Node.js, web apps |
| [Python](./python/) |  Production Ready | 40 passing | Good | Data science, scripting |

## Quick Start by Language

### Java

```java
// Maven: com.norikv:norikv-client:0.1.0
try (NoriKVClient client = new NoriKVClient(config)) {
    Version version = client.put(key, value, null);
    GetResult result = client.get(key, null);
}
```

**Features:**
- Builder patterns for configuration
- Try-with-resources support
- Comprehensive Javadoc
- Maven Central distribution
- 4 comprehensive guides (API, Architecture, Troubleshooting, Advanced Patterns)

[→ Java SDK Documentation](./java/)

### Go

```go
// go get github.com/norikv/norikv-go
client, _ := norikv.NewClient(ctx, config)
defer client.Close()

version, _ := client.Put(ctx, key, value, nil)
result, _ := client.Get(ctx, key, nil)
```

**Features:**
- Context-aware operations
- Zero-allocation routing (23ns/op)
- Goroutine-safe
- Single-flight leader discovery
- 4 comprehensive guides (API, Architecture, Troubleshooting, Advanced Patterns)

[→ Go SDK Documentation](./go/)

### TypeScript

```typescript
// npm install @norikv/client
const client = new NoriKVClient(config);
await client.connect();

await client.put(key, value);
const result = await client.get(key);
```

**Features:**
- Full TypeScript types
- Async/await API
- ES modules + CommonJS
- Browser compatible
- Comprehensive inline documentation

[→ TypeScript SDK Documentation](./typescript/)

### Python

```python
# pip install norikv
async with NoriKVClient(config) as client:
    version = await client.put(key, value)
    result = await client.get(key)
```

**Features:**
- Asyncio-based
- Type hints throughout
- Context managers
- Pythonic API
- Works with Python 3.9+

[→ Python SDK Documentation](./python/)

## Common Features

All SDKs support:

 **Smart Routing** - Direct requests to shard leaders
 **Automatic Retries** - Exponential backoff with jitter
 **Connection Pooling** - Efficient resource management
 **CAS Operations** - Compare-and-swap for optimistic locking
 **TTL Support** - Automatic key expiration
 **Consistency Levels** - Lease, linearizable, or stale reads
 **Idempotency Keys** - Safe retry semantics
 **Topology Tracking** - React to cluster changes

## Choosing an SDK

### By Use Case

**Enterprise Applications**
→ Use **Java SDK** for mature ecosystem, excellent tooling, and comprehensive documentation

**High-Performance Services**
→ Use **Go SDK** for zero-allocation hot paths and native concurrency

**Web Applications**
→ Use **TypeScript SDK** for type safety and seamless integration with Node.js/browsers

**Data Science & Scripting**
→ Use **Python SDK** for ease of use and rich data ecosystem

### By Performance Requirements

**Highest Throughput**
1. Go (zero-allocation routing, ~2.5ns hashing)
2. Java (JIT-optimized, mature GC)
3. TypeScript (V8-optimized)
4. Python (GIL limitations)

**Lowest Latency**
1. Go (native compilation, efficient runtime)
2. Java (after JIT warmup)
3. TypeScript (V8 optimization)
4. Python (interpreted overhead)

**Lowest Memory**
1. Go (efficient memory model)
2. Java (after optimization)
3. TypeScript (V8 garbage collection)
4. Python (higher overhead)

## Cross-SDK Compatibility

All SDKs use **identical hashing algorithms** to ensure compatibility:

- **XXHash64** (seed=0) for key hashing
- **Jump Consistent Hash** for shard assignment
- **1024 virtual shards** by default

This means you can:
- Use different SDKs for different services
- Migrate between languages without data migration
- Mix languages in the same cluster

[→ Hash Compatibility Guide](./hash-compatibility.md)

## Installation & Setup

Each SDK has detailed installation instructions:

- [Java SDK Installation](./java/#installation)
- [Go SDK Installation](./go/#installation)
- [TypeScript SDK Installation](./typescript/#installation)
- [Python SDK Installation](./python/#installation)

## Getting Help

- **Issues**: [GitHub Issues](https://github.com/j-haj/nori/issues)
- **Discussions**: [GitHub Discussions](https://github.com/j-haj/nori/discussions)
- **Documentation**: This site and SDK-specific guides

## Contributing

Each SDK welcomes contributions:

- Bug reports and feature requests via GitHub Issues
- Code contributions via Pull Requests
- Documentation improvements
- Test coverage expansion

See the main [CONTRIBUTING.md](https://github.com/j-haj/nori/blob/main/CONTRIBUTING.md) for guidelines.

---

{: .note }
> All SDKs are production-ready and maintained. Choose based on your language ecosystem and performance requirements.
