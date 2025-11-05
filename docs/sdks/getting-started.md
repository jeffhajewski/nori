---
layout: default
title: Getting Started
parent: Client SDKs
nav_order: 1
---

# Getting Started with NoriKV Client SDKs

Quick start guide for all NoriKV client SDKs.

## Installation

Choose your preferred language and follow the installation instructions:

### Java

```xml
<!-- Maven -->
<dependency>
  <groupId>com.norikv</groupId>
  <artifactId>norikv-client</artifactId>
  <version>0.1.0</version>
</dependency>
```

```groovy
// Gradle
implementation 'com.norikv:norikv-client:0.1.0'
```

### Go

```bash
go get github.com/norikv/norikv-go
```

### TypeScript

```bash
npm install @norikv/client
# or
yarn add @norikv/client
```

### Python

```bash
pip install norikv
```

## Quick Start Examples

### Java

```java
import com.norikv.client.*;

public class Example {
    public static void main(String[] args) {
        ClientConfig config = ClientConfig.builder()
            .nodes(List.of("localhost:9001", "localhost:9002"))
            .totalShards(1024)
            .timeout(5000)
            .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            // Connect
            client.connect();

            // Put a value
            byte[] key = "user:alice".getBytes(StandardCharsets.UTF_8);
            byte[] value = "Alice".getBytes(StandardCharsets.UTF_8);
            Version version = client.put(key, value, null);
            System.out.println("Written at version: " + version);

            // Get the value
            GetResult result = client.get(key, null);
            String retrieved = new String(result.getValue(), StandardCharsets.UTF_8);
            System.out.println("Value: " + retrieved);

            // Delete
            boolean deleted = client.delete(key, null);
            System.out.println("Deleted: " + deleted);

        } catch (Exception e) {
            e.printStackTrace();
        }
    }
}
```

### Go

```go
package main

import (
    "context"
    "fmt"
    "log"
    "time"

    norikv "github.com/norikv/norikv-go"
)

func main() {
    config := norikv.ClientConfig{
        Nodes:       []string{"localhost:9001", "localhost:9002"},
        TotalShards: 1024,
        Timeout:     5 * time.Second,
    }

    client, err := norikv.NewClient(config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    ctx := context.Background()

    // Put a value
    key := []byte("user:alice")
    value := []byte("Alice")
    version, err := client.Put(ctx, key, value, nil)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Written at version: %v\n", version)

    // Get the value
    result, err := client.Get(ctx, key, nil)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Value: %s\n", result.Value)

    // Delete
    deleted, err := client.Delete(ctx, key, nil)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Deleted: %v\n", deleted)
}
```

### TypeScript

```typescript
import { NoriKVClient, bytesToString } from '@norikv/client';

async function main() {
    const client = new NoriKVClient({
        nodes: ['localhost:9001', 'localhost:9002'],
        totalShards: 1024,
        timeout: 5000,
    });

    await client.connect();

    try {
        // Put a value
        const version = await client.put('user:alice', 'Alice');
        console.log(`Written at version: ${version}`);

        // Get the value
        const result = await client.get('user:alice');
        console.log(`Value: ${bytesToString(result.value)}`);

        // Delete
        const deleted = await client.delete('user:alice');
        console.log(`Deleted: ${deleted}`);

    } finally {
        await client.close();
    }
}

main().catch(console.error);
```

### Python

```python
import asyncio
from norikv import NoriKVClient, ClientConfig

async def main():
    config = ClientConfig(
        nodes=["localhost:9001", "localhost:9002"],
        total_shards=1024,
        timeout=5000,
    )

    async with NoriKVClient(config) as client:
        # Put a value
        version = await client.put("user:alice", b"Alice")
        print(f"Written at version: {version}")

        # Get the value
        result = await client.get("user:alice")
        print(f"Value: {result.value.decode()}")

        # Delete
        deleted = await client.delete("user:alice")
        print(f"Deleted: {deleted}")

if __name__ == "__main__":
    asyncio.run(main())
```

## Core Concepts

### Client Configuration

All SDKs require:
- **nodes**: List of cluster node addresses (host:port)
- **totalShards**: Total number of virtual shards (must match cluster config, typically 1024)
- **timeout**: Request timeout in milliseconds (default: 5000)

Optional:
- **retry**: Retry policy configuration for transient failures

### Operations

All SDKs support three core operations:

1. **PUT**: Write a key-value pair
   - Returns a `Version` for the written value
   - Supports optional TTL, idempotency keys, and CAS

2. **GET**: Read a value by key
   - Returns the value and its version
   - Supports consistency level selection

3. **DELETE**: Remove a key
   - Returns boolean indicating if key was deleted
   - Supports idempotency keys and CAS

### Resource Management

| Language | Pattern | Example |
|----------|---------|---------|
| Java | try-with-resources | `try (NoriKVClient client = ...) { }` |
| Go | defer | `defer client.Close()` |
| TypeScript | async close | `await client.close()` or context manager |
| Python | async with | `async with NoriKVClient(...) as client:` |

## Configuration Examples

### With Retry Policy

#### Java
```java
ClientConfig config = ClientConfig.builder()
    .nodes(List.of("localhost:9001"))
    .totalShards(1024)
    .retry(RetryConfig.builder()
        .maxAttempts(10)
        .initialDelayMs(100)
        .maxDelayMs(5000)
        .build())
    .build();
```

