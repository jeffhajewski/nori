# NoriKV Python Client Architecture

Understanding the internal design and components of the Python client SDK.

## Table of Contents

- [Overview](#overview)
- [Component Architecture](#component-architecture)
- [Request Flow](#request-flow)
- [Asyncio Model](#asyncio-model)
- [Connection Management](#connection-management)
- [Routing & Sharding](#routing--sharding)
- [Retry Logic](#retry-logic)
- [Error Handling](#error-handling)

## Overview

The NoriKV Python client is designed as a **smart async client** that:
- Routes requests directly to the appropriate shard leader
- Maintains connection pools for efficient communication
- Implements retry logic with exponential backoff
- Tracks cluster topology changes
- Provides async/await operations with asyncio
- Optimizes for CPython runtime

### Design Principles

1. **Zero-hop routing**: Client routes directly to shard leader (no proxy)
2. **Async-first**: All I/O operations use async/await
3. **Type-safe**: Full type hints for static analysis (mypy, pyright)
4. **Observable**: Expose metrics and statistics
5. **Pythonic**: Follows PEP 8 and Python best practices

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
     ┌─────────get_channel()───┤              │
     │                         │              │
     │                    ┌────▼────┐         │
     │                    │  gRPC   │         │
     │                    │Channels │         │
     │                    └────┬────┘         │
     │                         │              │
     │                         │              │
     └─────────update_view()───┴──────────────┘
```

### Components

#### 1. NoriKVClient

**Responsibility**: Main public API and component coordination

**Key Methods**:
- `put()`, `get()`, `delete()` - Core operations (all async)
- `get_cluster_view()` - Topology information
- `on_topology_change()` - Subscribe to topology updates
- `get_stats()` - Client statistics
- `close()` - Resource cleanup
- `__aenter__()`, `__aexit__()` - Context manager support

**Location**: `norikv/client.py`

#### 2. Router

**Responsibility**: Determine which node to send requests to

**Key Functions**:
- Hash key to shard: `xxhash64(key) → jump_consistent_hash(hash, total_shards) → shard_id`
- Map shard to leader node
- Cache leader information
- Handle leader hints from `NOT_LEADER` errors

**Location**: `norikv/internal/router.py`

**Algorithm**:
```
1. Hash key using XXHash64 (seed=0) via xxhash Python package
2. Map hash to shard using Jump Consistent Hash
3. Look up shard leader in topology cache
4. Return leader's address
```

#### 3. ConnectionPool

**Responsibility**: Manage gRPC channels to cluster nodes

**Key Functions**:
- Create and cache gRPC channels per node
- Thread-safe concurrent access (asyncio locks)
- Graceful shutdown

**Location**: `norikv/internal/conn/pool.py`

**Design**:
- One gRPC `Channel` per node address
- Lazy initialization (created on first use)
- Channels reused across requests
- Automatic cleanup on client close

#### 4. RetryPolicy

**Responsibility**: Handle transient failures with backoff

**Key Functions**:
- Exponential backoff: `delay = min(initial_delay * 2^attempt, max_delay)`
- Jitter: Add randomness to avoid thundering herd
- Selective retry: Only retry transient errors
- Attempt tracking

**Location**: `norikv/internal/retry/policy.py`

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

**Location**: `norikv/internal/topology/manager.py`

**Data Structures**:
- `ClusterView`: Current cluster state (epoch, nodes, shards)
- `shard_leader_cache`: dict[int, str]
- `listeners`: list of change callbacks

## Request Flow

### PUT Request Flow

```
Client.put(key, value, options)
    │
    ├─> 1. Validate inputs (key, value not null/empty)
    │
    ├─> 2. Router.get_node_for_key(key)
    │       ├─> hash = xxhash64(key)
    │       ├─> shard_id = jump_consistent_hash(hash, total_shards)
    │       └─> leader_addr = topology_manager.get_shard_leader(shard_id)
    │
    ├─> 3. ConnectionPool.get_channel(leader_addr)
    │       └─> Return cached or create new gRPC channel
    │
    ├─> 4. RetryPolicy.execute(async lambda: {
    │       ├─> Build gRPC PutRequest
    │       ├─> await grpc_client.put(request)
    │       └─> Convert response to Version
    │   })
    │       ├─> On SUCCESS: return Version
    │       ├─> On RETRYABLE_ERROR: backoff and retry
    │       └─> On NON_RETRYABLE: raise error
    │
    └─> 5. Return Version to caller
```

### GET Request Flow

Similar to PUT, but:
- Uses `GetRequest` with consistency level
- Returns `GetResult` (value + version)
- Raises `KeyNotFoundError` on NOT_FOUND

### Error Handling in Flow

```
gRPC Status Error
    │
    ├─> convert_grpc_error()
    │   ├─> NOT_FOUND → KeyNotFoundError
    │   ├─> FAILED_PRECONDITION + "version" → VersionMismatchError
    │   ├─> UNAVAILABLE → ConnectionError
    │   └─> OTHER → NoriKVError
    │
    └─> RetryPolicy decides:
        ├─> Retryable → backoff and retry
        └─> Non-retryable → raise to caller
```

## Asyncio Model

### Async/Await API

All client operations are async coroutines:

```python
# All methods are async
async def put(self, key: str | bytes, value: str | bytes, options: PutOptions | None = None) -> Version
async def get(self, key: str | bytes, options: GetOptions | None = None) -> GetResult
async def delete(self, key: str | bytes, options: DeleteOptions | None = None) -> bool
```

### Event Loop Integration

```python
# Modern async/await
async def example():
    version = await client.put(key, value)
    result = await client.get(key)
    await client.delete(key)

# Sequential operations
v1 = await client.put("k1", "v1")
v2 = await client.put("k2", "v2")  # Waits for v1

# Concurrent operations
v1, v2 = await asyncio.gather(
    client.put("k1", "v1"),
    client.put("k2", "v2"),  # Runs concurrently
)
```

### Context Manager Support

```python
# Automatic resource cleanup
async with NoriKVClient(config) as client:
    await client.put(key, value)
# Client automatically closed

# Manual management
client = NoriKVClient(config)
await client.connect()
try:
    await client.put(key, value)
finally:
    await client.close()
```

### Error Handling

```python
try:
    result = await client.get(key)
except KeyNotFoundError:
    # Handle not found
    pass
except ConnectionError:
    # Handle connection error
    pass
except Exception as err:
    # Handle other errors
    raise
```

## Connection Management

### Channel Lifecycle

```
Node Address
    │
    ├─> First request → Create gRPC Channel
    │   ├─> Configure: credentials, options
    │   └─> Store in pool
    │
    ├─> Subsequent requests → Reuse channel
    │
    └─> Client.close() → Close all channels
        └─> Graceful shutdown with timeout
```

### Channel Configuration

```python
import grpc

channel = grpc.aio.insecure_channel(
    address,
    options=[
        ("grpc.keepalive_time_ms", 10000),
        ("grpc.keepalive_timeout_ms", 3000),
    ],
)
```

### Health Checks

- Channels automatically reconnect on failure
- gRPC handles connection health internally
- Failed requests trigger retries (via RetryPolicy)

## Routing & Sharding

### Hash Function: XXHash64

```python
import xxhash

def hash_key(key: bytes) -> int:
    hasher = xxhash.xxh64(seed=0)
    hasher.update(key)
    return hasher.intdigest()
```

**Properties**:
- Fast: Optimized C implementation
- Consistent: Same key → same hash
- Cross-SDK compatible

### Consistent Hashing: Jump Consistent Hash

```python
def jump_consistent_hash(key: int, num_buckets: int) -> int:
    b = -1
    j = 0
    while j < num_buckets:
        b = j
        key = ((key * 2862933555777941757) + 1) & 0xFFFFFFFFFFFFFFFF
        j = int(float(b + 1) * (float(1 << 31) / float((key >> 33) + 1)))
    return b
```

**Properties**:
- Minimal key movement on shard count changes
- O(log n) time complexity
- Uniform distribution

### Shard → Leader Mapping

```
shard_id → TopologyManager.get_shard_leader(shard_id) → leader_addr
```

**Leader Cache**:
- Populated from ClusterView
- Updated on topology changes
- Updated from NOT_LEADER error hints

## Retry Logic

### Exponential Backoff

```python
import random

delay = min(
    initial_delay * (2 ** attempt),
    max_delay
) + random.random() * jitter

await asyncio.sleep(delay / 1000.0)  # Convert ms to seconds
```

**Example** (initial_delay=100ms, max_delay=5s, jitter=100ms):
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

```python
class NoriKVError(Exception):
    """Base error for all NoriKV errors."""

    def __init__(self, message: str, code: str, cause: Exception | None = None):
        super().__init__(message)
        self.code = code
        self.cause = cause

class KeyNotFoundError(NoriKVError):
    """Key was not found."""

class VersionMismatchError(NoriKVError):
    """Version mismatch during CAS operation."""

class AlreadyExistsError(NoriKVError):
    """Key already exists."""

class ConnectionError(NoriKVError):
    """Connection error."""
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

1. **Hash calculation**: Fast XXHash64 via C extension
2. **Channel lookup**: O(1) dict lookup
3. **Leader cache**: O(1) dict lookup
4. **Protobuf serialization**: Native C++ implementation

### Memory Usage

- **Per client**: ~5-20 MB (depends on number of nodes)
- **Per channel**: ~1-2 MB (gRPC overhead)
- **Per request**: Minimal (garbage collected)

### CPython GIL Considerations

- **I/O operations**: Release GIL during network calls
- **Hash computation**: XXHash C extension releases GIL
- **Multiple clients**: Can run in different threads

### Connection Pooling

- Channels reused across requests
- No connection per request overhead
- HTTP/2 multiplexing

## Python-Specific Features

### Full Type Hints

```python
from norikv import NoriKVClient, GetResult, Version

client: NoriKVClient = NoriKVClient(config)
result: GetResult = await client.get(key)
version: Version = result.version
```

### Context Managers

```python
# Async context manager
async with NoriKVClient(config) as client:
    await client.put(key, value)

# Equivalent to:
client = NoriKVClient(config)
await client.__aenter__()
try:
    await client.put(key, value)
finally:
    await client.__aexit__(None, None, None)
```

### Dataclasses

```python
from dataclasses import dataclass

@dataclass
class ClientConfig:
    nodes: list[str]
    total_shards: int
    timeout: int = 5000
    retry: RetryConfig | None = None
```

### Type Guards

```python
from typing import TypeGuard

def is_bytes(value: str | bytes) -> TypeGuard[bytes]:
    return isinstance(value, bytes)

if is_bytes(value):
    # Type checker knows value is bytes here
    hasher.update(value)
```

## Asyncio Best Practices

### 1. Use async with for Cleanup

```python
async with NoriKVClient(config) as client:
    await client.put(key, value)
```

### 2. Use gather for Concurrency

```python
results = await asyncio.gather(
    client.put("k1", "v1"),
    client.put("k2", "v2"),
    client.put("k3", "v3"),
)
```

### 3. Use create_task for Background Work

```python
task = asyncio.create_task(client.put(key, value))
# Do other work
await task
```

### 4. Use Timeouts

```python
try:
    result = await asyncio.wait_for(
        client.get(key),
        timeout=5.0,
    )
except asyncio.TimeoutError:
    print("Operation timed out")
```

### 5. Handle Cancellation

```python
try:
    result = await client.get(key)
except asyncio.CancelledError:
    print("Operation cancelled")
    raise
```

## Threading Considerations

### Thread Safety

The client is **not thread-safe** by default. Each thread should have its own client instance or use explicit synchronization.

### Running in Thread Pool

```python
import asyncio
from concurrent.futures import ThreadPoolExecutor

def sync_operation(key: str, value: str):
    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)

    async def async_work():
        async with NoriKVClient(config) as client:
            await client.put(key, value)

    loop.run_until_complete(async_work())
    loop.close()

# Run in thread pool
with ThreadPoolExecutor() as executor:
    future = executor.submit(sync_operation, "key", "value")
    future.result()
```

## Performance Optimizations

### 1. Reuse Client Instances

```python
#  Good: Reuse client
class Application:
    def __init__(self):
        self.client = NoriKVClient(config)

    async def handle_request(self):
        await self.client.put(key, value)

#  Bad: Create client per request
async def handle_request():
    async with NoriKVClient(config) as client:
        await client.put(key, value)
```

### 2. Batch with gather

```python
# Concurrent operations
await asyncio.gather(
    *[client.put(f"key{i}", f"value{i}") for i in range(100)]
)
```

### 3. Use bytes Directly

```python
#  Good: No encoding overhead
await client.put(b"key", b"value")

#  Less efficient: Encoding overhead
await client.put("key", "value")
```

### 4. Monitor with Stats

```python
stats = client.get_stats()
print(f"Active connections: {stats.pool.active_connections}")
```

## References

- [API Guide](API_GUIDE.md) - Public API documentation
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Source Code](https://github.com/jeffhajewski/norikv/tree/main/sdks/python) - Implementation
