# NoriKV Java Client Architecture

Understanding the internal design and components of the Java client SDK.

## Table of Contents

- [Overview](#overview)
- [Component Architecture](#component-architecture)
- [Request Flow](#request-flow)
- [Threading Model](#threading-model)
- [Connection Management](#connection-management)
- [Routing & Sharding](#routing--sharding)
- [Retry Logic](#retry-logic)
- [Error Handling](#error-handling)

## Overview

The NoriKV Java client is designed as a **smart client** that:
- Routes requests directly to the appropriate shard leader
- Maintains connection pools for efficient communication
- Implements retry logic with exponential backoff
- Tracks cluster topology changes
- Provides thread-safe operations

### Design Principles

1. **Zero-hop routing**: Client routes directly to shard leader (no proxy)
2. **Thread safety**: Single client instance shared across threads
3. **Fail-fast**: Detect and handle failures quickly
4. **Observable**: Expose metrics and statistics
5. **Resource efficient**: Connection pooling and reuse

## Component Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    NoriKVClient                         │
│  (Main API: put, get, delete, topology, stats)          │
└──────────────────┬──────────────────────────────────────┘
                   │
        ┌──────────┼──────────┬──────────┐
        │          │          │          │
┌───────▼────┐ ┌──▼────┐ ┌───▼──────┐ ┌─▼─────────┐
│   Router   │ │ Retry │ │   Pool   │ │ Topology  │
│            │ │Policy │ │          │ │ Manager   │
└────────────┘ └───────┘ └──────────┘ └───────────┘
     │                         │              │
     │                         │              │
     └─────────hash()──────────┤              │
                               │              │
     ┌─────────getChannel()────┤              │
     │                         │              │
     │                    ┌────▼────┐         │
     │                    │ gRPC    │         │
     │                    │Channels │         │
     │                    └────┬────┘         │
     │                         │              │
     │                         │              │
     └─────────updateView()────┴──────────────┘
```

### Components

#### 1. NoriKVClient

**Responsibility**: Main public API and component coordination

**Key Methods**:
- `put()`, `get()`, `delete()` - Core operations
- `getClusterView()` - Topology information
- `onTopologyChange()` - Subscribe to topology updates
- `getStats()` - Client statistics
- `close()` - Resource cleanup

**Location**: `src/main/java/com/norikv/client/NoriKVClient.java`

#### 2. Router

**Responsibility**: Determine which node to send requests to

**Key Functions**:
- Hash key to shard: `xxhash64(key) → jumpConsistentHash(hash, totalShards) → shardId`
- Map shard to leader node
- Cache leader information
- Handle leader hints from `NOT_LEADER` errors

**Location**: `src/main/java/com/norikv/client/internal/router/Router.java`

**Algorithm**:
```
1. Hash key using XXHash64 (seed=0)
2. Map hash to shard using Jump Consistent Hash
3. Look up shard leader in topology cache
4. Return leader's address
```

#### 3. ConnectionPool

**Responsibility**: Manage gRPC channels to cluster nodes

**Key Functions**:
- Create and cache gRPC channels per node
- Thread-safe concurrent access
- Graceful shutdown
- Health monitoring

**Location**: `src/main/java/com/norikv/client/internal/conn/ConnectionPool.java`

**Design**:
- One `ManagedChannel` per node address
- Lazy initialization (created on first use)
- Channels reused across requests
- Automatic shutdown on client close

#### 4. RetryPolicy

**Responsibility**: Handle transient failures with backoff

**Key Functions**:
- Exponential backoff: `delay = min(initialDelay * 2^attempt, maxDelay)`
- Jitter: Add randomness to avoid thundering herd
- Selective retry: Only retry transient errors
- Attempt tracking

**Location**: `src/main/java/com/norikv/client/internal/retry/RetryPolicy.java`

**Retryable Errors**:
- `UNAVAILABLE` - Server temporarily unavailable
- `ABORTED` - Operation aborted, safe to retry
- `DEADLINE_EXCEEDED` - Timeout, may succeed on retry
- `RESOURCE_EXHAUSTED` - Rate limited, backoff helps

**Non-Retryable Errors**:
- `INVALID_ARGUMENT` - Client error, won't succeed
- `NOT_FOUND` - Key doesn't exist
- `FAILED_PRECONDITION` - CAS conflict, application must retry
- `PERMISSION_DENIED` - Auth error

#### 5. TopologyManager

**Responsibility**: Track cluster membership and shard assignments

**Key Functions**:
- Store current `ClusterView`
- Cache shard → leader mappings
- Detect topology changes
- Notify listeners of changes
- Update leader hints

**Location**: `src/main/java/com/norikv/client/internal/topology/TopologyManager.java`

**Data Structures**:
- `ClusterView`: Current cluster state (epoch, nodes, shards)
- `shardLeaderCache`: ConcurrentHashMap<shardId, leaderAddr>
- `listeners`: CopyOnWriteArrayList of change callbacks

## Request Flow

### PUT Request Flow

```
Client.put(key, value, options)
    │
    ├─> 1. Validate inputs (key, value not null/empty)
    │
    ├─> 2. Router.getNodeForKey(key)
    │       ├─> hash = xxhash64(key)
    │       ├─> shardId = jumpConsistentHash(hash, totalShards)
    │       └─> leaderAddr = topologyManager.getShardLeader(shardId)
    │
    ├─> 3. ConnectionPool.getChannel(leaderAddr)
    │       └─> Return cached or create new gRPC channel
    │
    ├─> 4. RetryPolicy.execute(() -> {
    │       ├─> Build gRPC PutRequest (via ProtoConverters)
    │       ├─> KvGrpc.newBlockingStub(channel).put(request)
    │       └─> Convert response to Version
    │   })
    │       ├─> On SUCCESS: return Version
    │       ├─> On RETRYABLE_ERROR: backoff and retry
    │       └─> On NON_RETRYABLE: throw exception
    │
    └─> 5. Return Version to caller
```

### GET Request Flow

Similar to PUT, but:
- Uses `GetRequest` with consistency level
- Returns `GetResult` (value + version)
- Throws `KeyNotFoundException` on NOT_FOUND

### Error Handling in Flow

```
gRPC StatusRuntimeException
    │
    ├─> convertGrpcException()
    │   ├─> NOT_FOUND → KeyNotFoundException
    │   ├─> FAILED_PRECONDITION + "version" → VersionMismatchException
    │   ├─> UNAVAILABLE → ConnectionException
    │   └─> OTHER → NoriKVException
    │
    └─> RetryPolicy decides:
        ├─> Retryable → backoff and retry
        └─> Non-retryable → throw to caller
```

## Threading Model

### Thread Safety Guarantees

All client components are thread-safe:

1. **NoriKVClient**: Thread-safe, shareable across threads
2. **Router**: Uses immutable routing tables, atomic leader cache updates
3. **ConnectionPool**: ConcurrentHashMap for channel storage
4. **TopologyManager**: ReadWriteLock for view updates, ConcurrentHashMap for caches
5. **RetryPolicy**: Stateless, safe for concurrent use

### Concurrency Design

```java
// ✅ Safe: Single client, multiple threads
NoriKVClient client = new NoriKVClient(config);

ExecutorService executor = Executors.newFixedThreadPool(10);
for (int i = 0; i < 100; i++) {
    executor.submit(() -> {
        client.put(key, value, null); // Thread-safe
    });
}
```

### Synchronization Points

1. **TopologyManager.updateView()**: Write lock during view update
2. **ConnectionPool.getOrCreateChannel()**: Double-checked locking for channel creation
3. **Router leader cache**: ConcurrentHashMap for atomic updates

## Connection Management

### Channel Lifecycle

```
Node Address
    │
    ├─> First request → Create ManagedChannel
    │   ├─> Configure: plaintext, timeouts, keepalive
    │   └─> Store in pool
    │
    ├─> Subsequent requests → Reuse channel
    │
    └─> Client.close() → Shutdown all channels
        ├─> Graceful shutdown (5s timeout)
        └─> Force shutdown if needed
```

### Channel Configuration

```java
ManagedChannel channel = ManagedChannelBuilder
    .forTarget(address)
    .usePlaintext() // No TLS (for now)
    .build();
```

### Health Checks

- Channels automatically reconnect on failure
- gRPC handles connection health internally
- Failed requests trigger retries (via RetryPolicy)

## Routing & Sharding

### Hash Function: XXHash64

```java
long hash = xxhash64(key, seed=0);
```

**Properties**:
- Fast: ~10GB/s throughput
- Consistent: Same key → same hash
- Cross-SDK compatible: Identical to Go/Python/TypeScript implementations

### Consistent Hashing: Jump Consistent Hash

```java
int shardId = jumpConsistentHash(hash, totalShards);
```

**Properties**:
- Minimal key movement on shard count changes
- O(log n) time complexity
- Uniform distribution

### Shard → Leader Mapping

```
shardId → TopologyManager.getShardLeader(shardId) → leaderAddr
```

**Leader Cache**:
- Populated from ClusterView
- Updated on topology changes
- Updated from NOT_LEADER error hints

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

**Example** (initialDelay=100ms, maxDelay=5000ms, jitter=100ms):
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
    ├─> Is retryable? (UNAVAILABLE, ABORTED, etc.)
    │   │
    │   ├─> YES: attempt < maxAttempts?
    │   │   ├─> YES: backoff and retry
    │   │   └─> NO: throw RETRY_EXHAUSTED
    │   │
    │   └─> NO: throw original exception
    │
    └─> Success: return result
```

## Error Handling

### Exception Hierarchy

```
NoriKVException (checked)
├── KeyNotFoundException
├── VersionMismatchException
├── AlreadyExistsException
└── ConnectionException
```

### Error Code Mapping

| gRPC Status | NoriKV Exception | Retry? |
|-------------|------------------|--------|
| NOT_FOUND | KeyNotFoundException | No |
| FAILED_PRECONDITION (version) | VersionMismatchException | No |
| FAILED_PRECONDITION (other) | NoriKVException | No |
| ALREADY_EXISTS | AlreadyExistsException | No |
| UNAVAILABLE | ConnectionException | Yes |
| DEADLINE_EXCEEDED | ConnectionException | Yes |
| ABORTED | NoriKVException | Yes |
| RESOURCE_EXHAUSTED | NoriKVException | Yes |
| INVALID_ARGUMENT | NoriKVException | No |
| PERMISSION_DENIED | NoriKVException | No |
| OTHER | NoriKVException | No |

### Error Context

Exceptions include:
- Error code (string)
- Descriptive message
- Cause (original exception)
- Context (key, version for VersionMismatchException)

## Performance Considerations

### Hot Paths

1. **Hash calculation**: Optimized XXHash64 implementation
2. **Channel lookup**: O(1) ConcurrentHashMap lookup
3. **Leader cache**: O(1) lookup, populated eagerly
4. **Protobuf serialization**: Minimal overhead

### Memory Usage

- **Per client**: ~1-10 MB (depends on number of nodes)
- **Per channel**: ~100 KB (gRPC overhead)
- **Per request**: Minimal (request/response objects garbage collected)

### Connection Pooling

- Channels reused across requests
- No connection per request overhead
- gRPC multiplexes requests over HTTP/2

### Benchmarks

See [Performance Benchmarks](../README.md#performance-benchmarks) for detailed metrics.

## Observability

### Client Statistics

```java
ClientStats stats = client.getStats();

// Router stats
stats.getRouterStats().getTotalNodes();
stats.getRouterStats().getTotalShards();

// Connection pool stats
stats.getPoolStats().getActiveChannels();

// Topology stats
stats.getTopologyStats().getCurrentEpoch();
stats.getTopologyStats().getCachedLeaders();
```

### Topology Change Events

```java
client.onTopologyChange(event -> {
    logger.info("Topology changed to epoch {}", event.getCurrentEpoch());
    logger.info("Added nodes: {}", event.getAddedNodes());
    logger.info("Removed nodes: {}", event.getRemovedNodes());
    logger.info("Leader changes: {}", event.getLeaderChanges());
});
```

### Logging

Client uses `System.err` for internal errors (listener failures, etc.). Consider adding proper logging framework integration in production.

## Extensibility

### Custom Retry Policies

Implement custom backoff strategies:

```java
RetryConfig custom = RetryConfig.builder()
    .maxAttempts(20)
    .initialDelayMs(50)
    .maxDelayMs(10000)
    .jitterMs(200)
    .build();
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

### Java-Specific Features

- Builder pattern for configuration
- Try-with-resources (AutoCloseable)
- Java streams and functional interfaces
- Strong typing with generics

### Performance

Java SDK performance is comparable to other SDKs:
- Go: Fastest (native concurrency, minimal GC)
- Java: Fast (JIT optimization, mature runtime)
- TypeScript: Good (V8 optimization)
- Python: Slower (GIL limitations, interpreted)

## References

- [API Guide](API_GUIDE.md) - Public API documentation
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Source Code](../src/main/java/com/norikv/client/) - Implementation
