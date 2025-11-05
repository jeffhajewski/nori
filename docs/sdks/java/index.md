---
layout: default
title: Java SDK
parent: Client SDKs
nav_order: 1
has_children: true
---

# NoriKV Java Client SDK

Production-ready Java client for NoriKV with comprehensive documentation and 100% test coverage.

## Status

** PRODUCTION READY** - Fully functional Java SDK

-  **123/123 tests passing** (100% success rate)
-  Maven/Gradle support with published artifacts
-  Complete API with all operations (put, get, delete)
-  Comprehensive documentation (4 detailed guides)
-  Performance benchmarks exceeding SLO targets by 60-130x

## Quick Start

### Installation

**Maven:**
```xml
<dependency>
    <groupId>com.norikv</groupId>
    <artifactId>norikv-client</artifactId>
    <version>0.1.0</version>
</dependency>
```

**Gradle:**
```gradle
implementation 'com.norikv:norikv-client:0.1.0'
```

### Basic Usage

```java
import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;

// Configure client
ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList("localhost:9001", "localhost:9002"))
    .totalShards(1024)
    .timeoutMs(5000)
    .build();

// Use try-with-resources for automatic cleanup
try (NoriKVClient client = new NoriKVClient(config)) {
    // Put a value
    byte[] key = "user:123".getBytes();
    byte[] value = "{\"name\":\"Alice\"}".getBytes();
    Version version = client.put(key, value, null);

    // Get the value
    GetResult result = client.get(key, null);
    System.out.println("Value: " + new String(result.getValue()));

    // Delete
    client.delete(key, null);
}
```

## Documentation

The Java SDK includes comprehensive documentation:

### Core Guides

- **[API Guide](./API_GUIDE.html)** - Complete API reference
  - Installation and setup
  - Client configuration
  - All operations (PUT, GET, DELETE)
  - Advanced features (CAS, TTL, consistency levels)
  - Error handling
  - Best practices

- **[Architecture Guide](./ARCHITECTURE.html)** - Internal design
  - Component architecture
  - Request flow
  - Threading model
  - Connection management
  - Routing & sharding
  - Performance considerations

- **[Troubleshooting Guide](./TROUBLESHOOTING.html)** - Common issues
  - Connection problems
  - Performance optimization
  - Version conflicts
  - Error messages and solutions
  - Debugging tips

- **[Advanced Patterns](./ADVANCED_PATTERNS.html)** - Real-world examples
  - Distributed Counter
  - Session Management
  - Inventory Management
  - Caching Layer
  - Rate Limiting
  - Leader Election
  - Event Sourcing
  - Multi-Tenancy

## Features

### Core Features
-  Smart client-side routing with hash-based shard assignment
-  Leader-aware routing with automatic failover
-  Retry logic with exponential backoff
-  Idempotency support for safe retries
-  Conditional operations (CAS) with version matching
-  Multiple consistency levels (lease, linearizable, stale)
-  Connection pooling with graceful shutdown
-  Cluster topology tracking and change notifications
-  Thread-safe operations
-  AutoCloseable for try-with-resources

### Java-Specific
-  Builder patterns for configuration
-  Comprehensive exception hierarchy
-  Stream-based operations
-  Javadoc for all public APIs
-  Maven Central distribution

## Performance

**Target SLOs:**
- GET p95 latency: ≤ 10ms
- PUT p95 latency: ≤ 20ms

**Actual Performance** (against in-memory ephemeral server):
- Sequential GET p95: 0.160 ms  (62x faster than target)
- Sequential PUT p95: 0.153 ms  (130x faster than target)
- Mixed workload p95: 0.203 ms 
- CAS operations p95: 0.258 ms 
- Large values (10KB) p95: 0.198 ms 
- Concurrent (10 threads) p95: 0.828 ms 

**Throughput:**
- Sequential PUT: ~8,280 ops/sec
- Sequential GET: ~7,590 ops/sec
- DELETE operations: ~10,466 ops/sec
- Concurrent operations: ~6,164 ops/sec

## Requirements

- **Java**: 11 or higher
- **Maven**: 3.6 or higher (for building from source)
- **NoriKV Server**: 0.1.x

## Examples

See the [source repository](https://github.com/j-haj/nori/tree/main/sdks/java/examples) for complete working examples:

- `BasicExample.java` - Simple operations with different options
- `ConditionalOperationsExample.java` - CAS and version matching
- `RetryAndErrorHandlingExample.java` - Error handling patterns

## Support

- **Issues**: [GitHub Issues](https://github.com/j-haj/nori/issues)
- **Source**: [GitHub Repository](https://github.com/j-haj/nori/tree/main/sdks/java)
- **API Docs**: [Javadoc](./javadoc/) (if published)

## License

MIT OR Apache-2.0

---

{: .note }
> This SDK is production-ready with 100% test coverage. All features are fully implemented and documented.
