# NoriKV TypeScript Client Architecture

Understanding the internal design and components of the TypeScript client SDK.

## Table of Contents

- [Overview](#overview)
- [Component Architecture](#component-architecture)
- [Request Flow](#request-flow)
- [Async/Promise Model](#asyncpromise-model)
- [Connection Management](#connection-management)
- [Routing & Sharding](#routing--sharding)
- [Retry Logic](#retry-logic)
- [Error Handling](#error-handling)

## Overview

The NoriKV TypeScript client is designed as a **smart client** that:
- Routes requests directly to the appropriate shard leader
- Maintains connection pools for efficient communication
- Implements retry logic with exponential backoff
- Tracks cluster topology changes
- Provides Promise-based async operations
- Optimizes for V8 JavaScript engine

### Design Principles

1. **Zero-hop routing**: Client routes directly to shard leader (no proxy)
2. **Async-first**: All operations return Promises
3. **Type-safe**: Full TypeScript types for compile-time safety
4. **Observable**: Expose metrics and statistics
5. **Dual-package**: ESM + CommonJS support

## Component Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    NoriKVClient                          │
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
     │                    │  gRPC   │         │
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
- `put()`, `get()`, `delete()` - Core operations (all async)
- `getClusterView()` - Topology information
- `onTopologyChange()` - Subscribe to topology updates
- `getStats()` - Client statistics
- `close()` - Resource cleanup

**Location**: `src/client.ts`

#### 2. Router

**Responsibility**: Determine which node to send requests to

**Key Functions**:
- Hash key to shard: `xxhash64(key) → jumpConsistentHash(hash, totalShards) → shardId`
- Map shard to leader node
- Cache leader information
- Handle leader hints from `NOT_LEADER` errors

**Location**: `src/internal/router.ts`

**Algorithm**:
```
1. Hash key using XXHash64 (seed=0) via xxhash-wasm
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

**Location**: `src/internal/conn/pool.ts`

**Design**:
- One gRPC `Client` per node address
- Lazy initialization (created on first use)
- Channels reused across requests
- Automatic cleanup on client close

#### 4. RetryPolicy

**Responsibility**: Handle transient failures with backoff

**Key Functions**:
- Exponential backoff: `delay = min(initialDelay * 2^attempt, maxDelay)`
- Jitter: Add randomness to avoid thundering herd
- Selective retry: Only retry transient errors
- Attempt tracking

**Location**: `src/internal/retry/policy.ts`

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

**Location**: `src/internal/topology/manager.ts`

**Data Structures**:
- `ClusterView`: Current cluster state (epoch, nodes, shards)
- `shardLeaderCache`: Map<shardId, leaderAddr>
- `listeners`: Array of change callbacks

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
    ├─> 4. RetryPolicy.execute(async () => {
    │       ├─> Build gRPC PutRequest
    │       ├─> await grpcClient.put(request)
    │       └─> Convert response to Version
    │   })
    │       ├─> On SUCCESS: return Version
    │       ├─> On RETRYABLE_ERROR: backoff and retry
    │       └─> On NON_RETRYABLE: throw error
    │
    └─> 5. Return Version to caller
```

### GET Request Flow

Similar to PUT, but:
- Uses `GetRequest` with consistency level
- Returns `GetResult` (value + version)
- Throws `KeyNotFoundError` on NOT_FOUND

### Error Handling in Flow

```
gRPC Status Error
    │
    ├─> convertGrpcError()
    │   ├─> NOT_FOUND → KeyNotFoundError
    │   ├─> FAILED_PRECONDITION + "version" → VersionMismatchError
    │   ├─> UNAVAILABLE → ConnectionError
    │   └─> OTHER → NoriKVError
    │
    └─> RetryPolicy decides:
        ├─> Retryable → backoff and retry
        └─> Non-retryable → throw to caller
```

## Async/Promise Model

### Promise-Based API

All client operations return Promises:

```typescript
// All methods are async
async put(key: string | Uint8Array, value: string | Uint8Array, options?: PutOptions): Promise<Version>
async get(key: string | Uint8Array, options?: GetOptions): Promise<GetResult>
async delete(key: string | Uint8Array, options?: DeleteOptions): Promise<boolean>
```

### Async/Await Pattern

```typescript
// Modern async/await
async function example() {
  const version = await client.put(key, value);
  const result = await client.get(key);
  await client.delete(key);
}

// Sequential operations
const v1 = await client.put('k1', 'v1');
const v2 = await client.put('k2', 'v2'); // Waits for v1

// Concurrent operations
const [v1, v2] = await Promise.all([
  client.put('k1', 'v1'),
  client.put('k2', 'v2'), // Runs concurrently
]);
```

### Error Handling

```typescript
try {
  const result = await client.get(key);
} catch (error) {
  if (error instanceof KeyNotFoundError) {
    // Handle not found
  } else if (error instanceof ConnectionError) {
    // Handle connection error
  }
  throw error;
}
```

## Connection Management

### Channel Lifecycle

```
Node Address
    │
    ├─> First request → Create gRPC Client
    │   ├─> Configure: credentials, options
    │   └─> Store in pool
    │
    ├─> Subsequent requests → Reuse channel
    │
    └─> Client.close() → Close all channels
        └─> Graceful shutdown with timeout
```

### Channel Configuration

```typescript
const client = new grpc.Client(
  address,
  grpc.credentials.createInsecure(),
  {
    'grpc.keepalive_time_ms': 10000,
    'grpc.keepalive_timeout_ms': 3000,
  }
);
```

### Health Checks

- Channels automatically reconnect on failure
- gRPC handles connection health internally
- Failed requests trigger retries (via RetryPolicy)

## Routing & Sharding

### Hash Function: XXHash64

```typescript
import xxhash from 'xxhash-wasm';

const hash = xxhash.h64(key, 0); // seed=0
```

**Properties**:
- Fast: Optimized for V8
- Consistent: Same key → same hash
- Cross-SDK compatible

### Consistent Hashing: Jump Consistent Hash

```typescript
function jumpConsistentHash(key: bigint, numBuckets: number): number {
  let b = -1n, j = 0n;
  while (j < BigInt(numBuckets)) {
    b = j;
    key = key * 2862933555777941757n + 1n;
    j = BigInt((Number(b) + 1) * (Number((1n << 31n)) / Number((key >> 33n) + 1n)));
  }
  return Number(b);
}
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

## Retry Logic

### Exponential Backoff

```typescript
const delay = Math.min(
  initialDelay * Math.pow(2, attempt),
  maxDelay
) + Math.random() * jitter;

await new Promise(resolve => setTimeout(resolve, delay));
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

## Error Handling

### Error Hierarchy

```typescript
export class NoriKVError extends Error {
  constructor(
    message: string,
    public code: string,
    public cause?: Error
  ) {
    super(message);
    this.name = 'NoriKVError';
  }
}

export class KeyNotFoundError extends NoriKVError {}
export class VersionMismatchError extends NoriKVError {}
export class AlreadyExistsError extends NoriKVError {}
export class ConnectionError extends NoriKVError {}
```

### Error Code Mapping

| gRPC Status | NoriKV Error | Retry? |
|-------------|--------------|--------|
| NOT_FOUND | KeyNotFoundError | No |
| FAILED_PRECONDITION (version) | VersionMismatchError | No |
| FAILED_PRECONDITION (other) | NoriKVError | No |
| ALREADY_EXISTS | AlreadyExistsError | No |
| UNAVAILABLE | ConnectionError | Yes |
| DEADLINE_EXCEEDED | ConnectionError | Yes |
| ABORTED | NoriKVError | Yes |
| RESOURCE_EXHAUSTED | NoriKVError | Yes |
| INVALID_ARGUMENT | NoriKVError | No |
| PERMISSION_DENIED | NoriKVError | No |

## Performance Considerations

### Hot Paths

1. **Hash calculation**: Optimized XXHash64 via wasm
2. **Channel lookup**: O(1) Map lookup
3. **Leader cache**: O(1) Map lookup
4. **Protobuf serialization**: Native JavaScript

### Memory Usage

- **Per client**: ~1-10 MB (depends on number of nodes)
- **Per channel**: ~100 KB (gRPC overhead)
- **Per request**: Minimal (garbage collected)

### V8 Optimizations

- JIT compilation of hot paths
- Inline caching for property access
- Hidden classes for consistent object shapes

### Connection Pooling

- Channels reused across requests
- No connection per request overhead
- HTTP/2 multiplexing

## TypeScript-Specific Features

### Full Type Safety

```typescript
import { NoriKVClient, GetResult, Version } from '@norikv/client';

const client: NoriKVClient = new NoriKVClient(config);
const result: GetResult = await client.get(key);
const version: Version = result.version;
```

### Discriminated Unions

```typescript
type Result<T, E> =
  | { ok: true; value: T }
  | { ok: false; error: E };
```

### Generic Type Parameters

```typescript
async function withRetry<T>(
  operation: () => Promise<T>
): Promise<T> {
  // Implementation
}
```

## Browser Compatibility

The TypeScript SDK can run in browsers with:

1. **gRPC-Web**: Use @grpc/grpc-js polyfill
2. **Webpack 5**: Configure fallbacks for Node.js modules
3. **Buffer polyfill**: Use buffer package

```javascript
// webpack.config.js
module.exports = {
  resolve: {
    fallback: {
      buffer: require.resolve('buffer/'),
      stream: require.resolve('stream-browserify'),
    },
  },
};
```

## References

- [API Guide](API_GUIDE.md) - Public API documentation
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Source Code](https://github.com/j-haj/nori/tree/main/sdks/typescript) - Implementation