#### Go
```go
config := norikv.ClientConfig{
    Nodes:       []string{"localhost:9001"},
    TotalShards: 1024,
    Retry: &norikv.RetryConfig{
        MaxAttempts:     10,
        InitialDelayMs:  100,
        MaxDelayMs:      5000,
    },
}
```

#### TypeScript
```typescript
const client = new NoriKVClient({
    nodes: ['localhost:9001'],
    totalShards: 1024,
    retry: {
        maxAttempts: 10,
        initialDelayMs: 100,
        maxDelayMs: 5000,
    },
});
```

#### Python
```python
from norikv import RetryConfig

config = ClientConfig(
    nodes=["localhost:9001"],
    total_shards=1024,
    retry=RetryConfig(
        max_attempts=10,
        initial_delay_ms=100,
        max_delay_ms=5000,
    ),
)
```

## Advanced Operations

### Compare-And-Swap (CAS)

Optimistic concurrency control using version matching:

#### Java
```java
GetResult current = client.get(key, null);
PutOptions options = PutOptions.builder()
    .ifMatchVersion(current.getVersion())
    .build();
client.put(key, newValue, options);
```

#### Go
```go
result, _ := client.Get(ctx, key, nil)
options := &norikv.PutOptions{
    IfMatchVersion: result.Version,
}
client.Put(ctx, key, newValue, options)
```

#### TypeScript
```typescript
const result = await client.get(key);
await client.put(key, newValue, {
    ifMatchVersion: result.version,
});
```

#### Python
```python
result = await client.get(key)
await client.put(key, new_value, PutOptions(
    if_match_version=result.version,
))
```

### Time-To-Live (TTL)

Automatic expiration:

#### Java
```java
PutOptions options = PutOptions.builder()
    .ttlMs(60000)  // 60 seconds
    .build();
client.put(key, value, options);
```

#### Go
```go
options := &norikv.PutOptions{
    TtlMs: 60000,  // 60 seconds
}
client.Put(ctx, key, value, options)
```

#### TypeScript
```typescript
await client.put(key, value, {
    ttlMs: 60000,  // 60 seconds
});
```

#### Python
```python
await client.put(key, value, PutOptions(
    ttl_ms=60000,  # 60 seconds
))
```

### Idempotency Keys

Safe retries:

#### Java
```java
PutOptions options = PutOptions.builder()
    .idempotencyKey("order-12345")
    .build();
client.put(key, value, options);
```

#### Go
```go
options := &norikv.PutOptions{
    IdempotencyKey: "order-12345",
}
client.Put(ctx, key, value, options)
```

#### TypeScript
```typescript
await client.put(key, value, {
    idempotencyKey: 'order-12345',
});
```

#### Python
```python
await client.put(key, value, PutOptions(
    idempotency_key="order-12345",
))
```

## Consistency Levels

All SDKs support three consistency levels for reads:

| Level | Description | Use Case |
|-------|-------------|----------|
| LEASE (default) | Fast lease-based read | Most operations |
| LINEARIZABLE | Strictest consistency | Critical reads |
| STALE_OK | May return stale data | High-throughput reads |

#### Java
```java
GetOptions options = GetOptions.builder()
    .consistency(ConsistencyLevel.LINEARIZABLE)
    .build();
client.get(key, options);
```

#### Go
```go
options := &norikv.GetOptions{
    Consistency: norikv.ConsistencyLinearizable,
}
client.Get(ctx, key, options)
```

#### TypeScript
```typescript
await client.get(key, {
    consistency: ConsistencyLevel.LINEARIZABLE,
});
```

#### Python
```python
await client.get(key, GetOptions(
    consistency=ConsistencyLevel.LINEARIZABLE,
))
```

## Error Handling

All SDKs provide typed errors:

#### Java
```java
try {
    GetResult result = client.get(key, null);
} catch (KeyNotFoundException e) {
    // Key not found
} catch (VersionMismatchException e) {
    // CAS conflict
} catch (ConnectionException e) {
    // Connection error
}
```

#### Go
```go
result, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrKeyNotFound) {
    // Key not found
} else if errors.Is(err, norikv.ErrVersionMismatch) {
    // CAS conflict
} else if errors.Is(err, norikv.ErrConnection) {
    // Connection error
}
```

#### TypeScript
```typescript
try {
    const result = await client.get(key);
} catch (err) {
    if (err instanceof KeyNotFoundError) {
        // Key not found
    } else if (err instanceof VersionMismatchError) {
        // CAS conflict
    } else if (err instanceof ConnectionError) {
        // Connection error
    }
}
```

#### Python
```python
try:
    result = await client.get(key)
except KeyNotFoundError:
    # Key not found
except VersionMismatchError:
    # CAS conflict
except ConnectionError:
    # Connection error
```

## Next Steps

### SDK-Specific Documentation

- [Java SDK Guide](./java/API_GUIDE.html)
- [Go SDK Guide](./go/API_GUIDE.html)
- [TypeScript SDK Guide](./typescript/API_GUIDE.html)
- [Python SDK Guide](./python/API_GUIDE.html)

### Cross-SDK Topics

- [Hash Compatibility](./hash-compatibility.html) - Cross-SDK hash validation
- [Error Reference](./error-reference.html) - Unified error codes
- [SDK Comparison](./index.html#comparison) - Feature comparison

### Common Patterns

All SDKs have comprehensive Advanced Patterns guides covering:
- Distributed Counter
- Session Management
- Inventory Management
- Caching Layer
- Rate Limiting
- Leader Election
- Event Sourcing
- Multi-Tenancy

See each SDK's ADVANCED_PATTERNS guide for language-specific implementations.
