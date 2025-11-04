# Basic NoriKV Usage Example

This example demonstrates comprehensive usage of the NoriKV Go client SDK with a real cluster.

## Prerequisites

A running NoriKV cluster with at least one node. Update the node addresses in `main.go`:

```go
config := norikv.DefaultClientConfig([]string{
    "localhost:9001",  // Update these addresses
    "localhost:9002",
    "localhost:9003",
})
```

## Running

```bash
go run main.go
```

## What This Example Covers

### 1. Basic Put/Get Operations
- Writing key-value pairs
- Reading values back
- Understanding versions (term + index)

### 2. Conditional Updates (Compare-and-Swap)
- Using `IfMatchVersion` for optimistic concurrency control
- Handling version conflicts
- Implementing safe concurrent updates

### 3. TTL (Time-to-Live) Expiration
- Setting expiration times on keys
- Automatic cleanup after TTL
- Use cases: sessions, caches, temporary data

### 4. Consistency Levels
- **Lease-based**: Fast reads using leader leases (default)
- **Linearizable**: Strongest consistency, requires read-index
- **Stale-OK**: Fastest reads, may return stale data

### 5. Idempotent Writes
- Using idempotency keys for safe retries
- Preventing duplicate operations
- Network failure resilience

## Key Concepts

### Versions
Every write returns a `Version` with:
- `Term`: Raft term number (changes during leader elections)
- `Index`: Monotonically increasing log index

Use versions for conditional updates to prevent lost updates.

### Error Handling
Common errors to handle:
- `KeyNotFoundError`: Key doesn't exist
- `VersionMismatchError`: Conditional update failed
- `ConnectionError`: Network issues
- `NotLeaderError`: Redirect to correct leader (handled automatically with retries)

## Configuration Options

Customize the client:

```go
config := norikv.DefaultClientConfig(nodes)
config.TimeoutMs = 5000           // Request timeout
config.TotalShards = 1024          // Must match server
config.WatchCluster = true         // Enable topology watching
config.Retry.MaxAttempts = 3       // Retry attempts
config.Retry.RetryNotLeader = true // Auto-retry on redirect
```

## Learn More

- [Go SDK README](../../README.md)
- [NoriKV Documentation](../../../../README.md)
- [Ephemeral Server Example](../ephemeral/)
