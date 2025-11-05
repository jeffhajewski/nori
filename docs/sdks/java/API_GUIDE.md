---
layout: default
title: API Guide
parent: Java SDK
grand_parent: Client SDKs
nav_order: 1
---

# NoriKV Java Client API Guide

Complete reference for the NoriKV Java Client SDK.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Client Configuration](#client-configuration)
- [Core Operations](#core-operations)
- [Advanced Features](#advanced-features)
- [Error Handling](#error-handling)
- [Best Practices](#best-practices)

## Installation

### Maven

```xml
<dependency>
    <groupId>com.norikv</groupId>
    <artifactId>norikv-client</artifactId>
    <version>0.1.0</version>
</dependency>
```

### Gradle

```gradle
implementation 'com.norikv:norikv-client:0.1.0'
```

## Quick Start

```java
import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;
import java.util.Arrays;

public class QuickStart {
    public static void main(String[] args) throws NoriKVException {
        // Configure client
        ClientConfig config = ClientConfig.builder()
            .nodes(Arrays.asList("localhost:9001", "localhost:9002", "localhost:9003"))
            .totalShards(1024)
            .timeoutMs(5000)
            .build();

        // Use try-with-resources for automatic cleanup
        try (NoriKVClient client = new NoriKVClient(config)) {
            // Write
            byte[] key = "user:alice".getBytes();
            byte[] value = "{\"name\":\"Alice\",\"age\":30}".getBytes();
            Version version = client.put(key, value, null);

            // Read
            GetResult result = client.get(key, null);
            System.out.println("Value: " + new String(result.getValue()));

            // Delete
            client.delete(key, null);
        }
    }
}
```

## Client Configuration

### Basic Configuration

```java
ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList("node1:9001", "node2:9001"))
    .totalShards(1024)
    .timeoutMs(5000)
    .build();
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `nodes` | `List<String>` | **Required** | List of node addresses (host:port) |
| `totalShards` | `int` | **Required** | Total number of shards in cluster |
| `timeoutMs` | `long` | 5000 | Request timeout in milliseconds |
| `retry` | `RetryConfig` | See below | Retry policy configuration |

### Retry Configuration

```java
RetryConfig retryConfig = RetryConfig.builder()
    .maxAttempts(10)          // Max retry attempts
    .initialDelayMs(100)      // Initial backoff delay
    .maxDelayMs(5000)         // Maximum backoff delay
    .jitterMs(100)            // Random jitter to add
    .build();

ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList("localhost:9001"))
    .totalShards(1024)
    .retry(retryConfig)
    .build();
```

**Retry Behavior:**
- Retries transient errors: `UNAVAILABLE`, `ABORTED`, `DEADLINE_EXCEEDED`, `RESOURCE_EXHAUSTED`
- Does NOT retry: `INVALID_ARGUMENT`, `NOT_FOUND`, `FAILED_PRECONDITION`, `PERMISSION_DENIED`
- Uses exponential backoff with jitter to avoid thundering herd

### Default Configuration

```java
// Quick setup with defaults
ClientConfig config = ClientConfig.defaultConfig("localhost:9001");
```

## Core Operations

### PUT - Write Data

#### Basic PUT

```java
byte[] key = "user:123".getBytes();
byte[] value = "{\"name\":\"Alice\"}".getBytes();

Version version = client.put(key, value, null);
System.out.println("Written at version: " + version);
```

#### PUT with Options

```java
PutOptions options = PutOptions.builder()
    .ttlMs(60000L)                          // TTL: 60 seconds
    .idempotencyKey("order-12345")          // Idempotency key
    .ifMatchVersion(expectedVersion)        // CAS: only if version matches
    .build();

Version version = client.put(key, value, options);
```

**PutOptions Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `ttlMs` | `Long` | Time-to-live in milliseconds |
| `idempotencyKey` | `String` | Key for idempotent operations |
| `ifMatchVersion` | `Version` | Expected version for CAS |

### GET - Read Data

#### Basic GET

```java
byte[] key = "user:123".getBytes();
GetResult result = client.get(key, null);

byte[] value = result.getValue();
Version version = result.getVersion();
```

#### GET with Consistency Level

```java
GetOptions options = GetOptions.builder()
    .consistency(ConsistencyLevel.LINEARIZABLE)
    .build();

GetResult result = client.get(key, options);
```

**Consistency Levels:**

| Level | Description | Use Case |
|-------|-------------|----------|
| `LEASE` | Default, lease-based read | Most operations (fast, usually consistent) |
| `LINEARIZABLE` | Strictest, always up-to-date | Critical reads requiring absolute consistency |
| `STALE_OK` | May return stale data | Read-heavy workloads, caching |

### DELETE - Remove Data

#### Basic DELETE

```java
byte[] key = "user:123".getBytes();
boolean deleted = client.delete(key, null);

if (deleted) {
    System.out.println("Key was deleted");
} else {
    System.out.println("Key did not exist");
}
```

#### DELETE with Options

```java
DeleteOptions options = DeleteOptions.builder()
    .idempotencyKey("delete-order-12345")
    .ifMatchVersion(expectedVersion)
    .build();

boolean deleted = client.delete(key, options);
```

## Advanced Features

### Compare-And-Swap (CAS)

Optimistic concurrency control using version matching:

```java
// Read current value
GetResult current = client.get(key, null);
int value = Integer.parseInt(new String(current.getValue()));

// Update with CAS
int newValue = value + 1;
PutOptions options = PutOptions.builder()
    .ifMatchVersion(current.getVersion())
    .build();

try {
    client.put(key, String.valueOf(newValue).getBytes(), options);
    System.out.println("CAS succeeded");
} catch (VersionMismatchException e) {
    System.out.println("CAS failed - version changed");
}
```

### Idempotent Operations

Safe retries using idempotency keys:

```java
String idempotencyKey = "payment-" + UUID.randomUUID();

PutOptions options = PutOptions.builder()
    .idempotencyKey(idempotencyKey)
    .build();

// First attempt
Version v1 = client.put(key, value, options);

// Retry with same key (safe - returns same version)
Version v2 = client.put(key, value, options);

assert v1.equals(v2); // Same version returned
```

### Time-To-Live (TTL)

Automatic expiration:

```java
PutOptions options = PutOptions.builder()
    .ttlMs(60000L) // Expires in 60 seconds
    .build();

client.put(key, value, options);

// Key automatically deleted after TTL
Thread.sleep(61000);
try {
    client.get(key, null);
} catch (KeyNotFoundException e) {
    System.out.println("Key expired");
}
```

### Cluster Topology

Monitor cluster changes:

```java
// Get current cluster view
ClusterView view = client.getClusterView();
if (view != null) {
    System.out.println("Cluster epoch: " + view.getEpoch());
    System.out.println("Nodes: " + view.getNodes().size());
}

// Subscribe to topology changes
Runnable unsubscribe = client.onTopologyChange(event -> {
    System.out.println("Topology changed!");
    System.out.println("Previous epoch: " + event.getPreviousEpoch());
    System.out.println("Current epoch: " + event.getCurrentEpoch());
    System.out.println("Added nodes: " + event.getAddedNodes());
    System.out.println("Removed nodes: " + event.getRemovedNodes());
    System.out.println("Leader changes: " + event.getLeaderChanges());
});

// Later: unsubscribe
unsubscribe.run();
```

### Client Statistics

Monitor client performance:

```java
NoriKVClient.ClientStats stats = client.getStats();

System.out.println("Router stats: " + stats.getRouterStats());
System.out.println("Connection pool stats: " + stats.getPoolStats());
System.out.println("Topology stats: " + stats.getTopologyStats());
System.out.println("Client closed: " + stats.isClosed());
```

## Error Handling

### Exception Hierarchy

```
NoriKVException (base)
├── KeyNotFoundException
├── VersionMismatchException
├── AlreadyExistsException
└── ConnectionException
```

### Handling Specific Errors

```java
try {
    GetResult result = client.get(key, null);
} catch (KeyNotFoundException e) {
    // Key does not exist
    System.out.println("Key not found");
} catch (ConnectionException e) {
    // Network or cluster issues
    System.out.println("Connection error: " + e.getMessage());
} catch (NoriKVException e) {
    // Other errors
    System.out.println("Error: " + e.getCode() + " - " + e.getMessage());
}
```

### Retry Pattern

```java
int maxAttempts = 3;
for (int attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
        client.put(key, value, null);
        break; // Success
    } catch (ConnectionException e) {
        if (attempt == maxAttempts) {
            throw e; // Give up
        }
        // Exponential backoff
        Thread.sleep((long) Math.pow(2, attempt) * 100);
    }
}
```

### Graceful Degradation

```java
public byte[] getWithFallback(NoriKVClient client, byte[] key, byte[] defaultValue) {
    try {
        GetResult result = client.get(key, null);
        return result.getValue();
    } catch (KeyNotFoundException e) {
        return defaultValue;
    } catch (NoriKVException e) {
        logger.warn("Failed to get key, using default", e);
        return defaultValue;
    }
}
```

## Best Practices

### 1. Use Try-With-Resources

Always use try-with-resources for automatic cleanup:

```java
try (NoriKVClient client = new NoriKVClient(config)) {
    // Use client
} // Automatically closed
```

### 2. Reuse Client Instances

Clients are thread-safe and should be reused:

```java
//  Good: Single client instance
private final NoriKVClient client = new NoriKVClient(config);

//  Bad: Creating client per request
public void handleRequest() {
    try (NoriKVClient client = new NoriKVClient(config)) {
        // ...
    }
}
```

### 3. Use Idempotency Keys

For operations that must not be duplicated:

```java
String idempotencyKey = "order-" + orderId;
PutOptions options = PutOptions.builder()
    .idempotencyKey(idempotencyKey)
    .build();

client.put(key, value, options);
```

### 4. Choose Appropriate Consistency

- Use `LEASE` (default) for most operations
- Use `LINEARIZABLE` for critical reads
- Use `STALE_OK` for caching/read-heavy workloads

### 5. Handle Version Conflicts

Implement retry logic for CAS operations:

```java
int maxRetries = 10;
for (int i = 0; i < maxRetries; i++) {
    try {
        GetResult current = client.get(key, null);
        // ... compute new value ...

        PutOptions options = PutOptions.builder()
            .ifMatchVersion(current.getVersion())
            .build();

        client.put(key, newValue, options);
        break; // Success
    } catch (VersionMismatchException e) {
        if (i == maxRetries - 1) throw e;
        Thread.sleep(10); // Small backoff
    }
}
```

### 6. Set Appropriate Timeouts

```java
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(1024)
    .timeoutMs(5000) // 5 seconds
    .build();
```

### 7. Monitor Client Health

```java
// Periodically check stats
NoriKVClient.ClientStats stats = client.getStats();
if (stats.isClosed()) {
    logger.error("Client is closed!");
}
```

### 8. Use UTF-8 Encoding

Always use UTF-8 for string encoding:

```java
import java.nio.charset.StandardCharsets;

byte[] key = "user:123".getBytes(StandardCharsets.UTF_8);
String value = new String(bytes, StandardCharsets.UTF_8);
```

## Performance Tips

### 1. Batch Operations

Process multiple operations efficiently:

```java
// Process in batches
List<byte[]> keys = getKeysToProcess();
for (byte[] key : keys) {
    client.put(key, value, null);
}
```

### 2. Connection Pooling

The client maintains a connection pool internally - no need for external pooling.

### 3. Concurrent Access

Client is thread-safe:

```java
ExecutorService executor = Executors.newFixedThreadPool(10);
for (int i = 0; i < 100; i++) {
    executor.submit(() -> {
        client.put(key, value, null);
    });
}
```

### 4. Use Appropriate Value Sizes

- Optimal: 100 bytes - 10 KB
- Maximum: Limited by memory and network

### 5. Minimize Version Conflicts

Reduce contention on hot keys:
- Use finer-grained keys
- Implement backoff strategies
- Consider event sourcing patterns

## Complete Example

```java
import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

public class CompleteExample {
    public static void main(String[] args) {
        // Configure with retry policy
        RetryConfig retryConfig = RetryConfig.builder()
            .maxAttempts(5)
            .initialDelayMs(100)
            .maxDelayMs(2000)
            .build();

        ClientConfig config = ClientConfig.builder()
            .nodes(Arrays.asList("localhost:9001", "localhost:9002"))
            .totalShards(1024)
            .timeoutMs(5000)
            .retry(retryConfig)
            .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            // Write with TTL and idempotency
            byte[] key = "session:abc123".getBytes(StandardCharsets.UTF_8);
            byte[] value = "{\"user_id\":42}".getBytes(StandardCharsets.UTF_8);

            PutOptions putOpts = PutOptions.builder()
                .ttlMs(3600000L) // 1 hour
                .idempotencyKey("session-create-abc123")
                .build();

            Version version = client.put(key, value, putOpts);
            System.out.println("Written: " + version);

            // Read with linearizable consistency
            GetOptions getOpts = GetOptions.builder()
                .consistency(ConsistencyLevel.LINEARIZABLE)
                .build();

            GetResult result = client.get(key, getOpts);
            String data = new String(result.getValue(), StandardCharsets.UTF_8);
            System.out.println("Read: " + data);

            // Update with CAS
            byte[] newValue = "{\"user_id\":42,\"active\":true}"
                .getBytes(StandardCharsets.UTF_8);

            PutOptions casOpts = PutOptions.builder()
                .ifMatchVersion(result.getVersion())
                .build();

            try {
                client.put(key, newValue, casOpts);
                System.out.println("CAS succeeded");
            } catch (VersionMismatchException e) {
                System.out.println("CAS failed - retry needed");
            }

            // Monitor topology
            client.onTopologyChange(event -> {
                System.out.println("Cluster changed: epoch " +
                    event.getCurrentEpoch());
            });

            // Get statistics
            NoriKVClient.ClientStats stats = client.getStats();
            System.out.println("Stats: " + stats);

        } catch (KeyNotFoundException e) {
            System.err.println("Key not found");
        } catch (ConnectionException e) {
            System.err.println("Connection failed: " + e.getMessage());
        } catch (NoriKVException e) {
            System.err.println("Error: " + e.getCode());
        }
    }
}
```

## Next Steps

- [Architecture Guide](ARCHITECTURE.md) - Understanding client internals
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Solving common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Examples](../src/main/java/com/norikv/client/examples/) - Working code samples
