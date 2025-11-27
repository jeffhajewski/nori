# NoriKV Go Client Architecture

Understanding the internal design and components of the Go client SDK.

## Table of Contents

- [Overview](#overview)
- [Component Architecture](#component-architecture)
- [Request Flow](#request-flow)
- [Concurrency Model](#concurrency-model)
- [Connection Management](#connection-management)
- [Routing & Sharding](#routing--sharding)
- [Retry Logic](#retry-logic)
- [Error Handling](#error-handling)

## Overview

The NoriKV Go client is designed as a **smart client** that:
- Routes requests directly to the appropriate shard leader
- Maintains connection pools for efficient communication
- Implements retry logic with exponential backoff
- Tracks cluster topology changes
- Provides goroutine-safe operations
- Optimizes hot paths with zero-allocation routing

### Design Principles

1. **Zero-hop routing**: Client routes directly to shard leader (no proxy)
2. **Goroutine-safe**: Single client instance shared across goroutines
3. **Zero-allocation hot path**: No heap allocations in routing
4. **Single-flight pattern**: Deduplicate concurrent leader discovery
5. **Observable**: Expose metrics and statistics

## Component Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Client                              │
│  (Main API: Put, Get, Delete, topology, stats)          │
└──────────────────┬──────────────────────────────────────┘
                   │
        ┌──────────┼──────────┬──────────┐
        │          │          │          │
┌───────▼────┐ ┌──▼────┐ ┌───▼──────┐ ┌─▼─────────┐
│   Router   │ │ Retry │ │  ConnPool│ │ Topology  │
│            │ │Policy │ │          │ │ Manager   │
└────────────┘ └───────┘ └──────────┘ └───────────┘
     │                         │              │
     │                         │              │
     └─────────hash()──────────┤              │
                               │              │
     ┌─────────getConn()───────┤              │
     │                         │              │
     │                    ┌────▼────┐         │
     │                    │  gRPC   │         │
     │                    │  Conns  │         │
     │                    └────┬────┘         │
     │                         │              │
     │                         │              │
     └─────────updateView()────┴──────────────┘
```

### Components

#### 1. Client

**Responsibility**: Main public API and component coordination

**Key Methods**:
- `Put()`, `Get()`, `Delete()` - Core operations
- `GetClusterView()` - Topology information
- `OnTopologyChange()` - Subscribe to topology updates
- `Stats()` - Client statistics
- `Close()` - Resource cleanup

**Location**: `client.go`

#### 2. Router

**Responsibility**: Determine which node to send requests to

**Key Functions**:
- Hash key to shard: `xxhash64(key) → jumpConsistentHash(hash, totalShards) → shardId`
- Map shard to leader node
- Cache leader information
- Single-flight pattern for leader discovery
- Handle leader hints from `NOT_LEADER` errors

**Location**: `internal/router/router.go`

**Algorithm**:
```
1. Hash key using XXHash64 (seed=0)
2. Map hash to shard using Jump Consistent Hash
3. Look up shard leader in topology cache
4. If unknown, use single-flight to discover leader
5. Return leader's address
```

#### 3. ConnectionPool

**Responsibility**: Manage gRPC connections to cluster nodes

**Key Functions**:
- Create and cache gRPC connections per node
- Goroutine-safe concurrent access
- Health checking
- Graceful shutdown

**Location**: `internal/conn/pool.go`

**Design**:
- One `*grpc.ClientConn` per node address
- Lazy initialization (created on first use)
- Connections reused across requests
- Automatic cleanup on client close

#### 4. RetryPolicy

**Responsibility**: Handle transient failures with backoff

**Key Functions**:
- Exponential backoff: `delay = min(initialDelay * 2^attempt, maxDelay)`
- Jitter: Add randomness to avoid thundering herd
- Selective retry: Only retry transient errors
- Attempt tracking

**Location**: `internal/retry/policy.go`

**Retryable Errors**:
- `Unavailable` - Server temporarily unavailable
- `Aborted` - Operation aborted, safe to retry
- `DeadlineExceeded` - Timeout, may succeed on retry
- `ResourceExhausted` - Rate limited, backoff helps

**Non-Retryable Errors**:
- `InvalidArgument` - Client error, won't succeed
- `NotFound` - Key doesn't exist
- `FailedPrecondition` - CAS conflict, application must retry
- `PermissionDenied` - Auth error

#### 5. TopologyManager

**Responsibility**: Track cluster membership and shard assignments

**Key Functions**:
- Store current `ClusterView`
- Cache shard → leader mappings
- Detect topology changes
- Notify listeners of changes
- Update leader hints

**Location**: `internal/topology/manager.go`

**Data Structures**:
- `ClusterView`: Current cluster state (epoch, nodes, shards)
- `shardLeaderCache`: sync.Map for shard → leader address
- `listeners`: slice of change callbacks (protected by RWMutex)

## Request Flow

### PUT Request Flow

```
Client.Put(ctx, key, value, options)
    │
    ├─> 1. Validate inputs (key, value not nil/empty)
    │
    ├─> 2. Router.GetNodeForKey(key)
    │       ├─> hash = xxhash64(key)
    │       ├─> shardId = jumpConsistentHash(hash, totalShards)
    │       └─> leaderAddr = topologyManager.GetShardLeader(shardId)
    │
    ├─> 3. ConnectionPool.GetConn(leaderAddr)
    │       └─> Return cached or create new gRPC connection
    │
    ├─> 4. RetryPolicy.Do(func() error {
    │       ├─> Build gRPC PutRequest
    │       ├─> proto.NewKvClient(conn).Put(ctx, request)
    │       └─> Convert response to Version
    │   })
    │       ├─> On SUCCESS: return Version
    │       ├─> On RETRYABLE_ERROR: backoff and retry
    │       └─> On NON_RETRYABLE: return error
    │
    └─> 5. Return Version to caller
```

### GET Request Flow

Similar to PUT, but:
- Uses `GetRequest` with consistency level
- Returns `GetResult` (value + version)
- Returns `ErrKeyNotFound` on NOT_FOUND

### Error Handling in Flow

```
gRPC Status Error
    │
    ├─> convertGrpcError()
    │   ├─> NotFound → ErrKeyNotFound
    │   ├─> FailedPrecondition + "version" → ErrVersionMismatch
    │   ├─> Unavailable → ErrConnection
    │   └─> OTHER → wrapped error
    │
    └─> RetryPolicy decides:
        ├─> Retryable → backoff and retry
        └─> Non-retryable → return to caller
```

## Concurrency Model

### Goroutine Safety Guarantees

All client components are goroutine-safe:

1. **Client**: Goroutine-safe, shareable across goroutines
2. **Router**: Uses sync.Map for leader cache, immutable routing tables
3. **ConnectionPool**: sync.Mutex for connection creation
4. **TopologyManager**: sync.RWMutex for view updates, sync.Map for caches
5. **RetryPolicy**: Stateless, safe for concurrent use

### Concurrency Design

```go
//  Safe: Single client, multiple goroutines
client, _ := norikv.NewClient(ctx, config)

var wg sync.WaitGroup
for i := 0; i < 100; i++ {
    wg.Add(1)
    go func() {
        defer wg.Done()
        client.Put(ctx, key, value, nil) // Goroutine-safe
    }()
}
wg.Wait()
```

### Synchronization Points

1. **TopologyManager.UpdateView()**: Write lock during view update
2. **ConnectionPool.GetConn()**: Mutex for connection creation
3. **Router leader cache**: sync.Map for lock-free reads

### Single-Flight Pattern

Concurrent requests for the same shard's leader are deduplicated:

```go
// Multiple goroutines requesting same shard
// Only one goroutine makes the actual RPC
// Others wait for the result
result := router.getLeaderForShard(shardID)
```

## Connection Management

### Connection Lifecycle

```
Node Address
    │
    ├─> First request → Create gRPC ClientConn
    │   ├─> Configure: insecure, keepalive, timeout
    │   └─> Store in pool
    │
    ├─> Subsequent requests → Reuse connection
    │
    └─> Client.Close() → Close all connections
        └─> Graceful shutdown with context timeout
```

### Connection Configuration

```go
conn, err := grpc.Dial(
    address,
    grpc.WithTransportCredentials(insecure.NewCredentials()),
    grpc.WithKeepaliveParams(keepalive.ClientParameters{
        Time:                10 * time.Second,
        Timeout:             3 * time.Second,
        PermitWithoutStream: true,
    }),
)
```

### Health Checks

- Connections automatically reconnect on failure
- gRPC handles connection health internally
- Failed requests trigger retries (via RetryPolicy)

## Routing & Sharding

### Hash Function: XXHash64

```go
hash := xxhash.Sum64(key)
```

**Properties**:
- Fast: ~2.5ns per operation
- Consistent: Same key → same hash
- Zero allocations
- Cross-SDK compatible

### Consistent Hashing: Jump Consistent Hash

```go
shardID := jumpConsistentHash(hash, totalShards)
```

**Properties**:
- Minimal key movement on shard count changes
- O(log n) time complexity
- Uniform distribution
- Zero allocations

### Shard → Leader Mapping

```
shardID → TopologyManager.GetShardLeader(shardID) → leaderAddr
```

**Leader Cache**:
- Populated from ClusterView
- Updated on topology changes
- Updated from NOT_LEADER error hints
- Uses sync.Map for lock-free reads

### NOT_LEADER Handling

```
1. Request sent to node A for shard X
2. Node A returns NOT_LEADER error with hint: "leader is node B"
3. Client updates leader cache: shard X → node B
4. Retry policy re-sends request to node B
```

## Retry Logic

### Exponential Backoff

```
delay = min(initialDelay * 2^attempt, maxDelay) + jitter
```

**Example** (initialDelay=100ms, maxDelay=5s, jitter=100ms):
```
Attempt 1: delay = 100ms  + random(0-100ms)
Attempt 2: delay = 200ms  + random(0-100ms)
Attempt 3: delay = 400ms  + random(0-100ms)
Attempt 4: delay = 800ms  + random(0-100ms)
Attempt 5: delay = 1600ms + random(0-100ms)
Attempt 6: delay = 3200ms + random(0-100ms)
Attempt 7: delay = 5000ms + random(0-100ms) (capped)
```

### Jitter Benefits

- Avoids thundering herd (all clients retry at same time)
- Spreads load during recovery
- Reduces collision probability

### Retry Decision Tree

```
Error Received
    │
    ├─> Is retryable? (Unavailable, Aborted, etc.)
    │   │
    │   ├─> YES: attempt < maxAttempts?
    │   │   ├─> YES: backoff and retry
    │   │   └─> NO: return RETRY_EXHAUSTED
    │   │
    │   └─> NO: return original error
    │
    └─> Success: return result
```

## Error Handling

### Error Types

```go
var (
    ErrKeyNotFound      = errors.New("key not found")
    ErrVersionMismatch  = errors.New("version mismatch")
    ErrAlreadyExists    = errors.New("key already exists")
    ErrConnection       = errors.New("connection error")
)
```

### Error Code Mapping

| gRPC Status | NoriKV Error | Retry? |
|-------------|--------------|--------|
| NotFound | ErrKeyNotFound | No |
| FailedPrecondition (version) | ErrVersionMismatch | No |
| FailedPrecondition (other) | wrapped error | No |
| AlreadyExists | ErrAlreadyExists | No |
| Unavailable | ErrConnection | Yes |
| DeadlineExceeded | ErrConnection | Yes |
| Aborted | wrapped error | Yes |
| ResourceExhausted | wrapped error | Yes |
| InvalidArgument | wrapped error | No |
| PermissionDenied | wrapped error | No |

### Error Context

Errors include:
- Error message
- Wrapped original error (via errors.Unwrap)
- Context (key, version for VersionMismatch)

## Performance Considerations

### Hot Paths

1. **Hash calculation**: Optimized XXHash64 (2.5ns)
2. **Connection lookup**: O(1) map lookup
3. **Leader cache**: Lock-free sync.Map reads
4. **Protobuf serialization**: Minimal overhead

### Zero-Allocation Routing

```
GetShardForKey: 23ns/op, 0 B/op, 0 allocs/op
```

The routing hot path allocates no heap memory.

### Memory Usage

- **Per client**: ~1-10 MB (depends on number of nodes)
- **Per connection**: ~100 KB (gRPC overhead)
- **Per request**: Minimal (request/response objects garbage collected)

### Connection Pooling

- Connections reused across requests
- No connection per request overhead
- gRPC multiplexes requests over HTTP/2

### Single-Flight Optimization

Concurrent requests for the same shard's leader are deduplicated:
- First request makes the RPC
- Subsequent requests wait for the result
- Reduces load on cluster during leader discovery

## Observability

### Client Statistics

```go
stats := client.Stats()

// Router stats
fmt.Printf("Total nodes: %d\n", stats.Router.TotalNodes)
fmt.Printf("Total shards: %d\n", stats.Router.TotalShards)

// Connection pool stats
fmt.Printf("Active connections: %d\n", stats.Pool.ActiveConnections)

// Topology stats
fmt.Printf("Current epoch: %d\n", stats.Topology.CurrentEpoch)
fmt.Printf("Cached leaders: %d\n", stats.Topology.CachedLeaders)
```

### Topology Change Events

```go
client.OnTopologyChange(func(event *TopologyChangeEvent) {
    log.Printf("Topology changed to epoch %d", event.CurrentEpoch)
    log.Printf("Added nodes: %v", event.AddedNodes)
    log.Printf("Removed nodes: %v", event.RemovedNodes)
    log.Printf("Leader changes: %v", event.LeaderChanges)
})
```

## Extensibility

### Custom Retry Policies

Implement custom backoff strategies:

```go
customRetry := &norikv.RetryConfig{
    MaxAttempts:  20,
    InitialDelay: 50 * time.Millisecond,
    MaxDelay:     10 * time.Second,
    Jitter:       200 * time.Millisecond,
}
```

### Future Extensions

Potential areas for extension:
- Custom hash functions (requires cross-SDK coordination)
- TLS/SSL support
- Authentication integration
- Metrics export (Prometheus, OTLP)
- Circuit breaker patterns
- Request tracing

## Comparison with Other SDKs

### Similarities

- Same hash functions (XXHash64 + Jump Consistent Hash)
- Same routing algorithm
- Same proto definitions
- Same error handling patterns

### Go-Specific Features

- Zero-allocation routing hot path
- Single-flight pattern for leader discovery
- Goroutine-safe concurrent access
- Context-aware operations
- defer-based resource management

### Performance

Go SDK performance is excellent:
- **Hash operations**: 2.5ns per operation (fastest)
- **Zero allocations**: In routing hot path
- **Native concurrency**: Goroutines with minimal overhead
- **Efficient GC**: Minimal garbage collection pressure

## References

- [API Guide](API_GUIDE.md) - Public API documentation
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Source Code](../) - Implementation
