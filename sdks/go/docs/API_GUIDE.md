# NoriKV Go Client API Guide

Complete reference for the NoriKV Go Client SDK.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Client Configuration](#client-configuration)
- [Core Operations](#core-operations)
- [Advanced Features](#advanced-features)
- [Error Handling](#error-handling)
- [Best Practices](#best-practices)

## Installation

```bash
go get github.com/norikv/norikv-go
```

## Quick Start

```go
package main

import (
    "context"
    "fmt"
    "log"

    norikv "github.com/norikv/norikv-go"
)

func main() {
    ctx := context.Background()

    // Configure client
    config := norikv.ClientConfig{
        Nodes:       []string{"localhost:9001", "localhost:9002"},
        TotalShards: 1024,
        Timeout:     5 * time.Second,
    }

    // Create client
    client, err := norikv.NewClient(ctx, &config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Put a value
    key := []byte("user:alice")
    value := []byte(`{"name":"Alice","age":30}`)

    version, err := client.Put(ctx, key, value, nil)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Wrote version: %v\n", version)

    // Get the value
    result, err := client.Get(ctx, key, nil)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Value: %s\n", result.Value)

    // Delete
    err = client.Delete(ctx, key, nil)
    if err != nil {
        log.Fatal(err)
    }
}
```

## Client Configuration

### Basic Configuration

```go
config := &norikv.ClientConfig{
    Nodes:       []string{"node1:9001", "node2:9001"},
    TotalShards: 1024,
    Timeout:     5 * time.Second,
}
```

### Configuration Options

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `Nodes` | `[]string` | **Required** | List of node addresses (host:port) |
| `TotalShards` | `int` | **Required** | Total number of shards in cluster |
| `Timeout` | `time.Duration` | 5s | Request timeout |
| `Retry` | `*RetryConfig` | See below | Retry policy configuration |

### Retry Configuration

```go
retryConfig := &norikv.RetryConfig{
    MaxAttempts:   10,
    InitialDelay:  100 * time.Millisecond,
    MaxDelay:      5 * time.Second,
    Jitter:        100 * time.Millisecond,
}

config := &norikv.ClientConfig{
    Nodes:       []string{"localhost:9001"},
    TotalShards: 1024,
    Retry:       retryConfig,
}
```

**Retry Behavior:**
- Retries transient errors: `Unavailable`, `Aborted`, `DeadlineExceeded`, `ResourceExhausted`
- Does NOT retry: `InvalidArgument`, `NotFound`, `FailedPrecondition`, `PermissionDenied`
- Uses exponential backoff with jitter

### Default Configuration

```go
// Quick setup with defaults
config := norikv.DefaultClientConfig([]string{"localhost:9001"})
```

## Core Operations

### PUT - Write Data

#### Basic PUT

```go
key := []byte("user:123")
value := []byte(`{"name":"Alice"}`)

version, err := client.Put(ctx, key, value, nil)
if err != nil {
    log.Fatal(err)
}
fmt.Printf("Written at version: %v\n", version)
```

#### PUT with Options

```go
ttl := uint64(60000) // 60 seconds

options := &norikv.PutOptions{
    TTLMs:          &ttl,
    IdempotencyKey: "order-12345",
    IfMatchVersion: expectedVersion, // CAS
}

version, err := client.Put(ctx, key, value, options)
```

**PutOptions Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `TTLMs` | `*uint64` | Time-to-live in milliseconds |
| `IdempotencyKey` | `string` | Key for idempotent operations |
| `IfMatchVersion` | `*Version` | Expected version for CAS |
| `IfNotExists` | `bool` | Only write if key doesn't exist |

### GET - Read Data

#### Basic GET

```go
key := []byte("user:123")
result, err := client.Get(ctx, key, nil)
if err != nil {
    log.Fatal(err)
}

value := result.Value
version := result.Version
```

#### GET with Consistency Level

```go
options := &norikv.GetOptions{
    Consistency: norikv.ConsistencyLinearizable,
}

result, err := client.Get(ctx, key, options)
```

**Consistency Levels:**

| Level | Description | Use Case |
|-------|-------------|----------|
| `ConsistencyLease` | Default, lease-based read | Most operations (fast, usually consistent) |
| `ConsistencyLinearizable` | Strictest, always up-to-date | Critical reads requiring absolute consistency |
| `ConsistencyStaleOK` | May return stale data | Read-heavy workloads, caching |

### DELETE - Remove Data

#### Basic DELETE

```go
key := []byte("user:123")
err := client.Delete(ctx, key, nil)
if err != nil {
    log.Fatal(err)
}
```

#### DELETE with Options

```go
options := &norikv.DeleteOptions{
    IdempotencyKey: "delete-order-12345",
    IfMatchVersion: expectedVersion,
}

err := client.Delete(ctx, key, options)
```

## Advanced Features

### Compare-And-Swap (CAS)

Optimistic concurrency control using version matching:

```go
// Read current value
result, err := client.Get(ctx, key, nil)
if err != nil {
    log.Fatal(err)
}

// Modify value
value, _ := strconv.Atoi(string(result.Value))
newValue := []byte(strconv.Itoa(value + 1))

// Update with CAS
options := &norikv.PutOptions{
    IfMatchVersion: result.Version,
}

_, err = client.Put(ctx, key, newValue, options)
if errors.Is(err, norikv.ErrVersionMismatch) {
    fmt.Println("CAS failed - version changed")
} else if err != nil {
    log.Fatal(err)
}
```

### Idempotent Operations

Safe retries using idempotency keys:

```go
idempotencyKey := "payment-" + uuid.New().String()

options := &norikv.PutOptions{
    IdempotencyKey: idempotencyKey,
}

// First attempt
v1, err := client.Put(ctx, key, value, options)

// Retry with same key (safe - returns same version)
v2, err := client.Put(ctx, key, value, options)

// v1 and v2 are equal
```

### Time-To-Live (TTL)

Automatic expiration:

```go
ttl := uint64(60000) // 60 seconds

options := &norikv.PutOptions{
    TTLMs: &ttl,
}

client.Put(ctx, key, value, options)

// Key automatically deleted after TTL
time.Sleep(61 * time.Second)
_, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrKeyNotFound) {
    fmt.Println("Key expired")
}
```

### Cluster Topology

Monitor cluster changes:

```go
// Get current cluster view
view := client.GetClusterView()
if view != nil {
    fmt.Printf("Cluster epoch: %d\n", view.Epoch)
    fmt.Printf("Nodes: %d\n", len(view.Nodes))
}

// Subscribe to topology changes
unsubscribe := client.OnTopologyChange(func(event *norikv.TopologyChangeEvent) {
    fmt.Printf("Topology changed!\n")
    fmt.Printf("Previous epoch: %d\n", event.PreviousEpoch)
    fmt.Printf("Current epoch: %d\n", event.CurrentEpoch)
    fmt.Printf("Added nodes: %v\n", event.AddedNodes)
    fmt.Printf("Removed nodes: %v\n", event.RemovedNodes)
})

// Later: unsubscribe
defer unsubscribe()
```

### Client Statistics

Monitor client performance:

```go
stats := client.Stats()

fmt.Printf("Active connections: %d\n", stats.Pool.ActiveConnections)
fmt.Printf("Total nodes: %d\n", stats.Router.TotalNodes)
fmt.Printf("Cached leaders: %d\n", stats.Topology.CachedLeaders)
```

## Error Handling

### Error Types

```go
var (
    ErrKeyNotFound      error // Key does not exist
    ErrVersionMismatch  error // CAS version conflict
    ErrAlreadyExists    error // IfNotExists conflict
    ErrConnection       error // Network or cluster issues
)
```

### Handling Specific Errors

```go
result, err := client.Get(ctx, key, nil)
if err != nil {
    switch {
    case errors.Is(err, norikv.ErrKeyNotFound):
        fmt.Println("Key not found")
    case errors.Is(err, norikv.ErrConnection):
        fmt.Println("Connection error:", err)
    default:
        fmt.Println("Error:", err)
    }
}
```

### Retry Pattern

```go
maxAttempts := 3
for attempt := 1; attempt <= maxAttempts; attempt++ {
    _, err := client.Put(ctx, key, value, nil)
    if err == nil {
        break // Success
    }

    if !errors.Is(err, norikv.ErrConnection) {
        return err // Non-retryable
    }

    if attempt == maxAttempts {
        return err // Give up
    }

    // Exponential backoff
    time.Sleep(time.Duration(1<<attempt) * 100 * time.Millisecond)
}
```

### Graceful Degradation

```go
func getWithFallback(client *norikv.Client, ctx context.Context, key, defaultValue []byte) []byte {
    result, err := client.Get(ctx, key, nil)
    if err != nil {
        log.Printf("Failed to get key, using default: %v", err)
        return defaultValue
    }
    return result.Value
}
```

## Best Practices

### 1. Use Context for Timeouts

Always pass context with timeout:

```go
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
defer cancel()

result, err := client.Get(ctx, key, nil)
```

### 2. Reuse Client Instances

Clients manage connection pools and should be reused:

```go
// ✅ Good: Single client instance
var client *norikv.Client

func init() {
    config := norikv.DefaultClientConfig([]string{"localhost:9001"})
    client, _ = norikv.NewClient(context.Background(), config)
}

// ❌ Bad: Creating client per request
func handleRequest() {
    client, _ := norikv.NewClient(context.Background(), config)
    defer client.Close() // Closes connections!
}
```

### 3. Use defer for Cleanup

```go
client, err := norikv.NewClient(ctx, config)
if err != nil {
    return err
}
defer client.Close()
```

### 4. Use Idempotency Keys

For operations that must not be duplicated:

```go
idempotencyKey := "order-" + orderID

options := &norikv.PutOptions{
    IdempotencyKey: idempotencyKey,
}

client.Put(ctx, key, value, options)
```

### 5. Choose Appropriate Consistency

- Use `ConsistencyLease` (default) for most operations
- Use `ConsistencyLinearizable` for critical reads
- Use `ConsistencyStaleOK` for caching/read-heavy workloads

### 6. Handle Version Conflicts

Implement retry logic for CAS operations:

```go
maxRetries := 10
for i := 0; i < maxRetries; i++ {
    result, err := client.Get(ctx, key, nil)
    if err != nil {
        return err
    }

    // ... compute new value ...

    options := &norikv.PutOptions{
        IfMatchVersion: result.Version,
    }

    _, err = client.Put(ctx, key, newValue, options)
    if err == nil {
        break // Success
    }

    if !errors.Is(err, norikv.ErrVersionMismatch) {
        return err
    }

    if i == maxRetries-1 {
        return err
    }

    time.Sleep(10 * time.Millisecond) // Small backoff
}
```

### 7. Use Goroutines Safely

Client is goroutine-safe:

```go
var wg sync.WaitGroup

for i := 0; i < 100; i++ {
    wg.Add(1)
    go func(i int) {
        defer wg.Done()
        key := []byte(fmt.Sprintf("key-%d", i))
        client.Put(ctx, key, value, nil)
    }(i)
}

wg.Wait()
```

### 8. Monitor Client Health

```go
// Periodically check stats
stats := client.Stats()
if stats.Pool.ActiveConnections == 0 {
    log.Error("No active connections!")
}
```

## Performance Tips

### 1. Concurrent Access

Client is optimized for concurrent use:

```go
numWorkers := runtime.NumCPU()
work := make(chan []byte, 100)

for i := 0; i < numWorkers; i++ {
    go func() {
        for key := range work {
            client.Put(ctx, key, value, nil)
        }
    }()
}
```

### 2. Zero-Allocation Routing

The client uses optimized routing with zero heap allocations in the hot path.

### 3. Connection Pooling

The client maintains a connection pool internally - no external pooling needed.

### 4. Single-Flight Pattern

Concurrent requests for the same shard's leader are deduplicated automatically.

### 5. Use Appropriate Value Sizes

- Optimal: 100 bytes - 10 KB
- Maximum: Limited by memory and network

## Complete Example

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
    ctx := context.Background()

    // Configure with retry policy
    retryConfig := &norikv.RetryConfig{
        MaxAttempts:  5,
        InitialDelay: 100 * time.Millisecond,
        MaxDelay:     2 * time.Second,
    }

    config := &norikv.ClientConfig{
        Nodes:       []string{"localhost:9001", "localhost:9002"},
        TotalShards: 1024,
        Timeout:     5 * time.Second,
        Retry:       retryConfig,
    }

    client, err := norikv.NewClient(ctx, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Write with TTL and idempotency
    key := []byte("session:abc123")
    value := []byte(`{"user_id":42}`)
    ttl := uint64(3600000) // 1 hour

    putOpts := &norikv.PutOptions{
        TTLMs:          &ttl,
        IdempotencyKey: "session-create-abc123",
    }

    version, err := client.Put(ctx, key, value, putOpts)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Written: %v\n", version)

    // Read with linearizable consistency
    getOpts := &norikv.GetOptions{
        Consistency: norikv.ConsistencyLinearizable,
    }

    result, err := client.Get(ctx, key, getOpts)
    if err != nil {
        log.Fatal(err)
    }
    fmt.Printf("Read: %s\n", result.Value)

    // Update with CAS
    newValue := []byte(`{"user_id":42,"active":true}`)
    casOpts := &norikv.PutOptions{
        IfMatchVersion: result.Version,
    }

    _, err = client.Put(ctx, key, newValue, casOpts)
    if err != nil {
        if errors.Is(err, norikv.ErrVersionMismatch) {
            fmt.Println("CAS failed - retry needed")
        } else {
            log.Fatal(err)
        }
    }

    // Monitor topology
    client.OnTopologyChange(func(event *norikv.TopologyChangeEvent) {
        fmt.Printf("Cluster changed: epoch %d\n", event.CurrentEpoch)
    })

    // Get statistics
    stats := client.Stats()
    fmt.Printf("Stats: %+v\n", stats)
}
```

## Next Steps

- [Architecture Guide](ARCHITECTURE.md) - Understanding client internals
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Solving common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Examples](../examples/) - Working code samples
