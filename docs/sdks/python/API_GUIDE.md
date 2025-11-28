# NoriKV Python Client API Guide

Complete reference for the NoriKV Python Client SDK with async/await support.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Client Configuration](#client-configuration)
- [Core Operations](#core-operations)
- [Advanced Features](#advanced-features)
- [Vector Operations](#vector-operations)
- [Error Handling](#error-handling)
- [Best Practices](#best-practices)

## Installation

```bash
pip install norikv
# or
poetry add norikv
# or
pipenv install norikv
```

**Requirements:**
- Python 3.9 or higher
- asyncio support

## Quick Start

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
        version = await client.put("user:alice", "Alice")

        # Get the value
        result = await client.get("user:alice")
        print(f"Value: {result.value.decode()}")

        # Delete
        await client.delete("user:alice")

if __name__ == "__main__":
    asyncio.run(main())
```

## Client Configuration

### Basic Configuration

```python
from norikv import NoriKVClient, ClientConfig

config = ClientConfig(
    nodes=["node1:9001", "node2:9001"],
    total_shards=1024,
    timeout=5000,
)

client = NoriKVClient(config)
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `nodes` | `list[str]` | **Required** | List of node addresses (host:port) |
| `total_shards` | `int` | **Required** | Total number of shards in cluster |
| `timeout` | `int` | 5000 | Request timeout in milliseconds |
| `retry` | `RetryConfig` | See below | Retry policy configuration |

### Retry Configuration

```python
from norikv import RetryConfig

retry_config = RetryConfig(
    max_attempts=10,
    initial_delay_ms=100,
    max_delay_ms=5000,
    jitter_ms=100,
)

config = ClientConfig(
    nodes=["localhost:9001"],
    total_shards=1024,
    retry=retry_config,
)
```

**Retry Behavior:**
- Retries transient errors: `Unavailable`, `Aborted`, `DeadlineExceeded`
- Does NOT retry: `InvalidArgument`, `NotFound`, `FailedPrecondition`
- Uses exponential backoff with jitter

## Core Operations

### PUT - Write Data

#### Basic PUT

```python
key = "user:123"
value = json.dumps({"name": "Alice"})

version = await client.put(key, value)
print(f"Written at version: {version}")
```

#### PUT with Options

```python
from norikv import PutOptions

options = PutOptions(
    ttl_ms=60000,                    # TTL: 60 seconds
    idempotency_key="order-12345",   # Idempotency key
    if_match_version=expected_version, # CAS
)

version = await client.put(key, value, options)
```

**PutOptions Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `ttl_ms` | `int \| None` | Time-to-live in milliseconds |
| `idempotency_key` | `str \| None` | Key for idempotent operations |
| `if_match_version` | `Version \| None` | Expected version for CAS |

### GET - Read Data

#### Basic GET

```python
result = await client.get("user:123")

value = result.value.decode("utf-8")
version = result.version
```

#### GET with Consistency Level

```python
from norikv import GetOptions, ConsistencyLevel

options = GetOptions(
    consistency=ConsistencyLevel.LINEARIZABLE,
)

result = await client.get(key, options)
```

**Consistency Levels:**

| Level | Description | Use Case |
|-------|-------------|----------|
| `LEASE` | Default, lease-based read | Most operations (fast, usually consistent) |
| `LINEARIZABLE` | Strictest, always up-to-date | Critical reads requiring absolute consistency |
| `STALE_OK` | May return stale data | Read-heavy workloads, caching |

### DELETE - Remove Data

#### Basic DELETE

```python
deleted = await client.delete("user:123")
print(f"Deleted: {deleted}")
```

#### DELETE with Options

```python
from norikv import DeleteOptions

options = DeleteOptions(
    idempotency_key="delete-order-12345",
    if_match_version=expected_version,
)

await client.delete(key, options)
```

## Advanced Features

### Compare-And-Swap (CAS)

Optimistic concurrency control using version matching:

```python
# Read current value
current = await client.get(key)
value = int(current.value.decode())

# Update with CAS
new_value = str(value + 1)
try:
    await client.put(
        key,
        new_value,
        PutOptions(if_match_version=current.version),
    )
    print("CAS succeeded")
except VersionMismatchError:
    print("CAS failed - version changed")
```

### Idempotent Operations

Safe retries using idempotency keys:

```python
import uuid

idempotency_key = f"payment-{uuid.uuid4()}"

# First attempt
v1 = await client.put(key, value, PutOptions(idempotency_key=idempotency_key))

# Retry with same key (safe - returns same version)
v2 = await client.put(key, value, PutOptions(idempotency_key=idempotency_key))

print(v1 == v2)  # True
```

### Time-To-Live (TTL)

Automatic expiration:

```python
await client.put(key, value, PutOptions(ttl_ms=60000))  # Expires in 60 seconds

# Key automatically deleted after TTL
await asyncio.sleep(61)

try:
    await client.get(key)
except KeyNotFoundError:
    print("Key expired")
```

### Cluster Topology

Monitor cluster changes:

```python
# Get current cluster view
view = client.get_cluster_view()
if view:
    print(f"Cluster epoch: {view.epoch}")
    print(f"Nodes: {len(view.nodes)}")

# Subscribe to topology changes
def on_topology_change(event):
    print("Topology changed!")
    print(f"Previous epoch: {event.previous_epoch}")
    print(f"Current epoch: {event.current_epoch}")
    print(f"Added nodes: {event.added_nodes}")
    print(f"Removed nodes: {event.removed_nodes}")

unsubscribe = client.on_topology_change(on_topology_change)

# Later: unsubscribe
unsubscribe()
```

### Client Statistics

Monitor client performance:

```python
stats = client.get_stats()

print(f"Active connections: {stats.pool.active_connections}")
print(f"Total nodes: {stats.router.total_nodes}")
print(f"Cached leaders: {stats.topology.cached_leaders}")
```

## Vector Operations

NoriKV supports vector similarity search for building AI/ML applications, recommendation systems, and semantic search.

### Creating a Vector Index

Before inserting vectors, create an index with your configuration:

```python
from norikv import DistanceFunction, VectorIndexType

created = await client.vector_create_index(
    "embeddings",                    # namespace
    1536,                            # dimensions
    DistanceFunction.COSINE,         # distance function
    VectorIndexType.HNSW,            # index type
)

if created:
    print("Index created")
else:
    print("Index already exists")
```

#### With Options

```python
from norikv import CreateVectorIndexOptions

options = CreateVectorIndexOptions(
    idempotency_key="create-embeddings-index",
)

created = await client.vector_create_index(
    "embeddings",
    1536,
    DistanceFunction.COSINE,
    VectorIndexType.HNSW,
    options,
)
```

### Distance Functions

| Enum Value | Description | Use Case |
|------------|-------------|----------|
| `DistanceFunction.EUCLIDEAN` | L2 distance | General purpose |
| `DistanceFunction.COSINE` | Cosine similarity (1 - cos) | Text embeddings, normalized vectors |
| `DistanceFunction.INNER_PRODUCT` | Negative inner product | Maximum inner product search |

### Index Types

| Enum Value | Description | Trade-off |
|------------|-------------|-----------|
| `VectorIndexType.BRUTE_FORCE` | Exact linear scan | Exact results, O(n) complexity |
| `VectorIndexType.HNSW` | Hierarchical Navigable Small World | Approximate, O(log n) complexity |

### Inserting Vectors

```python
embedding = await get_embedding("Hello world")

version = await client.vector_insert(
    "embeddings",    # namespace
    "doc-123",       # unique ID
    embedding,       # list[float]
)

print(f"Inserted at version: {version}")
```

#### With Options

```python
from norikv import VectorInsertOptions

options = VectorInsertOptions(
    idempotency_key="insert-doc-123",
)

version = await client.vector_insert("embeddings", "doc-123", embedding, options)
```

### Searching for Similar Vectors

```python
query = await get_embedding("Find similar documents")

result = await client.vector_search(
    "embeddings",    # namespace
    query,           # query vector
    10,              # k nearest neighbors
)

print(f"Search took {result.search_time_us}us")

for match in result.matches:
    print(f"ID: {match.id}, Distance: {match.distance}")
```

#### With Options

```python
from norikv import VectorSearchOptions, VectorSearchResult

options = VectorSearchOptions(
    include_vectors=True,  # include vector data in results
)

result: VectorSearchResult = await client.vector_search(
    "embeddings",
    query,
    10,
    options,
)

for match in result.matches:
    print(f"ID: {match.id}, Distance: {match.distance}")
    if match.vector:
        print(f"Vector dims: {len(match.vector)}")
```

### Getting a Vector by ID

```python
vector = await client.vector_get("embeddings", "doc-123")

if vector:
    print(f"Vector has {len(vector)} dimensions")
else:
    print("Vector not found")
```

### Deleting Vectors

```python
deleted = await client.vector_delete("embeddings", "doc-123")

if deleted:
    print("Vector deleted")
else:
    print("Vector not found")
```

#### With Options

```python
from norikv import VectorDeleteOptions

options = VectorDeleteOptions(
    idempotency_key="delete-doc-123",
)

deleted = await client.vector_delete("embeddings", "doc-123", options)
```

### Dropping a Vector Index

```python
dropped = await client.vector_drop_index("embeddings")

if dropped:
    print("Index dropped")
else:
    print("Index did not exist")
```

### Complete Vector Example

```python
import asyncio
from norikv import (
    NoriKVClient,
    ClientConfig,
    DistanceFunction,
    VectorIndexType,
)

async def main():
    config = ClientConfig(
        nodes=["localhost:9001"],
        total_shards=1024,
    )

    async with NoriKVClient(config) as client:
        # Create index
        await client.vector_create_index(
            "products",
            768,
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
        )

        # Insert product embeddings
        product_embedding = await get_product_embedding("Red running shoes")
        await client.vector_insert("products", "prod-001", product_embedding)

        # Search for similar products
        query_embedding = await get_product_embedding("Athletic footwear")
        results = await client.vector_search("products", query_embedding, 5)

        print("Similar products:")
        for match in results.matches:
            print(f"  {match.id} (distance: {match.distance:.4f})")

        # Cleanup
        await client.vector_delete("products", "prod-001")
        await client.vector_drop_index("products")


async def get_product_embedding(text: str) -> list[float]:
    # Call your embedding model here
    return [0.0] * 768


if __name__ == "__main__":
    asyncio.run(main())
```

## Error Handling

### Error Types

```python
from norikv import (
    KeyNotFoundError,
    VersionMismatchError,
    AlreadyExistsError,
    ConnectionError,
    NoriKVError,
)
```

### Handling Specific Errors

```python
try:
    result = await client.get(key)
except KeyNotFoundError:
    print("Key not found")
except ConnectionError as err:
    print(f"Connection error: {err}")
except NoriKVError as err:
    print(f"Error: {err.code} - {err}")
```

### Retry Pattern

```python
from typing import TypeVar, Callable, Awaitable

T = TypeVar("T")

async def retry_operation(
    operation: Callable[[], Awaitable[T]],
    max_attempts: int = 3,
) -> T:
    for attempt in range(1, max_attempts + 1):
        try:
            return await operation()
        except ConnectionError as err:
            if attempt == max_attempts:
                raise  # Give up

            # Exponential backoff
            await asyncio.sleep((2 ** attempt) * 0.1)

    raise RuntimeError("Unreachable")

# Usage
result = await retry_operation(lambda: client.put(key, value))
```

### Graceful Degradation

```python
async def get_with_fallback(
    client: NoriKVClient,
    key: str,
    default_value: str,
) -> str:
    try:
        result = await client.get(key)
        return result.value.decode()
    except Exception as err:
        print(f"Failed to get key, using default: {err}")
        return default_value
```

## Best Practices

### 1. Use Context Managers

```python
#  Good: Context manager ensures cleanup
async with NoriKVClient(config) as client:
    await client.put(key, value)

#  Bad: Manual cleanup required
client = NoriKVClient(config)
await client.connect()
await client.put(key, value)
await client.close()
```

### 2. Reuse Client Instances

```python
#  Good: Single client instance
client: NoriKVClient | None = None

async def init():
    global client
    config = ClientConfig(nodes=["localhost:9001"], total_shards=1024)
    client = NoriKVClient(config)
    await client.connect()

#  Bad: Creating client per request
async def handle_request():
    async with NoriKVClient(config) as client:
        await client.put(key, value)
    # Closes connections!
```

### 3. Use Type Hints

```python
from typing import Any
from dataclasses import dataclass

@dataclass
class UserData:
    id: str
    name: str
    email: str

async def update_user(user_id: str, data: UserData) -> Version:
    key = f"user:{user_id}"
    value = json.dumps(data.__dict__)

    options = PutOptions(ttl_ms=3600000)
    return await client.put(key, value, options)
```

### 4. Handle Errors Properly

```python
async def safe_get(key: str) -> str | None:
    try:
        result = await client.get(key)
        return result.value.decode()
    except KeyNotFoundError:
        return None
```

### 5. Use asyncio Consistently

```python
#  Good: Clean async/await
async def process_user(user_id: str):
    user_data = await client.get(f"user:{user_id}")
    processed = await process_data(user_data.value)
    await client.put(f"processed:{user_id}", processed)

#  Bad: Mixing sync and async
def process_user_bad(user_id: str):
    loop = asyncio.get_event_loop()
    user_data = loop.run_until_complete(client.get(f"user:{user_id}"))
    # This blocks the event loop
```

### 6. Use Idempotency Keys for Important Operations

```python
async def create_order(order_id: str, data: dict[str, Any]):
    await client.put(
        f"order:{order_id}",
        json.dumps(data),
        PutOptions(idempotency_key=f"create-order-{order_id}"),
    )
```

### 7. Choose Appropriate Consistency

```python
# For critical reads
result = await client.get(
    key,
    GetOptions(consistency=ConsistencyLevel.LINEARIZABLE),
)

# For cache-like reads
result = await client.get(
    key,
    GetOptions(consistency=ConsistencyLevel.STALE_OK),
)
```

### 8. Use Proper Encoding

```python
# Always specify encoding
value_bytes = "Hello, World!".encode("utf-8")
await client.put(key, value_bytes)

result = await client.get(key)
text = result.value.decode("utf-8")
```

## Performance Tips

### 1. Batch Operations with gather

```python
# Process multiple operations concurrently
await asyncio.gather(
    *[client.put(key, value) for key in keys]
)
```

### 2. Connection Pooling (Automatic)

The client maintains connection pools internally - no external pooling needed.

### 3. Avoid Creating Clients Per Request

Reuse client instances across requests for better performance.

### 4. Use Appropriate Value Sizes

- Optimal: 100 bytes - 10 KB
- Maximum: Limited by memory and network

### 5. Monitor Performance

```python
import time

start = time.time()
await client.put(key, value)
duration = time.time() - start
print(f"PUT took {duration * 1000:.2f}ms")
```

## Complete Example

```python
import asyncio
import json
from typing import Any

from norikv import (
    NoriKVClient,
    ClientConfig,
    PutOptions,
    GetOptions,
    ConsistencyLevel,
    RetryConfig,
    VersionMismatchError,
)

async def main():
    # Configure with retry policy
    config = ClientConfig(
        nodes=["localhost:9001", "localhost:9002"],
        total_shards=1024,
        timeout=5000,
        retry=RetryConfig(
            max_attempts=5,
            initial_delay_ms=100,
            max_delay_ms=2000,
        ),
    )

    async with NoriKVClient(config) as client:
        # Write with TTL and idempotency
        key = "session:abc123"
        value = json.dumps({"userId": 42})

        put_opts = PutOptions(
            ttl_ms=3600000,  # 1 hour
            idempotency_key="session-create-abc123",
        )

        version = await client.put(key, value.encode(), put_opts)
        print(f"Written: {version}")

        # Read with linearizable consistency
        get_opts = GetOptions(
            consistency=ConsistencyLevel.LINEARIZABLE,
        )

        result = await client.get(key, get_opts)
        print(f"Read: {result.value.decode()}")

        # Update with CAS
        new_value = json.dumps({"userId": 42, "active": True})

        try:
            await client.put(
                key,
                new_value.encode(),
                PutOptions(if_match_version=result.version),
            )
            print("CAS succeeded")
        except VersionMismatchError:
            print("CAS failed - retry needed")

        # Monitor topology
        def on_change(event):
            print(f"Cluster changed: epoch {event.current_epoch}")

        client.on_topology_change(on_change)

        # Get statistics
        stats = client.get_stats()
        print(f"Stats: {stats}")

if __name__ == "__main__":
    asyncio.run(main())
```

## Async Context Manager Pattern

```python
# Proper pattern for long-lived applications
class Application:
    def __init__(self):
        self.client: NoriKVClient | None = None

    async def startup(self):
        config = ClientConfig(
            nodes=["localhost:9001"],
            total_shards=1024,
        )
        self.client = NoriKVClient(config)
        await self.client.connect()

    async def shutdown(self):
        if self.client:
            await self.client.close()

    async def handle_request(self, key: str, value: str):
        if not self.client:
            raise RuntimeError("Client not initialized")

        await self.client.put(key, value)

# Usage
app = Application()
await app.startup()

try:
    await app.handle_request("key", "value")
finally:
    await app.shutdown()
```

## Next Steps

- [Architecture Guide](ARCHITECTURE.md) - Understanding client internals
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Solving common issues
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
- [Examples](https://github.com/jeffhajewski/norikv/tree/main/sdks/python/examples) - Working code samples
