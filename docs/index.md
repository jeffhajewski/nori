---
hide:
  - navigation
  - toc
---

# NoriKV

<div class="hero" markdown>

## Distributed KV Store Built for Scale

A **sharded, Raft-replicated, log-structured key-value store** with first-class observability and multi-language SDKs.

[Get Started](sdks/getting-started.md){ .md-button .md-button--primary }
[View on GitHub](https://github.com/jeffhajewski/norikv){ .md-button }

</div>

---

<div class="grid cards" markdown>

-   :material-server-network:{ .lg .middle } **Distributed by Design**

    ---

    Raft consensus for strong consistency with automatic leader election and failover. No single point of failure.

    [:octicons-arrow-right-24: Architecture](architecture/index.md)

-   :material-chart-line:{ .lg .middle } **LSM-Tree Storage**

    ---

    High-performance log-structured merge tree with WAL, SSTables, bloom filters, and automatic compaction.

    [:octicons-arrow-right-24: Storage Crates](crates/index.md)

-   :material-scale-balance:{ .lg .middle } **Smart Sharding**

    ---

    Jump consistent hashing with 1024 virtual shards. Client-side routing directly to shard leaders.

    [:octicons-arrow-right-24: Multi-Shard Server](architecture/multi-shard-server.md)

-   :material-language-python:{ .lg .middle } **Multi-Language SDKs**

    ---

    Production-ready clients for Java, Go, TypeScript, and Python with identical APIs and behavior.

    [:octicons-arrow-right-24: Client SDKs](sdks/index.md)

</div>

---

## Quick Example

=== "Java"

    ```java
    ClientConfig config = ClientConfig.builder()
        .nodes("localhost:9001", "localhost:9002", "localhost:9003")
        .totalShards(1024)
        .build();

    try (NoriKVClient client = new NoriKVClient(config)) {
        // Put a value
        Version version = client.put("user:123".getBytes(),
            "{\"name\":\"Alice\"}".getBytes(), null);

        // Get the value
        GetResult result = client.get("user:123".getBytes(), null);
        System.out.println(new String(result.getValue()));

        // Conditional update (CAS)
        PutOptions options = PutOptions.builder()
            .ifMatchVersion(version)
            .build();
        client.put("user:123".getBytes(), newValue, options);
    }
    ```

=== "Go"

    ```go
    config := norikv.ClientConfig{
        Nodes:       []string{"localhost:9001", "localhost:9002", "localhost:9003"},
        TotalShards: 1024,
    }

    client, err := norikv.NewClient(ctx, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Put a value
    version, err := client.Put(ctx, []byte("user:123"),
        []byte(`{"name":"Alice"}`), nil)

    // Get the value
    result, err := client.Get(ctx, []byte("user:123"), nil)
    fmt.Println(string(result.Value))

    // Conditional update (CAS)
    _, err = client.Put(ctx, []byte("user:123"), newValue,
        &norikv.PutOptions{IfMatchVersion: version})
    ```

=== "TypeScript"

    ```typescript
    const client = new NoriKVClient({
      nodes: ['localhost:9001', 'localhost:9002', 'localhost:9003'],
      totalShards: 1024,
    });

    await client.connect();

    // Put a value
    const version = await client.put('user:123', '{"name":"Alice"}');

    // Get the value
    const result = await client.get('user:123');
    console.log(bytesToString(result.value));

    // Conditional update (CAS)
    await client.put('user:123', newValue, { ifMatchVersion: version });

    await client.close();
    ```

=== "Python"

    ```python
    config = ClientConfig(
        nodes=["localhost:9001", "localhost:9002", "localhost:9003"],
        total_shards=1024,
    )

    async with NoriKVClient(config) as client:
        # Put a value
        version = await client.put("user:123", '{"name":"Alice"}')

        # Get the value
        result = await client.get("user:123")
        print(result.value.decode())

        # Conditional update (CAS)
        await client.put("user:123", new_value,
            PutOptions(if_match_version=version))
    ```

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                          NoriKV Cluster                              │
├─────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐              │
│  │    Node 1    │  │    Node 2    │  │    Node 3    │              │
│  │   (Leader)   │  │  (Follower)  │  │  (Follower)  │              │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘              │
│         └─────────────────┼─────────────────┘                       │
│                           │ Raft                                    │
│         ┌─────────────────┼─────────────────┐                       │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────▼───────┐              │
│  │  LSM Engine  │  │  LSM Engine  │  │  LSM Engine  │              │
│  │  WAL + SST   │  │  WAL + SST   │  │  WAL + SST   │              │
│  └──────────────┘  └──────────────┘  └──────────────┘              │
└─────────────────────────────────────────────────────────────────────┘
                           ▲
                           │ gRPC
┌──────────────────────────┴──────────────────────────┐
│                    Smart Clients                     │
│  ┌─────────┐  ┌─────────┐  ┌──────────┐  ┌────────┐ │
│  │  Java   │  │   Go    │  │TypeScript│  │ Python │ │
│  └─────────┘  └─────────┘  └──────────┘  └────────┘ │
└─────────────────────────────────────────────────────┘
```

[:octicons-arrow-right-24: Learn more about Architecture](architecture/index.md)

---

## Design Principles

<div class="grid" markdown>

!!! info "Strong Consistency"
    Raft consensus ensures linearizable operations. Every write is replicated to a quorum before acknowledgment.

!!! success "Smart Routing"
    Clients route directly to shard leaders. No proxy layer bottleneck. Automatic retry on failover.

!!! tip "Observable"
    First-class Prometheus metrics, structured logging, and distributed tracing support.

!!! example "Production-Ready"
    Comprehensive test suites across all SDKs. Battle-tested at scale.

</div>

---

## Choose Your SDK

| Language | Status | Tests | Key Features |
|----------|--------|-------|--------------|
| [**Java**](sdks/java/index.md) | Production | 123+ | Thread-safe, connection pooling |
| [**Go**](sdks/go/index.md) | Production | 102+ | Zero-allocation routing, context support |
| [**TypeScript**](sdks/typescript/index.md) | Production | 100+ | Full type safety, async/await |
| [**Python**](sdks/python/index.md) | Production | 40+ | Asyncio-based, type hints |

---

## Get Started

<div class="grid cards" markdown>

-   :material-rocket-launch:{ .lg } **Quick Start**

    Get up and running in 5 minutes with a simple key-value example.

    [:octicons-arrow-right-24: Quick Start](sdks/getting-started.md)

-   :material-book-open-variant:{ .lg } **SDK Documentation**

    Comprehensive guides for Java, Go, TypeScript, and Python clients.

    [:octicons-arrow-right-24: Client SDKs](sdks/index.md)

-   :material-cog:{ .lg } **Operations Guide**

    Deploy, configure, and monitor your NoriKV cluster.

    [:octicons-arrow-right-24: Operations](operations/index.md)

</div>
