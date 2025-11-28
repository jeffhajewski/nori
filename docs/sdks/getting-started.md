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

## Vector Search

NoriKV supports vector similarity search for building AI/ML applications, recommendation systems, and semantic search.

### Creating a Vector Index

Before inserting vectors, create an index with your desired configuration:

#### Java
```java
boolean created = client.vectorCreateIndex(
    "embeddings",      // namespace
    1536,              // dimensions
    DistanceFunction.COSINE,
    VectorIndexType.HNSW,
    null               // options
);
```

#### Go
```go
created, err := client.VectorCreateIndex(
    ctx,
    "embeddings",           // namespace
    1536,                   // dimensions
    norikv.DistanceCosine,
    norikv.VectorIndexHNSW,
    nil,                    // options
)
```

#### TypeScript
```typescript
const created = await client.vectorCreateIndex(
    'embeddings',      // namespace
    1536,              // dimensions
    'cosine',          // distance function
    'hnsw'             // index type
);
```

#### Python
```python
created = await client.vector_create_index(
    "embeddings",                    # namespace
    1536,                            # dimensions
    DistanceFunction.COSINE,
    VectorIndexType.HNSW,
)
```

### Inserting Vectors

#### Java
```java
float[] embedding = getEmbedding("Hello world");
Version version = client.vectorInsert(
    "embeddings",
    "doc-123",
    embedding,
    null
);
```

#### Go
```go
embedding := getEmbedding("Hello world")
version, err := client.VectorInsert(
    ctx,
    "embeddings",
    "doc-123",
    embedding,
    nil,
)
```

#### TypeScript
```typescript
const embedding = await getEmbedding('Hello world');
const version = await client.vectorInsert(
    'embeddings',
    'doc-123',
    embedding
);
```

#### Python
```python
embedding = await get_embedding("Hello world")
version = await client.vector_insert(
    "embeddings",
    "doc-123",
    embedding,
)
```

### Searching for Similar Vectors

#### Java
```java
float[] query = getEmbedding("Find similar documents");
VectorSearchResult result = client.vectorSearch(
    "embeddings",
    query,
    10,    // k nearest neighbors
    VectorSearchOptions.builder()
        .includeVectors(true)
        .build()
);

for (VectorMatch match : result.getMatches()) {
    System.out.printf("ID: %s, Distance: %.4f%n",
        match.getId(), match.getDistance());
}
```

#### Go
```go
query := getEmbedding("Find similar documents")
result, err := client.VectorSearch(
    ctx,
    "embeddings",
    query,
    10,  // k nearest neighbors
    &norikv.VectorSearchOptions{
        IncludeVectors: true,
    },
)

for _, match := range result.Matches {
    fmt.Printf("ID: %s, Distance: %.4f\n", match.ID, match.Distance)
}
```

#### TypeScript
```typescript
const query = await getEmbedding('Find similar documents');
const result = await client.vectorSearch(
    'embeddings',
    query,
    10,  // k nearest neighbors
    { includeVectors: true }
);

for (const match of result.matches) {
    console.log(`ID: ${match.id}, Distance: ${match.distance}`);
}
```

#### Python
```python
query = await get_embedding("Find similar documents")
result = await client.vector_search(
    "embeddings",
    query,
    10,  # k nearest neighbors
    VectorSearchOptions(include_vectors=True),
)

for match in result.matches:
    print(f"ID: {match.id}, Distance: {match.distance}")
```

### Getting a Vector by ID

#### Java
```java
float[] vector = client.vectorGet("embeddings", "doc-123");
if (vector != null) {
    System.out.printf("Vector has %d dimensions%n", vector.length);
}
```

#### Go
```go
vector, err := client.VectorGet(ctx, "embeddings", "doc-123")
if err == nil && vector != nil {
    fmt.Printf("Vector has %d dimensions\n", len(vector))
}
```

#### TypeScript
```typescript
const vector = await client.vectorGet('embeddings', 'doc-123');
if (vector) {
    console.log(`Vector has ${vector.length} dimensions`);
}
```

#### Python
```python
vector = await client.vector_get("embeddings", "doc-123")
if vector:
    print(f"Vector has {len(vector)} dimensions")
```

### Deleting Vectors

#### Java
```java
boolean deleted = client.vectorDelete("embeddings", "doc-123", null);
```

#### Go
```go
deleted, err := client.VectorDelete(ctx, "embeddings", "doc-123", nil)
```

#### TypeScript
```typescript
const deleted = await client.vectorDelete('embeddings', 'doc-123');
```

#### Python
```python
deleted = await client.vector_delete("embeddings", "doc-123")
```

### Distance Functions

| Function | Description | Use Case |
|----------|-------------|----------|
| EUCLIDEAN | L2 distance | General purpose |
| COSINE | Cosine similarity (1 - cos) | Text embeddings, normalized vectors |
| INNER_PRODUCT | Negative inner product | Maximum inner product search |

### Index Types

| Type | Description | Trade-off |
|------|-------------|-----------|
| BRUTE_FORCE | Exact linear scan | Exact results, O(n) complexity |
| HNSW | Hierarchical Navigable Small World | Approximate, O(log n) complexity |

---

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

- [Java SDK Guide](./java/API_GUIDE.md)
- [Go SDK Guide](./go/API_GUIDE.md)
- [TypeScript SDK Guide](./typescript/API_GUIDE.md)
- [Python SDK Guide](./python/API_GUIDE.md)

### Cross-SDK Topics

- [Hash Compatibility](./hash-compatibility.md) - Cross-SDK hash validation
- [Error Reference](./error-reference.md) - Unified error codes
- [SDK Comparison](./index.md#comparison) - Feature comparison

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
