# NoriKV Python Client Advanced Patterns

Complex real-world usage patterns and production-ready design examples with async/await.

## Table of Contents

- [Distributed Counter](#distributed-counter)
- [Session Management](#session-management)
- [Inventory Management](#inventory-management)
- [Caching Layer](#caching-layer)
- [Rate Limiting](#rate-limiting)
- [Leader Election](#leader-election)
- [Event Sourcing](#event-sourcing)
- [Multi-Tenancy](#multi-tenancy)
- [Semantic Search](#semantic-search)
- [Recommendation System](#recommendation-system)
- [Document Deduplication](#document-deduplication)

## Distributed Counter

Implement a high-throughput distributed counter with sharding to reduce contention.

### Basic Counter

```python
import asyncio
from norikv import NoriKVClient, VersionMismatchError, KeyNotFoundError

class DistributedCounter:
    def __init__(self, client: NoriKVClient, counter_name: str):
        self.client = client
        self.key = counter_name
        self.max_retries = 20

    async def initialize(self) -> None:
        """Initialize counter to 0 if not exists."""
        try:
            await self.client.get(self.key)
        except KeyNotFoundError:
            try:
                await self.client.put(self.key, b"0")
            except Exception:
                # Ignore - someone else may have initialized
                pass

    async def increment(self) -> int:
        """Increment counter by 1."""
        return await self.increment_by(1)

    async def increment_by(self, delta: int) -> int:
        """Increment counter by delta."""
        for attempt in range(self.max_retries):
            try:
                # Read current value
                current = await self.client.get(self.key)
                value = int(current.value.decode())

                # Increment
                new_value = value + delta

                # CAS write
                await self.client.put(
                    self.key,
                    str(new_value).encode(),
                    PutOptions(if_match_version=current.version),
                )

                return new_value

            except VersionMismatchError:
                if attempt == self.max_retries - 1:
                    raise RuntimeError(
                        f"Failed to increment after {self.max_retries} attempts"
                    )

                # Exponential backoff with jitter
                backoff = min(2 ** attempt, 1000) / 1000.0
                jitter = random.random() * 0.1
                await asyncio.sleep(backoff + jitter)

        raise RuntimeError("Should not reach here")

    async def get(self) -> int:
        """Get current counter value."""
        result = await self.client.get(self.key)
        return int(result.value.decode())
```

### Sharded Counter (High Throughput)

For very high write rates, distribute writes across multiple shards:

```python
import random
from typing import List

class ShardedCounter:
    def __init__(
        self,
        client: NoriKVClient,
        name: str,
        num_shards: int = 10,
    ):
        self.client = client
        self.name = name
        self.num_shards = num_shards

    def _shard_key(self, shard_id: int) -> str:
        return f"{self.name}:shard:{shard_id}"

    def _random_shard(self) -> int:
        return random.randint(0, self.num_shards - 1)

    async def increment(self) -> None:
        """Increment counter by 1 on a random shard."""
        shard_id = self._random_shard()
        key = self._shard_key(shard_id)

        max_retries = 10
        for attempt in range(max_retries):
            try:
                # Read current value
                current_value = 0
                current_version = None

                try:
                    result = await self.client.get(key)
                    current_value = int(result.value.decode())
                    current_version = result.version
                except KeyNotFoundError:
                    pass

                # Increment
                new_value = current_value + 1
                options = (
                    PutOptions(if_match_version=current_version)
                    if current_version
                    else PutOptions()
                )

                await self.client.put(key, str(new_value).encode(), options)
                return

            except VersionMismatchError:
                # Exponential backoff
                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Increment failed after retries")

    async def get(self) -> int:
        """Get total count across all shards."""
        # Fetch all shards concurrently
        tasks = [
            self._get_shard(i) for i in range(self.num_shards)
        ]
        values = await asyncio.gather(*tasks)
        return sum(values)

    async def _get_shard(self, shard_id: int) -> int:
        try:
            result = await self.client.get(self._shard_key(shard_id))
            return int(result.value.decode())
        except KeyNotFoundError:
            return 0
```

### Usage Example

```python
async def main():
    async with NoriKVClient(config) as client:
        # Basic counter
        counter = DistributedCounter(client, "api:requests")
        await counter.initialize()

        new_count = await counter.increment()
        print(f"Request count: {new_count}")

        # Sharded counter for high throughput
        sharded = ShardedCounter(client, "page:views", num_shards=20)

        # Many concurrent increments
        await asyncio.gather(
            *[sharded.increment() for _ in range(1000)]
        )

        total = await sharded.get()
        print(f"Total views: {total}")
```

## Session Management

Implement secure session storage with automatic expiration using TTL.

### Session Manager

```python
import json
import uuid
from dataclasses import dataclass, asdict
from typing import Optional, Callable

@dataclass
class SessionData:
    user_id: str
    email: str
    roles: list[str]
    created_at: int
    last_accessed_at: int

class SessionManager:
    def __init__(self, client: NoriKVClient, ttl_ms: int = 3600000):
        self.client = client
        self.ttl_ms = ttl_ms  # Default: 1 hour
        self.prefix = "session"

    def _session_key(self, session_id: str) -> str:
        return f"{self.prefix}:{session_id}"

    async def create(
        self,
        user_id: str,
        email: str,
        roles: list[str],
    ) -> str:
        """Create a new session."""
        session_id = str(uuid.uuid4())
        key = self._session_key(session_id)

        data = SessionData(
            user_id=user_id,
            email=email,
            roles=roles,
            created_at=int(time.time() * 1000),
            last_accessed_at=int(time.time() * 1000),
        )

        await self.client.put(
            key,
            json.dumps(asdict(data)).encode(),
            PutOptions(
                ttl_ms=self.ttl_ms,
                idempotency_key=f"session-create-{session_id}",
            ),
        )

        return session_id

    async def get(self, session_id: str) -> Optional[SessionData]:
        """Get session data, updating last accessed time."""
        try:
            result = await self.client.get(self._session_key(session_id))
            data_dict = json.loads(result.value.decode())
            data = SessionData(**data_dict)

            # Update last accessed time
            data.last_accessed_at = int(time.time() * 1000)
            await self.client.put(
                self._session_key(session_id),
                json.dumps(asdict(data)).encode(),
                PutOptions(ttl_ms=self.ttl_ms),  # Reset TTL
            )

            return data

        except KeyNotFoundError:
            return None

    async def update(
        self,
        session_id: str,
        update_fn: Callable[[SessionData], SessionData],
    ) -> bool:
        """Update session with CAS."""
        max_retries = 5

        for attempt in range(max_retries):
            try:
                result = await self.client.get(self._session_key(session_id))
                data_dict = json.loads(result.value.decode())
                data = SessionData(**data_dict)

                # Apply update
                updated = update_fn(data)
                updated.last_accessed_at = int(time.time() * 1000)

                # CAS write
                await self.client.put(
                    self._session_key(session_id),
                    json.dumps(asdict(updated)).encode(),
                    PutOptions(
                        if_match_version=result.version,
                        ttl_ms=self.ttl_ms,
                    ),
                )

                return True

            except KeyNotFoundError:
                return False

            except VersionMismatchError:
                await asyncio.sleep((2 ** attempt) * 0.01)

        return False

    async def destroy(self, session_id: str) -> bool:
        """Delete session."""
        try:
            return await self.client.delete(
                self._session_key(session_id),
                DeleteOptions(idempotency_key=f"session-destroy-{session_id}"),
            )
        except KeyNotFoundError:
            return False

    async def validate(self, session_id: str) -> bool:
        """Check if session exists."""
        try:
            await self.client.get(self._session_key(session_id))
            return True
        except KeyNotFoundError:
            return False
```

### Usage Example

```python
import time

sessions = SessionManager(client, ttl_ms=1800000)  # 30 min

# Create session
session_id = await sessions.create(
    "user123",
    "alice@example.com",
    ["user", "admin"],
)

# Middleware: validate session
async def auth_middleware(request):
    session_id = request.cookies.get("session_id")

    session_data = await sessions.get(session_id)
    if not session_data:
        raise Unauthorized("Invalid session")

    request.user = session_data

# Update session
await sessions.update(
    session_id,
    lambda data: SessionData(
        **{**asdict(data), "roles": data.roles + ["premium"]}
    ),
)

# Destroy session
await sessions.destroy(session_id)
```

## Inventory Management

Prevent overselling with optimistic concurrency control.

### Inventory System

```python
from dataclasses import dataclass
import time

@dataclass
class InventoryItem:
    sku: str
    quantity: int
    reserved: int
    last_updated: int

class InventoryManager:
    def __init__(self, client: NoriKVClient):
        self.client = client
        self.prefix = "inventory"

    def _item_key(self, sku: str) -> str:
        return f"{self.prefix}:{sku}"

    def _reservation_key(self, reservation_id: str) -> str:
        return f"{self.prefix}:reservation:{reservation_id}"

    async def create_item(self, sku: str, initial_quantity: int) -> None:
        """Create inventory item."""
        item = InventoryItem(
            sku=sku,
            quantity=initial_quantity,
            reserved=0,
            last_updated=int(time.time() * 1000),
        )

        await self.client.put(
            self._item_key(sku),
            json.dumps(asdict(item)).encode(),
        )

    async def reserve(self, sku: str, quantity: int) -> str:
        """Reserve quantity for purchase."""
        reservation_id = str(uuid.uuid4())
        max_retries = 10

        for attempt in range(max_retries):
            try:
                # Read current inventory
                result = await self.client.get(self._item_key(sku))
                item_dict = json.loads(result.value.decode())
                item = InventoryItem(**item_dict)

                # Check availability
                available = item.quantity - item.reserved
                if available < quantity:
                    raise ValueError(
                        f"Insufficient inventory: need {quantity}, have {available}"
                    )

                # Reserve quantity
                item.reserved += quantity
                item.last_updated = int(time.time() * 1000)

                # CAS write
                await self.client.put(
                    self._item_key(sku),
                    json.dumps(asdict(item)).encode(),
                    PutOptions(if_match_version=result.version),
                )

                # Store reservation
                reservation = {
                    "sku": sku,
                    "quantity": quantity,
                    "created_at": int(time.time() * 1000),
                }
                await self.client.put(
                    self._reservation_key(reservation_id),
                    json.dumps(reservation).encode(),
                    PutOptions(ttl_ms=600000),  # 10 min expiration
                )

                return reservation_id

            except VersionMismatchError:
                if attempt == max_retries - 1:
                    raise RuntimeError("Failed to reserve after retries")

                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Should not reach here")

    async def commit(self, reservation_id: str) -> None:
        """Commit reservation (finalize purchase)."""
        max_retries = 10

        # Get reservation details
        reservation_key = self._reservation_key(reservation_id)
        res_result = await self.client.get(reservation_key)
        reservation = json.loads(res_result.value.decode())

        for attempt in range(max_retries):
            try:
                # Read current inventory
                result = await self.client.get(self._item_key(reservation["sku"]))
                item_dict = json.loads(result.value.decode())
                item = InventoryItem(**item_dict)

                # Commit: reduce both quantity and reserved
                item.quantity -= reservation["quantity"]
                item.reserved -= reservation["quantity"]
                item.last_updated = int(time.time() * 1000)

                # CAS write
                await self.client.put(
                    self._item_key(reservation["sku"]),
                    json.dumps(asdict(item)).encode(),
                    PutOptions(if_match_version=result.version),
                )

                # Delete reservation
                await self.client.delete(reservation_key)
                return

            except VersionMismatchError:
                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Failed to commit reservation")

    async def release(self, reservation_id: str) -> None:
        """Release reservation (cancel purchase)."""
        max_retries = 10

        # Get reservation details
        reservation_key = self._reservation_key(reservation_id)
        res_result = await self.client.get(reservation_key)
        reservation = json.loads(res_result.value.decode())

        for attempt in range(max_retries):
            try:
                # Read current inventory
                result = await self.client.get(self._item_key(reservation["sku"]))
                item_dict = json.loads(result.value.decode())
                item = InventoryItem(**item_dict)

                # Release: reduce reserved only
                item.reserved -= reservation["quantity"]
                item.last_updated = int(time.time() * 1000)

                # CAS write
                await self.client.put(
                    self._item_key(reservation["sku"]),
                    json.dumps(asdict(item)).encode(),
                    PutOptions(if_match_version=result.version),
                )

                # Delete reservation
                await self.client.delete(reservation_key)
                return

            except VersionMismatchError:
                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Failed to release reservation")

    async def get_available(self, sku: str) -> int:
        """Get available quantity."""
        result = await self.client.get(self._item_key(sku))
        item_dict = json.loads(result.value.decode())
        item = InventoryItem(**item_dict)
        return item.quantity - item.reserved
```

### Usage Example

```python
inventory = InventoryManager(client)

# Initialize inventory
await inventory.create_item("SKU-12345", 100)

# Purchase flow
async def purchase_item(sku: str, quantity: int):
    # 1. Reserve inventory
    reservation_id = await inventory.reserve(sku, quantity)

    try:
        # 2. Process payment
        await process_payment()

        # 3. Commit reservation
        await inventory.commit(reservation_id)
        print("Purchase successful")

    except Exception as err:
        # 4. Release reservation on failure
        await inventory.release(reservation_id)
        print(f"Purchase failed: {err}")
        raise

# Check availability
available = await inventory.get_available("SKU-12345")
print(f"Available: {available}")
```

## Caching Layer

Implement a write-through cache with automatic invalidation.

### Cache Implementation

```python
from typing import TypeVar, Generic, Callable, Optional
from dataclasses import dataclass
import time

T = TypeVar("T")

@dataclass
class CacheEntry(Generic[T]):
    value: T
    version: Version
    cached_at: int

class CachedClient(Generic[T]):
    def __init__(
        self,
        client: NoriKVClient,
        ttl_ms: int = 60000,
        max_size: int = 1000,
    ):
        self.client = client
        self.cache: dict[str, CacheEntry[T]] = {}
        self.ttl_ms = ttl_ms
        self.max_size = max_size

    async def get(
        self,
        key: str,
        parser: Callable[[bytes], T],
    ) -> T:
        """Get value with caching."""
        # Check cache
        cached = self.cache.get(key)
        now = int(time.time() * 1000)

        if cached and (now - cached.cached_at) < self.ttl_ms:
            return cached.value

        # Cache miss - fetch from NoriKV
        result = await self.client.get(key)
        value = parser(result.value)

        # Update cache
        self._set_cache(key, CacheEntry(
            value=value,
            version=result.version,
            cached_at=now,
        ))

        return value

    async def put(
        self,
        key: str,
        value: T,
        serializer: Callable[[T], bytes],
    ) -> Version:
        """Write-through to NoriKV."""
        # Write-through to NoriKV
        version = await self.client.put(key, serializer(value))

        # Update cache
        self._set_cache(key, CacheEntry(
            value=value,
            version=version,
            cached_at=int(time.time() * 1000),
        ))

        return version

    async def delete(self, key: str) -> bool:
        """Delete with cache invalidation."""
        # Invalidate cache
        self.cache.pop(key, None)

        # Delete from NoriKV
        return await self.client.delete(key)

    def invalidate(self, key: str) -> None:
        """Invalidate cache entry."""
        self.cache.pop(key, None)

    def invalidate_all(self) -> None:
        """Invalidate all cache entries."""
        self.cache.clear()

    def _set_cache(self, key: str, entry: CacheEntry[T]) -> None:
        """LRU eviction."""
        if len(self.cache) >= self.max_size:
            # Remove oldest entry (first key)
            first_key = next(iter(self.cache))
            self.cache.pop(first_key)

        self.cache[key] = entry

    def get_stats(self) -> dict:
        return {
            "size": len(self.cache),
            "max_size": self.max_size,
            "ttl_ms": self.ttl_ms,
        }
```

### Usage Example

```python
@dataclass
class UserProfile:
    id: str
    name: str
    email: str

cache = CachedClient[UserProfile](client, ttl_ms=60000, max_size=1000.md)

# Get with automatic caching
profile = await cache.get(
    "user:123",
    lambda b: UserProfile(**json.loads(b.decode())),
)

# Write-through cache
await cache.put(
    "user:123",
    UserProfile(id="123", name="Alice", email="alice@example.com"),
    lambda p: json.dumps(asdict(p)).encode(),
)

# Manual invalidation
cache.invalidate("user:123")

# Check stats
stats = cache.get_stats()
print(f"Cache stats: {stats}")
```

## Rate Limiting

Implement sliding window rate limiting for API throttling.

### Rate Limiter

```python
from dataclasses import dataclass

@dataclass
class RateLimitConfig:
    max_requests: int
    window_ms: int

class RateLimiter:
    def __init__(self, client: NoriKVClient):
        self.client = client
        self.prefix = "ratelimit"

    def _key(self, identifier: str) -> str:
        return f"{self.prefix}:{identifier}"

    async def check_limit(
        self,
        identifier: str,
        config: RateLimitConfig,
    ) -> dict:
        """Check if request is within rate limit."""
        key = self._key(identifier)
        now = int(time.time() * 1000)
        window_start = now - config.window_ms

        max_retries = 5
        for attempt in range(max_retries):
            try:
                timestamps: list[int] = []
                version = None

                # Read current timestamps
                try:
                    result = await self.client.get(key)
                    timestamps = json.loads(result.value.decode())
                    version = result.version
                except KeyNotFoundError:
                    pass

                # Remove old timestamps outside window
                timestamps = [ts for ts in timestamps if ts > window_start]

                # Check if limit exceeded
                allowed = len(timestamps) < config.max_requests

                if allowed:
                    # Add current timestamp
                    timestamps.append(now)

                    # Save updated timestamps
                    options = (
                        PutOptions(
                            if_match_version=version,
                            ttl_ms=config.window_ms,
                        )
                        if version
                        else PutOptions(ttl_ms=config.window_ms)
                    )

                    await self.client.put(
                        key,
                        json.dumps(timestamps).encode(),
                        options,
                    )

                return {
                    "allowed": allowed,
                    "remaining": max(0, config.max_requests - len(timestamps)),
                    "reset_at": (
                        min(timestamps) + config.window_ms
                        if timestamps
                        else now + config.window_ms
                    ),
                }

            except VersionMismatchError:
                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Rate limit check failed after retries")

    async def reset(self, identifier: str) -> None:
        """Reset rate limit for identifier."""
        await self.client.delete(self._key(identifier))
```

### FastAPI Middleware Example

```python
from fastapi import FastAPI, Request, HTTPException
from fastapi.responses import JSONResponse

app = FastAPI()
rate_limiter = RateLimiter(client)

@app.middleware("http")
async def rate_limit_middleware(request: Request, call_next):
    # Use IP address or user ID as identifier
    identifier = request.client.host

    config = RateLimitConfig(
        max_requests=100,
        window_ms=60000,  # 100 requests per minute
    )

    try:
        result = await rate_limiter.check_limit(identifier, config)

        # Set rate limit headers
        response = await call_next(request)
        response.headers["X-RateLimit-Limit"] = str(config.max_requests)
        response.headers["X-RateLimit-Remaining"] = str(result["remaining"])
        response.headers["X-RateLimit-Reset"] = str(result["reset_at"])

        if not result["allowed"]:
            return JSONResponse(
                status_code=429,
                content={
                    "error": "Too Many Requests",
                    "retry_after": (result["reset_at"] - int(time.time() * 1000)) // 1000,
                },
            )

        return response

    except Exception as err:
        print(f"Rate limit error: {err}")
        return await call_next(request)  # Fail open
```

## Leader Election

Implement distributed leader election with automatic failover.

### Leader Election

```python
class LeaderElection:
    def __init__(
        self,
        client: NoriKVClient,
        election_name: str,
        node_id: str,
        lease_ttl_ms: int = 10000,
    ):
        self.client = client
        self.name = election_name
        self.node_id = node_id
        self.lease_ttl_ms = lease_ttl_ms
        self.running = False
        self.is_leader = False
        self.heartbeat_task: Optional[asyncio.Task] = None

    def _leader_key(self) -> str:
        return f"leader:{self.name}"

    async def start(
        self,
        on_became_leader: Optional[Callable[[], None]] = None,
        on_lost_leadership: Optional[Callable[[], None]] = None,
    ) -> None:
        """Start leader election."""
        self.running = True

        while self.running:
            try:
                await self._try_acquire_lease()

                if self.is_leader:
                    if on_became_leader:
                        on_became_leader()

                    # Start heartbeat
                    await self._maintain_lease()
                else:
                    # Wait before retrying
                    await asyncio.sleep(self.lease_ttl_ms / 2000.0)

            except Exception as err:
                print(f"Leader election error: {err}")

                if self.is_leader and on_lost_leadership:
                    on_lost_leadership()

                self.is_leader = False
                await asyncio.sleep(1)

    async def stop(self) -> None:
        """Stop leader election."""
        self.running = False

        if self.heartbeat_task:
            self.heartbeat_task.cancel()
            try:
                await self.heartbeat_task
            except asyncio.CancelledError:
                pass

        if self.is_leader:
            await self._release_lease()

    async def _try_acquire_lease(self) -> None:
        """Try to acquire leadership lease."""
        key = self._leader_key()

        try:
            # Try to read current leader
            result = await self.client.get(key)
            current_leader = result.value.decode()

            if current_leader == self.node_id:
                self.is_leader = True

        except KeyNotFoundError:
            # No leader - try to become leader
            try:
                await self.client.put(
                    key,
                    self.node_id.encode(),
                    PutOptions(ttl_ms=self.lease_ttl_ms),
                )
                self.is_leader = True
            except Exception:
                # Someone else became leader
                self.is_leader = False

    async def _maintain_lease(self) -> None:
        """Maintain leadership lease with heartbeat."""
        refresh_interval = self.lease_ttl_ms / 3000.0

        while self.is_leader and self.running:
            try:
                await asyncio.sleep(refresh_interval)

                await self.client.put(
                    self._leader_key(),
                    self.node_id.encode(),
                    PutOptions(ttl_ms=self.lease_ttl_ms),
                )

            except Exception as err:
                print(f"Failed to refresh lease: {err}")
                self.is_leader = False
                break

    async def _release_lease(self) -> None:
        """Release leadership lease."""
        try:
            await self.client.delete(self._leader_key())
        except Exception as err:
            print(f"Failed to release lease: {err}")

    def get_is_leader(self) -> bool:
        return self.is_leader
```

### Usage Example

```python
election = LeaderElection(client, "my-service", "node-1", lease_ttl_ms=10000)

def on_became_leader():
    print("Became leader - starting background tasks")
    start_background_jobs()

def on_lost_leadership():
    print("Lost leadership - stopping background tasks")
    stop_background_jobs()

# Start election in background
asyncio.create_task(election.start(on_became_leader, on_lost_leadership))

# Check leadership
if election.get_is_leader():
    print("I am the leader")

# Graceful shutdown
await election.stop()
```

## Event Sourcing

Implement event sourcing pattern for audit logs and state reconstruction.

### Event Store

```python
from dataclasses import dataclass
from typing import Any

@dataclass
class Event:
    id: str
    aggregate_id: str
    type: str
    data: dict[str, Any]
    timestamp: int
    version: int

class EventStore:
    def __init__(self, client: NoriKVClient):
        self.client = client
        self.prefix = "events"

    def _event_key(self, aggregate_id: str, version: int) -> str:
        return f"{self.prefix}:{aggregate_id}:{version}"

    def _metadata_key(self, aggregate_id: str) -> str:
        return f"{self.prefix}:meta:{aggregate_id}"

    async def append(
        self,
        aggregate_id: str,
        event_type: str,
        data: dict[str, Any],
    ) -> Event:
        """Append event to aggregate."""
        max_retries = 10

        for attempt in range(max_retries):
            try:
                # Read current version
                current_version = 0
                meta_version = None

                try:
                    meta = await self.client.get(self._metadata_key(aggregate_id))
                    metadata = json.loads(meta.value.decode())
                    current_version = metadata["version"]
                    meta_version = meta.version
                except KeyNotFoundError:
                    pass

                new_version = current_version + 1

                # Create event
                event = Event(
                    id=str(uuid.uuid4()),
                    aggregate_id=aggregate_id,
                    type=event_type,
                    data=data,
                    timestamp=int(time.time() * 1000),
                    version=new_version,
                )

                # Write event
                await self.client.put(
                    self._event_key(aggregate_id, new_version),
                    json.dumps(asdict(event)).encode(),
                )

                # Update metadata with CAS
                options = (
                    PutOptions(if_match_version=meta_version)
                    if meta_version
                    else PutOptions()
                )
                await self.client.put(
                    self._metadata_key(aggregate_id),
                    json.dumps({"version": new_version}).encode(),
                    options,
                )

                return event

            except VersionMismatchError:
                await asyncio.sleep((2 ** attempt) * 0.01)

        raise RuntimeError("Failed to append event after retries")

    async def get_events(self, aggregate_id: str) -> list[Event]:
        """Get all events for aggregate."""
        try:
            meta = await self.client.get(self._metadata_key(aggregate_id))
            metadata = json.loads(meta.value.decode())
            current_version = metadata["version"]

            # Fetch all events concurrently
            tasks = [
                self.client.get(self._event_key(aggregate_id, i + 1))
                for i in range(current_version)
            ]

            results = await asyncio.gather(*tasks)
            return [
                Event(**json.loads(r.value.decode()))
                for r in results
            ]

        except KeyNotFoundError:
            return []

    async def get_events_after(
        self,
        aggregate_id: str,
        after_version: int,
    ) -> list[Event]:
        """Get events after specific version."""
        all_events = await self.get_events(aggregate_id)
        return [e for e in all_events if e.version > after_version]
```

### Aggregate Example

```python
@dataclass
class BankAccount:
    id: str
    balance: int
    is_open: bool

class BankAccountAggregate:
    def __init__(self, store: EventStore, account_id: str):
        self.store = store
        self.state = BankAccount(
            id=account_id,
            balance=0,
            is_open=False,
        )

    async def load(self) -> None:
        """Load aggregate from event history."""
        events = await self.store.get_events(self.state.id)

        for event in events:
            self._apply(event)

    async def open_account(self) -> None:
        """Open bank account."""
        if self.state.is_open:
            raise ValueError("Account already open")

        event = await self.store.append(self.state.id, "AccountOpened", {})
        self._apply(event)

    async def deposit(self, amount: int) -> None:
        """Deposit money."""
        if not self.state.is_open:
            raise ValueError("Account not open")

        event = await self.store.append(
            self.state.id,
            "MoneyDeposited",
            {"amount": amount},
        )
        self._apply(event)

    async def withdraw(self, amount: int) -> None:
        """Withdraw money."""
        if not self.state.is_open:
            raise ValueError("Account not open")

        if self.state.balance < amount:
            raise ValueError("Insufficient funds")

        event = await self.store.append(
            self.state.id,
            "MoneyWithdrawn",
            {"amount": amount},
        )
        self._apply(event)

    def _apply(self, event: Event) -> None:
        """Apply event to state."""
        if event.type == "AccountOpened":
            self.state.is_open = True
        elif event.type == "MoneyDeposited":
            self.state.balance += event.data["amount"]
        elif event.type == "MoneyWithdrawn":
            self.state.balance -= event.data["amount"]

    def get_balance(self) -> int:
        return self.state.balance
```

### Usage Example

```python
event_store = EventStore(client)

account = BankAccountAggregate(event_store, "account-123")

# Replay history
await account.load()

# Execute commands (generates events)
await account.open_account()
await account.deposit(100)
await account.withdraw(30)

print(f"Balance: {account.get_balance()}")  # 70

# Audit trail
events = await event_store.get_events("account-123")
print(f"Event history: {events}")
```

## Multi-Tenancy

Implement tenant isolation with namespace prefixing.

### Tenant Client

```python
class TenantClient:
    def __init__(self, client: NoriKVClient, tenant_id: str):
        self.client = client
        self.tenant_id = tenant_id

    def _tenant_key(self, key: str) -> str:
        return f"tenant:{self.tenant_id}:{key}"

    async def put(
        self,
        key: str,
        value: bytes,
        options: Optional[PutOptions] = None,
    ) -> Version:
        return await self.client.put(self._tenant_key(key), value, options)

    async def get(
        self,
        key: str,
        options: Optional[GetOptions] = None,
    ) -> GetResult:
        return await self.client.get(self._tenant_key(key), options)

    async def delete(
        self,
        key: str,
        options: Optional[DeleteOptions] = None,
    ) -> bool:
        return await self.client.delete(self._tenant_key(key), options)

    def get_tenant_id(self) -> str:
        return self.tenant_id
```

### Tenant Manager

```python
from typing import Literal

@dataclass
class TenantMetadata:
    id: str
    name: str
    plan: Literal["free", "pro", "enterprise"]
    created_at: int
    limits: dict[str, int]

class TenantManager:
    def __init__(self, client: NoriKVClient):
        self.client = client
        self.prefix = "tenant-meta"

    def _metadata_key(self, tenant_id: str) -> str:
        return f"{self.prefix}:{tenant_id}"

    async def create(
        self,
        tenant_id: str,
        name: str,
        plan: Literal["free", "pro", "enterprise"],
    ) -> TenantMetadata:
        """Create tenant."""
        limits = self._get_limits_for_plan(plan)

        metadata = TenantMetadata(
            id=tenant_id,
            name=name,
            plan=plan,
            created_at=int(time.time() * 1000),
            limits=limits,
        )

        await self.client.put(
            self._metadata_key(tenant_id),
            json.dumps(asdict(metadata)).encode(),
        )

        return metadata

    async def get(self, tenant_id: str) -> Optional[TenantMetadata]:
        """Get tenant metadata."""
        try:
            result = await self.client.get(self._metadata_key(tenant_id))
            data = json.loads(result.value.decode())
            return TenantMetadata(**data)
        except KeyNotFoundError:
            return None

    async def get_client(self, tenant_id: str) -> TenantClient:
        """Get tenant-scoped client."""
        metadata = await self.get(tenant_id)
        if not metadata:
            raise ValueError(f"Tenant not found: {tenant_id}")

        return TenantClient(self.client, tenant_id)

    def _get_limits_for_plan(self, plan: str) -> dict[str, int]:
        limits_map = {
            "free": {"max_keys": 1000, "max_storage_mb": 10},
            "pro": {"max_keys": 100000, "max_storage_mb": 1000},
            "enterprise": {"max_keys": float("inf"), "max_storage_mb": float("inf")},
        }
        return limits_map.get(plan, limits_map["free"])
```

### FastAPI Integration

```python
from fastapi import FastAPI, Header, HTTPException

app = FastAPI()
tenant_manager = TenantManager(client)

async def get_tenant_client(x_tenant_id: str = Header(...)) -> TenantClient:
    """Dependency to extract tenant client from header."""
    try:
        return await tenant_manager.get_client(x_tenant_id)
    except ValueError:
        raise HTTPException(status_code=404, detail="Tenant not found")

@app.get("/api/data/{key}")
async def get_data(key: str, tenant_client: TenantClient = Depends(get_tenant_client)):
    try:
        result = await tenant_client.get(key)
        return {"value": result.value.decode()}
    except KeyNotFoundError:
        raise HTTPException(status_code=404, detail="Not found")

@app.post("/api/data/{key}")
async def set_data(
    key: str,
    value: str,
    tenant_client: TenantClient = Depends(get_tenant_client),
):
    await tenant_client.put(key, value.encode())
    return {"success": True}
```

### Usage Example

```python
# Create tenants
await tenant_manager.create("tenant-acme", "Acme Corp", "pro")
await tenant_manager.create("tenant-widgets", "Widgets Inc", "enterprise")

# Get tenant-scoped clients
acme_client = await tenant_manager.get_client("tenant-acme")
widgets_client = await tenant_manager.get_client("tenant-widgets")

# Isolated operations
await acme_client.put("config", b"acme-config")
await widgets_client.put("config", b"widgets-config")

# Data is isolated
acme_config = await acme_client.get("config")  # b"acme-config"
widgets_config = await widgets_client.get("config")  # b"widgets-config"
```

## Best Practices

### 1. Always Use Context Managers

```python
async with NoriKVClient(config) as client:
    await client.put(key, value)
```

### 2. Implement Retry Logic for CAS

```python
async def cas_retry(operation, max_retries: int = 10):
    for attempt in range(max_retries):
        try:
            return await operation()
        except VersionMismatchError:
            if attempt == max_retries - 1:
                raise
            await asyncio.sleep((2 ** attempt) * 0.01)
```

### 3. Use Type Hints

```python
from typing import Optional

async def get_user(key: str) -> Optional[dict]:
    try:
        result = await client.get(key)
        return json.loads(result.value.decode())
    except KeyNotFoundError:
        return None
```

### 4. Clean Up Resources

```python
async with NoriKVClient(config) as client:
    # Client automatically closed
    await client.put(key, value)
```

### 5. Use Idempotency Keys for Critical Operations

```python
await client.put(
    key,
    value,
    PutOptions(idempotency_key=f"operation-{operation_id}"),
)
```

## Semantic Search

Build a semantic search engine using vector embeddings:

```python
from dataclasses import dataclass
import json
from norikv import (
    NoriKVClient,
    DistanceFunction,
    VectorIndexType,
    KeyNotFoundError,
)

@dataclass
class SearchResult:
    id: str
    title: str
    content: str
    score: float

@dataclass
class DocumentMeta:
    title: str
    content: str

class SemanticSearchEngine:
    def __init__(self, client: NoriKVClient, embedding_model):
        self.client = client
        self.embedding_model = embedding_model
        self.namespace = "documents"

    async def initialize(self) -> None:
        await self.client.vector_create_index(
            self.namespace,
            self.embedding_model.dimensions,
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
        )

    async def index_document(self, doc_id: str, title: str, content: str) -> None:
        # Generate embedding
        embedding = await self.embedding_model.embed(f"{title} {content}")

        # Store embedding
        await self.client.vector_insert(self.namespace, doc_id, embedding)

        # Store metadata
        meta = {"title": title, "content": content}
        await self.client.put(f"doc:meta:{doc_id}", json.dumps(meta).encode())

    async def search(self, query: str, top_k: int) -> list[SearchResult]:
        # Generate query embedding
        query_embedding = await self.embedding_model.embed(query)

        # Search
        result = await self.client.vector_search(self.namespace, query_embedding, top_k)

        # Fetch metadata
        results = []
        for match in result.matches:
            try:
                meta_result = await self.client.get(f"doc:meta:{match.id}")
                meta = json.loads(meta_result.value.decode())

                results.append(SearchResult(
                    id=match.id,
                    title=meta["title"],
                    content=meta["content"],
                    score=1.0 - match.distance,
                ))
            except KeyNotFoundError:
                pass  # Skip if metadata not found

        return results

    async def delete_document(self, doc_id: str) -> None:
        await self.client.vector_delete(self.namespace, doc_id)
        await self.client.delete(f"doc:meta:{doc_id}")


# Usage
async def main():
    engine = SemanticSearchEngine(client, openai_embedding)
    await engine.initialize()

    # Index documents
    await engine.index_document("doc1", "Machine Learning",
        "Machine learning is a subset of AI...")
    await engine.index_document("doc2", "Deep Learning",
        "Deep learning uses neural networks...")

    # Search
    results = await engine.search("AI neural networks", 5)
    for r in results:
        print(f"{r.score:.2f}: {r.title}")
```

## Recommendation System

Build a product recommendation engine:

```python
@dataclass
class ProductRecommendation:
    product_id: str
    name: str
    category: str
    score: float

class RecommendationEngine:
    def __init__(self, client: NoriKVClient, embedding_model):
        self.client = client
        self.embedding_model = embedding_model

    async def initialize(self) -> None:
        # Create product index
        await self.client.vector_create_index(
            "products",
            self.embedding_model.dimensions,
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
        )

        # Create user preference index
        await self.client.vector_create_index(
            "user_prefs",
            self.embedding_model.dimensions,
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
        )

    async def index_product(self, product_id: str, name: str, desc: str, category: str) -> None:
        embedding = await self.embedding_model.embed(f"{name} {desc} {category}")
        await self.client.vector_insert("products", product_id, embedding)

    async def record_interaction(self, user_id: str, product_id: str) -> None:
        # Get product embedding
        product_embed = await self.client.vector_get("products", product_id)
        if product_embed is None:
            return

        # Update user preferences (simple moving average)
        current_pref = await self.client.vector_get("user_prefs", user_id)
        if current_pref is not None:
            current_pref = [(c + p) / 2 for c, p in zip(current_pref, product_embed)]
        else:
            current_pref = product_embed

        await self.client.vector_insert("user_prefs", user_id, current_pref)

    async def get_recommendations(self, user_id: str, top_k: int) -> list[ProductRecommendation]:
        user_pref = await self.client.vector_get("user_prefs", user_id)
        if user_pref is None:
            return []

        result = await self.client.vector_search("products", user_pref, top_k)

        return [
            ProductRecommendation(
                product_id=match.id,
                name="",
                category="",
                score=1.0 - match.distance,
            )
            for match in result.matches
        ]

    async def get_similar_products(self, product_id: str, top_k: int) -> list[ProductRecommendation]:
        embedding = await self.client.vector_get("products", product_id)
        if embedding is None:
            return []

        result = await self.client.vector_search("products", embedding, top_k + 1)

        return [
            ProductRecommendation(
                product_id=match.id,
                name="",
                category="",
                score=1.0 - match.distance,
            )
            for match in result.matches
            if match.id != product_id
        ][:top_k]


# Usage
async def main():
    engine = RecommendationEngine(client, embedding_model)
    await engine.initialize()

    # Index products
    await engine.index_product("prod1", "Running Shoes", "Lightweight", "Footwear")
    await engine.index_product("prod2", "Trail Shoes", "Durable", "Footwear")

    # Record user interactions
    await engine.record_interaction("user123", "prod1")

    # Get recommendations
    recs = await engine.get_recommendations("user123", 5)
    for r in recs:
        print(f"{r.score:.2f}: {r.product_id}")
```

## Document Deduplication

Detect near-duplicate documents using vector similarity:

```python
@dataclass
class DeduplicationResult:
    is_duplicate: bool
    duplicate_of: str | None
    similarity: float

class DocumentDeduplicator:
    def __init__(self, client: NoriKVClient, embedding_model, threshold: float):
        self.client = client
        self.embedding_model = embedding_model
        self.similarity_threshold = threshold

    async def initialize(self) -> None:
        await self.client.vector_create_index(
            "doc_fingerprints",
            self.embedding_model.dimensions,
            DistanceFunction.COSINE,
            VectorIndexType.HNSW,
        )

    async def check_and_store(self, doc_id: str, content: str) -> DeduplicationResult:
        embedding = await self.embedding_model.embed(content)

        # Search for similar documents
        result = await self.client.vector_search("doc_fingerprints", embedding, 5)

        # Check for duplicates
        for match in result.matches:
            similarity = 1.0 - match.distance
            if similarity >= self.similarity_threshold:
                return DeduplicationResult(
                    is_duplicate=True,
                    duplicate_of=match.id,
                    similarity=similarity,
                )

        # No duplicate found, store new document
        await self.client.vector_insert("doc_fingerprints", doc_id, embedding)

        return DeduplicationResult(is_duplicate=False, duplicate_of=None, similarity=0.0)

    async def remove_document(self, doc_id: str) -> None:
        await self.client.vector_delete("doc_fingerprints", doc_id)


# Usage
async def main():
    dedup = DocumentDeduplicator(client, embedding_model, 0.95)
    await dedup.initialize()

    doc1 = "The quick brown fox jumps over the lazy dog."
    doc2 = "A quick brown fox jumped over a lazy dog."
    doc3 = "Machine learning is transforming industries."

    result1 = await dedup.check_and_store("doc1", doc1)
    print(f"doc1 is duplicate: {result1.is_duplicate}")  # False

    result2 = await dedup.check_and_store("doc2", doc2)
    print(f"doc2 is duplicate: {result2.is_duplicate}")  # True
    print(f"doc2 duplicate of: {result2.duplicate_of}")  # doc1
    print(f"Similarity: {result2.similarity:.2f}")       # ~0.96

    result3 = await dedup.check_and_store("doc3", doc3)
    print(f"doc3 is duplicate: {result3.is_duplicate}")  # False
```

## Next Steps

- [API Guide](API_GUIDE.md) - Core API reference
- [Architecture Guide](ARCHITECTURE.md) - Internal design
- [Troubleshooting Guide](TROUBLESHOOTING.md) - Common issues
- [GitHub Examples](https://github.com/jeffhajewski/norikv/tree/main/sdks/python/examples) - More code samples
