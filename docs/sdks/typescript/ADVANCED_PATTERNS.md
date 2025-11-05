---
layout: default
title: Advanced Patterns
parent: TypeScript SDK
grand_parent: Client SDKs
nav_order: 4
---

# NoriKV TypeScript Client Advanced Patterns

Complex real-world usage patterns and production-ready design examples.

## Table of Contents

- [Distributed Counter](#distributed-counter)
- [Session Management](#session-management)
- [Inventory Management](#inventory-management)
- [Caching Layer](#caching-layer)
- [Rate Limiting](#rate-limiting)
- [Leader Election](#leader-election)
- [Event Sourcing](#event-sourcing)
- [Multi-Tenancy](#multi-tenancy)

## Distributed Counter

Implement a high-throughput distributed counter with sharding to reduce contention.

### Basic Counter

```typescript
import {
  NoriKVClient,
  GetResult,
  VersionMismatchError,
  KeyNotFoundError,
  stringToBytes,
  bytesToString,
} from '@norikv/client';

export class DistributedCounter {
  private client: NoriKVClient;
  private key: string;
  private maxRetries: number;

  constructor(client: NoriKVClient, counterName: string) {
    this.client = client;
    this.key = counterName;
    this.maxRetries = 20;
  }

  async initialize(): Promise<void> {
    try {
      await this.client.get(this.key);
    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        try {
          await this.client.put(this.key, '0');
        } catch (putErr) {
          // Ignore - someone else may have initialized
        }
      } else {
        throw err;
      }
    }
  }

  async increment(): Promise<number> {
    return this.incrementBy(1);
  }

  async incrementBy(delta: number): Promise<number> {
    for (let attempt = 0; attempt < this.maxRetries; attempt++) {
      try {
        // Read current value
        const current = await this.client.get(this.key);
        const value = parseInt(bytesToString(current.value));

        // Increment
        const newValue = value + delta;

        // CAS write
        await this.client.put(this.key, newValue.toString(), {
          ifMatchVersion: current.version,
        });

        return newValue;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        if (attempt === this.maxRetries - 1) {
          throw new Error(`Failed to increment after ${this.maxRetries} attempts`);
        }

        // Exponential backoff with jitter
        const backoff = Math.min(Math.pow(2, attempt), 1000);
        const jitter = Math.random() * 100;
        await new Promise(resolve => setTimeout(resolve, backoff + jitter));
      }
    }

    throw new Error('Should not reach here');
  }

  async get(): Promise<number> {
    const result = await this.client.get(this.key);
    return parseInt(bytesToString(result.value));
  }
}
```

### Sharded Counter (High Throughput)

For very high write rates, distribute writes across multiple shards:

```typescript
export class ShardedCounter {
  private client: NoriKVClient;
  private name: string;
  private numShards: number;

  constructor(client: NoriKVClient, name: string, numShards: number = 10) {
    this.client = client;
    this.name = name;
    this.numShards = numShards;
  }

  private randomShard(): number {
    return Math.floor(Math.random() * this.numShards);
  }

  private shardKey(shardId: number): string {
    return `${this.name}:shard:${shardId}`;
  }

  async increment(): Promise<void> {
    // Randomly select a shard to reduce contention
    const shardId = this.randomShard();
    const key = this.shardKey(shardId);

    const maxRetries = 10;
    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        let currentValue = 0;
        let currentVersion = null;

        try {
          const result = await this.client.get(key);
          currentValue = parseInt(bytesToString(result.value));
          currentVersion = result.version;
        } catch (err) {
          if (!(err instanceof KeyNotFoundError)) {
            throw err;
          }
          // Key doesn't exist yet - start at 0
        }

        const newValue = currentValue + 1;
        const options = currentVersion ? { ifMatchVersion: currentVersion } : {};

        await this.client.put(key, newValue.toString(), options);
        return;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        // Exponential backoff
        const backoff = Math.pow(2, attempt) * 10;
        const jitter = Math.random() * 10;
        await new Promise(resolve => setTimeout(resolve, backoff + jitter));
      }
    }

    throw new Error('Increment failed after retries');
  }

  async get(): Promise<number> {
    // Sum all shards concurrently
    const promises = Array.from({ length: this.numShards }, async (_, i) => {
      try {
        const result = await this.client.get(this.shardKey(i));
        return parseInt(bytesToString(result.value));
      } catch (err) {
        if (err instanceof KeyNotFoundError) {
          return 0;
        }
        throw err;
      }
    });

    const values = await Promise.all(promises);
    return values.reduce((sum, val) => sum + val, 0);
  }
}
```

### Usage Example

```typescript
const client = new NoriKVClient(config);
await client.connect();

// Basic counter
const counter = new DistributedCounter(client, 'api:requests');
await counter.initialize();

const newCount = await counter.increment();
console.log('Request count:', newCount);

// Sharded counter for high throughput
const sharded = new ShardedCounter(client, 'page:views', 20);

// Many concurrent increments
await Promise.all(
  Array(1000).fill(0).map(() => sharded.increment())
);

const total = await sharded.get();
console.log('Total views:', total);
```

## Session Management

Implement secure session storage with automatic expiration using TTL.

### Session Manager

```typescript
import { v4 as uuidv4 } from 'uuid';

interface SessionData {
  userId: string;
  email: string;
  roles: string[];
  createdAt: number;
  lastAccessedAt: number;
}

export class SessionManager {
  private client: NoriKVClient;
  private ttlMs: number;
  private prefix: string;

  constructor(client: NoriKVClient, ttlMs: number = 3600000) {
    this.client = client;
    this.ttlMs = ttlMs; // Default: 1 hour
    this.prefix = 'session';
  }

  private sessionKey(sessionId: string): string {
    return `${this.prefix}:${sessionId}`;
  }

  async create(userId: string, email: string, roles: string[]): Promise<string> {
    const sessionId = uuidv4();
    const key = this.sessionKey(sessionId);

    const data: SessionData = {
      userId,
      email,
      roles,
      createdAt: Date.now(),
      lastAccessedAt: Date.now(),
    };

    await this.client.put(key, JSON.stringify(data), {
      ttlMs: this.ttlMs,
      idempotencyKey: `session-create-${sessionId}`,
    });

    return sessionId;
  }

  async get(sessionId: string): Promise<SessionData | null> {
    try {
      const result = await this.client.get(this.sessionKey(sessionId));
      const data: SessionData = JSON.parse(bytesToString(result.value));

      // Update last accessed time
      data.lastAccessedAt = Date.now();
      await this.client.put(this.sessionKey(sessionId), JSON.stringify(data), {
        ttlMs: this.ttlMs, // Reset TTL
      });

      return data;
    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        return null;
      }
      throw err;
    }
  }

  async update(sessionId: string, updateFn: (data: SessionData) => SessionData): Promise<boolean> {
    const maxRetries = 5;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        const result = await this.client.get(this.sessionKey(sessionId));
        const data: SessionData = JSON.parse(bytesToString(result.value));

        // Apply update
        const updated = updateFn(data);
        updated.lastAccessedAt = Date.now();

        // CAS write
        await this.client.put(
          this.sessionKey(sessionId),
          JSON.stringify(updated),
          {
            ifMatchVersion: result.version,
            ttlMs: this.ttlMs,
          }
        );

        return true;

      } catch (err) {
        if (err instanceof KeyNotFoundError) {
          return false;
        }

        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        // Retry with backoff
        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    return false;
  }

  async destroy(sessionId: string): Promise<boolean> {
    try {
      return await this.client.delete(this.sessionKey(sessionId), {
        idempotencyKey: `session-destroy-${sessionId}`,
      });
    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        return false;
      }
      throw err;
    }
  }

  async validate(sessionId: string): Promise<boolean> {
    try {
      await this.client.get(this.sessionKey(sessionId));
      return true;
    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        return false;
      }
      throw err;
    }
  }
}
```

### Usage Example

```typescript
const sessions = new SessionManager(client, 1800000); // 30 min

// Create session
const sessionId = await sessions.create('user123', 'alice@example.com', ['user', 'admin']);

// Middleware: validate session
async function authMiddleware(req, res, next) {
  const sessionId = req.cookies.sessionId;

  const sessionData = await sessions.get(sessionId);
  if (!sessionData) {
    return res.status(401).json({ error: 'Unauthorized' });
  }

  req.user = sessionData;
  next();
}

// Update session
await sessions.update(sessionId, (data) => ({
  ...data,
  roles: [...data.roles, 'premium'],
}));

// Destroy session
await sessions.destroy(sessionId);
```

## Inventory Management

Prevent overselling with optimistic concurrency control.

### Inventory System

```typescript
interface InventoryItem {
  sku: string;
  quantity: number;
  reserved: number;
  lastUpdated: number;
}

export class InventoryManager {
  private client: NoriKVClient;
  private prefix: string;

  constructor(client: NoriKVClient) {
    this.client = client;
    this.prefix = 'inventory';
  }

  private itemKey(sku: string): string {
    return `${this.prefix}:${sku}`;
  }

  async createItem(sku: string, initialQuantity: number): Promise<void> {
    const item: InventoryItem = {
      sku,
      quantity: initialQuantity,
      reserved: 0,
      lastUpdated: Date.now(),
    };

    await this.client.put(this.itemKey(sku), JSON.stringify(item));
  }

  async reserve(sku: string, quantity: number): Promise<string> {
    const reservationId = uuidv4();
    const maxRetries = 10;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        // Read current inventory
        const result = await this.client.get(this.itemKey(sku));
        const item: InventoryItem = JSON.parse(bytesToString(result.value));

        // Check availability
        const available = item.quantity - item.reserved;
        if (available < quantity) {
          throw new Error(`Insufficient inventory: need ${quantity}, have ${available}`);
        }

        // Reserve quantity
        item.reserved += quantity;
        item.lastUpdated = Date.now();

        // CAS write
        await this.client.put(
          this.itemKey(sku),
          JSON.stringify(item),
          { ifMatchVersion: result.version }
        );

        // Store reservation
        await this.client.put(
          `${this.prefix}:reservation:${reservationId}`,
          JSON.stringify({ sku, quantity, createdAt: Date.now() }),
          { ttlMs: 600000 } // 10 min expiration
        );

        return reservationId;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        if (attempt === maxRetries - 1) {
          throw new Error('Failed to reserve after retries');
        }

        // Exponential backoff
        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    throw new Error('Should not reach here');
  }

  async commit(reservationId: string): Promise<void> {
    const maxRetries = 10;

    // Get reservation details
    const reservationKey = `${this.prefix}:reservation:${reservationId}`;
    const resResult = await this.client.get(reservationKey);
    const reservation = JSON.parse(bytesToString(resResult.value));

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        // Read current inventory
        const result = await this.client.get(this.itemKey(reservation.sku));
        const item: InventoryItem = JSON.parse(bytesToString(result.value));

        // Commit: reduce both quantity and reserved
        item.quantity -= reservation.quantity;
        item.reserved -= reservation.quantity;
        item.lastUpdated = Date.now();

        // CAS write
        await this.client.put(
          this.itemKey(reservation.sku),
          JSON.stringify(item),
          { ifMatchVersion: result.version }
        );

        // Delete reservation
        await this.client.delete(reservationKey);
        return;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    throw new Error('Failed to commit reservation');
  }

  async release(reservationId: string): Promise<void> {
    const maxRetries = 10;

    // Get reservation details
    const reservationKey = `${this.prefix}:reservation:${reservationId}`;
    const resResult = await this.client.get(reservationKey);
    const reservation = JSON.parse(bytesToString(resResult.value));

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        // Read current inventory
        const result = await this.client.get(this.itemKey(reservation.sku));
        const item: InventoryItem = JSON.parse(bytesToString(result.value));

        // Release: reduce reserved only
        item.reserved -= reservation.quantity;
        item.lastUpdated = Date.now();

        // CAS write
        await this.client.put(
          this.itemKey(reservation.sku),
          JSON.stringify(item),
          { ifMatchVersion: result.version }
        );

        // Delete reservation
        await this.client.delete(reservationKey);
        return;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    throw new Error('Failed to release reservation');
  }

  async getAvailable(sku: string): Promise<number> {
    const result = await this.client.get(this.itemKey(sku));
    const item: InventoryItem = JSON.parse(bytesToString(result.value));
    return item.quantity - item.reserved;
  }
}
```

### Usage Example

```typescript
const inventory = new InventoryManager(client);

// Initialize inventory
await inventory.createItem('SKU-12345', 100);

// Purchase flow
async function purchaseItem(sku: string, quantity: number) {
  // 1. Reserve inventory
  const reservationId = await inventory.reserve(sku, quantity);

  try {
    // 2. Process payment
    await processPayment();

    // 3. Commit reservation
    await inventory.commit(reservationId);
    console.log('Purchase successful');

  } catch (err) {
    // 4. Release reservation on failure
    await inventory.release(reservationId);
    console.error('Purchase failed:', err);
    throw err;
  }
}

// Check availability
const available = await inventory.getAvailable('SKU-12345');
console.log('Available:', available);
```

## Caching Layer

Implement a write-through cache with automatic invalidation.

### Cache Implementation

```typescript
interface CacheEntry<T> {
  value: T;
  version: Version;
  cachedAt: number;
}

export class CachedClient<T> {
  private client: NoriKVClient;
  private cache: Map<string, CacheEntry<T>>;
  private ttlMs: number;
  private maxSize: number;

  constructor(client: NoriKVClient, ttlMs: number = 60000, maxSize: number = 1000) {
    this.client = client;
    this.cache = new Map();
    this.ttlMs = ttlMs;
    this.maxSize = maxSize;
  }

  async get(key: string, parser: (bytes: Uint8Array) => T): Promise<T> {
    // Check cache
    const cached = this.cache.get(key);
    if (cached && Date.now() - cached.cachedAt < this.ttlMs) {
      return cached.value;
    }

    // Cache miss - fetch from NoriKV
    const result = await this.client.get(key);
    const value = parser(result.value);

    // Update cache
    this.setCache(key, {
      value,
      version: result.version,
      cachedAt: Date.now(),
    });

    return value;
  }

  async put(key: string, value: T, serializer: (val: T) => string | Uint8Array): Promise<Version> {
    // Write-through to NoriKV
    const version = await this.client.put(key, serializer(value));

    // Update cache
    this.setCache(key, {
      value,
      version,
      cachedAt: Date.now(),
    });

    return version;
  }

  async delete(key: string): Promise<boolean> {
    // Invalidate cache
    this.cache.delete(key);

    // Delete from NoriKV
    return await this.client.delete(key);
  }

  invalidate(key: string): void {
    this.cache.delete(key);
  }

  invalidateAll(): void {
    this.cache.clear();
  }

  private setCache(key: string, entry: CacheEntry<T>): void {
    // LRU eviction
    if (this.cache.size >= this.maxSize) {
      const firstKey = this.cache.keys().next().value;
      this.cache.delete(firstKey);
    }

    this.cache.set(key, entry);
  }

  getStats() {
    return {
      size: this.cache.size,
      maxSize: this.maxSize,
      ttlMs: this.ttlMs,
    };
  }
}
```

### Usage Example

```typescript
interface UserProfile {
  id: string;
  name: string;
  email: string;
}

const cache = new CachedClient<UserProfile>(client, 60000, 1000);

// Get with automatic caching
const profile = await cache.get(
  'user:123',
  (bytes) => JSON.parse(bytesToString(bytes)) as UserProfile
);

// Write-through cache
await cache.put(
  'user:123',
  { id: '123', name: 'Alice', email: 'alice@example.com' },
  (val) => JSON.stringify(val)
);

// Manual invalidation
cache.invalidate('user:123');

// Check stats
const stats = cache.getStats();
console.log('Cache stats:', stats);
```

## Rate Limiting

Implement sliding window rate limiting for API throttling.

### Rate Limiter

```typescript
interface RateLimitConfig {
  maxRequests: number;
  windowMs: number;
}

export class RateLimiter {
  private client: NoriKVClient;
  private prefix: string;

  constructor(client: NoriKVClient) {
    this.client = client;
    this.prefix = 'ratelimit';
  }

  private key(identifier: string): string {
    return `${this.prefix}:${identifier}`;
  }

  async checkLimit(
    identifier: string,
    config: RateLimitConfig
  ): Promise<{ allowed: boolean; remaining: number; resetAt: number }> {
    const key = this.key(identifier);
    const now = Date.now();
    const windowStart = now - config.windowMs;

    const maxRetries = 5;
    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        let timestamps: number[] = [];
        let version = null;

        // Read current timestamps
        try {
          const result = await this.client.get(key);
          timestamps = JSON.parse(bytesToString(result.value));
          version = result.version;
        } catch (err) {
          if (!(err instanceof KeyNotFoundError)) {
            throw err;
          }
        }

        // Remove old timestamps outside window
        timestamps = timestamps.filter(ts => ts > windowStart);

        // Check if limit exceeded
        const allowed = timestamps.length < config.maxRequests;

        if (allowed) {
          // Add current timestamp
          timestamps.push(now);

          // Save updated timestamps
          const options = version ? { ifMatchVersion: version } : {};
          await this.client.put(
            key,
            JSON.stringify(timestamps),
            {
              ...options,
              ttlMs: config.windowMs,
            }
          );
        }

        return {
          allowed,
          remaining: Math.max(0, config.maxRequests - timestamps.length),
          resetAt: timestamps.length > 0
            ? Math.min(...timestamps) + config.windowMs
            : now + config.windowMs,
        };

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        // Retry with backoff
        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    throw new Error('Rate limit check failed after retries');
  }

  async reset(identifier: string): Promise<void> {
    await this.client.delete(this.key(identifier));
  }
}
```

### Express Middleware Example

```typescript
const rateLimiter = new RateLimiter(client);

function rateLimitMiddleware(config: RateLimitConfig) {
  return async (req, res, next) => {
    // Use IP address or user ID as identifier
    const identifier = req.user?.id || req.ip;

    try {
      const result = await rateLimiter.checkLimit(identifier, config);

      // Set rate limit headers
      res.setHeader('X-RateLimit-Limit', config.maxRequests);
      res.setHeader('X-RateLimit-Remaining', result.remaining);
      res.setHeader('X-RateLimit-Reset', result.resetAt);

      if (!result.allowed) {
        return res.status(429).json({
          error: 'Too Many Requests',
          retryAfter: Math.ceil((result.resetAt - Date.now()) / 1000),
        });
      }

      next();
    } catch (err) {
      console.error('Rate limit error:', err);
      next(); // Fail open
    }
  };
}

// Usage
app.use('/api', rateLimitMiddleware({
  maxRequests: 100,
  windowMs: 60000, // 100 requests per minute
}));
```

## Leader Election

Implement distributed leader election with automatic failover.

### Leader Election

```typescript
export class LeaderElection {
  private client: NoriKVClient;
  private name: string;
  private nodeId: string;
  private leaseTtlMs: number;
  private running: boolean;
  private isLeader: boolean;
  private heartbeatInterval: NodeJS.Timeout | null;

  constructor(client: NoriKVClient, electionName: string, nodeId: string, leaseTtlMs: number = 10000) {
    this.client = client;
    this.name = electionName;
    this.nodeId = nodeId;
    this.leaseTtlMs = leaseTtlMs;
    this.running = false;
    this.isLeader = false;
    this.heartbeatInterval = null;
  }

  private leaderKey(): string {
    return `leader:${this.name}`;
  }

  async start(onBecameLeader?: () => void, onLostLeadership?: () => void): Promise<void> {
    this.running = true;

    while (this.running) {
      try {
        await this.tryAcquireLease();

        if (this.isLeader) {
          if (onBecameLeader) {
            onBecameLeader();
          }

          // Start heartbeat
          await this.maintainLease();
        } else {
          // Wait before retrying
          await new Promise(resolve => setTimeout(resolve, this.leaseTtlMs / 2));
        }

      } catch (err) {
        console.error('Leader election error:', err);

        if (this.isLeader && onLostLeadership) {
          onLostLeadership();
        }

        this.isLeader = false;
        await new Promise(resolve => setTimeout(resolve, 1000));
      }
    }
  }

  async stop(): Promise<void> {
    this.running = false;

    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval);
      this.heartbeatInterval = null;
    }

    if (this.isLeader) {
      await this.releaseLease();
    }
  }

  private async tryAcquireLease(): Promise<void> {
    const key = this.leaderKey();

    try {
      // Try to read current leader
      const result = await this.client.get(key);
      const currentLeader = bytesToString(result.value);

      if (currentLeader === this.nodeId) {
        this.isLeader = true;
      }

    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        // No leader - try to become leader
        try {
          await this.client.put(key, this.nodeId, {
            ttlMs: this.leaseTtlMs,
          });
          this.isLeader = true;
        } catch (putErr) {
          // Someone else became leader
          this.isLeader = false;
        }
      } else {
        throw err;
      }
    }
  }

  private async maintainLease(): Promise<void> {
    const refreshInterval = this.leaseTtlMs / 3;

    return new Promise((resolve) => {
      this.heartbeatInterval = setInterval(async () => {
        try {
          await this.client.put(this.leaderKey(), this.nodeId, {
            ttlMs: this.leaseTtlMs,
          });
        } catch (err) {
          console.error('Failed to refresh lease:', err);
          this.isLeader = false;

          if (this.heartbeatInterval) {
            clearInterval(this.heartbeatInterval);
            this.heartbeatInterval = null;
          }

          resolve();
        }
      }, refreshInterval);
    });
  }

  private async releaseLease(): Promise<void> {
    try {
      await this.client.delete(this.leaderKey());
    } catch (err) {
      console.error('Failed to release lease:', err);
    }
  }

  getIsLeader(): boolean {
    return this.isLeader;
  }
}
```

### Usage Example

```typescript
const election = new LeaderElection(client, 'my-service', 'node-1', 10000);

await election.start(
  () => {
    console.log('Became leader - starting background tasks');
    startBackgroundJobs();
  },
  () => {
    console.log('Lost leadership - stopping background tasks');
    stopBackgroundJobs();
  }
);

// Check leadership
if (election.getIsLeader()) {
  console.log('I am the leader');
}

// Graceful shutdown
process.on('SIGTERM', async () => {
  await election.stop();
  await client.close();
});
```

## Event Sourcing

Implement event sourcing pattern for audit logs and state reconstruction.

### Event Store

```typescript
interface Event {
  id: string;
  aggregateId: string;
  type: string;
  data: any;
  timestamp: number;
  version: number;
}

export class EventStore {
  private client: NoriKVClient;
  private prefix: string;

  constructor(client: NoriKVClient) {
    this.client = client;
    this.prefix = 'events';
  }

  private eventKey(aggregateId: string, version: number): string {
    return `${this.prefix}:${aggregateId}:${version}`;
  }

  private metadataKey(aggregateId: string): string {
    return `${this.prefix}:meta:${aggregateId}`;
  }

  async append(aggregateId: string, type: string, data: any): Promise<Event> {
    const maxRetries = 10;

    for (let attempt = 0; attempt < maxRetries; attempt++) {
      try {
        // Read current version
        let currentVersion = 0;
        let metaVersion = null;

        try {
          const meta = await this.client.get(this.metadataKey(aggregateId));
          const metadata = JSON.parse(bytesToString(meta.value));
          currentVersion = metadata.version;
          metaVersion = meta.version;
        } catch (err) {
          if (!(err instanceof KeyNotFoundError)) {
            throw err;
          }
        }

        const newVersion = currentVersion + 1;

        // Create event
        const event: Event = {
          id: uuidv4(),
          aggregateId,
          type,
          data,
          timestamp: Date.now(),
          version: newVersion,
        };

        // Write event
        await this.client.put(
          this.eventKey(aggregateId, newVersion),
          JSON.stringify(event)
        );

        // Update metadata with CAS
        const options = metaVersion ? { ifMatchVersion: metaVersion } : {};
        await this.client.put(
          this.metadataKey(aggregateId),
          JSON.stringify({ version: newVersion }),
          options
        );

        return event;

      } catch (err) {
        if (!(err instanceof VersionMismatchError)) {
          throw err;
        }

        await new Promise(resolve =>
          setTimeout(resolve, Math.pow(2, attempt) * 10)
        );
      }
    }

    throw new Error('Failed to append event after retries');
  }

  async getEvents(aggregateId: string): Promise<Event[]> {
    // Get current version
    try {
      const meta = await this.client.get(this.metadataKey(aggregateId));
      const metadata = JSON.parse(bytesToString(meta.value));
      const currentVersion = metadata.version;

      // Fetch all events concurrently
      const promises = Array.from({ length: currentVersion }, (_, i) =>
        this.client.get(this.eventKey(aggregateId, i + 1))
      );

      const results = await Promise.all(promises);
      return results.map(r => JSON.parse(bytesToString(r.value)));

    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        return [];
      }
      throw err;
    }
  }

  async getEventsAfter(aggregateId: string, afterVersion: number): Promise<Event[]> {
    const allEvents = await this.getEvents(aggregateId);
    return allEvents.filter(e => e.version > afterVersion);
  }
}
```

### Aggregate Example

```typescript
interface BankAccount {
  id: string;
  balance: number;
  isOpen: boolean;
}

class BankAccountAggregate {
  private store: EventStore;
  private state: BankAccount;

  constructor(store: EventStore, accountId: string) {
    this.store = store;
    this.state = {
      id: accountId,
      balance: 0,
      isOpen: false,
    };
  }

  async load(): Promise<void> {
    const events = await this.store.getEvents(this.state.id);

    for (const event of events) {
      this.apply(event);
    }
  }

  async openAccount(): Promise<void> {
    if (this.state.isOpen) {
      throw new Error('Account already open');
    }

    const event = await this.store.append(this.state.id, 'AccountOpened', {});
    this.apply(event);
  }

  async deposit(amount: number): Promise<void> {
    if (!this.state.isOpen) {
      throw new Error('Account not open');
    }

    const event = await this.store.append(this.state.id, 'MoneyDeposited', { amount });
    this.apply(event);
  }

  async withdraw(amount: number): Promise<void> {
    if (!this.state.isOpen) {
      throw new Error('Account not open');
    }

    if (this.state.balance < amount) {
      throw new Error('Insufficient funds');
    }

    const event = await this.store.append(this.state.id, 'MoneyWithdrawn', { amount });
    this.apply(event);
  }

  private apply(event: Event): void {
    switch (event.type) {
      case 'AccountOpened':
        this.state.isOpen = true;
        break;
      case 'MoneyDeposited':
        this.state.balance += event.data.amount;
        break;
      case 'MoneyWithdrawn':
        this.state.balance -= event.data.amount;
        break;
    }
  }

  getBalance(): number {
    return this.state.balance;
  }
}
```

### Usage Example

```typescript
const eventStore = new EventStore(client);

const account = new BankAccountAggregate(eventStore, 'account-123');

// Replay history
await account.load();

// Execute commands (generates events)
await account.openAccount();
await account.deposit(100);
await account.withdraw(30);

console.log('Balance:', account.getBalance()); // 70

// Audit trail
const events = await eventStore.getEvents('account-123');
console.log('Event history:', events);
```

## Multi-Tenancy

Implement tenant isolation with namespace prefixing.

### Tenant Client

```typescript
export class TenantClient {
  private client: NoriKVClient;
  private tenantId: string;

  constructor(client: NoriKVClient, tenantId: string) {
    this.client = client;
    this.tenantId = tenantId;
  }

  private tenantKey(key: string): string {
    return `tenant:${this.tenantId}:${key}`;
  }

  async put(key: string, value: string | Uint8Array, options?: PutOptions): Promise<Version> {
    return this.client.put(this.tenantKey(key), value, options);
  }

  async get(key: string, options?: GetOptions): Promise<GetResult> {
    return this.client.get(this.tenantKey(key), options);
  }

  async delete(key: string, options?: DeleteOptions): Promise<boolean> {
    return this.client.delete(this.tenantKey(key), options);
  }

  getTenantId(): string {
    return this.tenantId;
  }
}
```

### Tenant Manager

```typescript
interface TenantMetadata {
  id: string;
  name: string;
  plan: 'free' | 'pro' | 'enterprise';
  createdAt: number;
  limits: {
    maxKeys: number;
    maxStorageMB: number;
  };
}

export class TenantManager {
  private client: NoriKVClient;
  private prefix: string;

  constructor(client: NoriKVClient) {
    this.client = client;
    this.prefix = 'tenant-meta';
  }

  private metadataKey(tenantId: string): string {
    return `${this.prefix}:${tenantId}`;
  }

  async create(tenantId: string, name: string, plan: 'free' | 'pro' | 'enterprise'): Promise<TenantMetadata> {
    const limits = this.getLimitsForPlan(plan);

    const metadata: TenantMetadata = {
      id: tenantId,
      name,
      plan,
      createdAt: Date.now(),
      limits,
    };

    await this.client.put(
      this.metadataKey(tenantId),
      JSON.stringify(metadata)
    );

    return metadata;
  }

  async get(tenantId: string): Promise<TenantMetadata | null> {
    try {
      const result = await this.client.get(this.metadataKey(tenantId));
      return JSON.parse(bytesToString(result.value));
    } catch (err) {
      if (err instanceof KeyNotFoundError) {
        return null;
      }
      throw err;
    }
  }

  async getClient(tenantId: string): Promise<TenantClient> {
    const metadata = await this.get(tenantId);
    if (!metadata) {
      throw new Error(`Tenant not found: ${tenantId}`);
    }

    return new TenantClient(this.client, tenantId);
  }

  private getLimitsForPlan(plan: string) {
    const limits = {
      free: { maxKeys: 1000, maxStorageMB: 10 },
      pro: { maxKeys: 100000, maxStorageMB: 1000 },
      enterprise: { maxKeys: Infinity, maxStorageMB: Infinity },
    };

    return limits[plan] || limits.free;
  }
}
```

### Express Integration

```typescript
const tenantManager = new TenantManager(client);

// Middleware: extract tenant from request
async function tenantMiddleware(req, res, next) {
  const tenantId = req.headers['x-tenant-id'] || req.query.tenantId;

  if (!tenantId) {
    return res.status(400).json({ error: 'Tenant ID required' });
  }

  // Get tenant-scoped client
  try {
    req.tenantClient = await tenantManager.getClient(tenantId);
    req.tenantMetadata = await tenantManager.get(tenantId);
    next();
  } catch (err) {
    return res.status(404).json({ error: 'Tenant not found' });
  }
}

// Route handlers use tenant-scoped client
app.use('/api', tenantMiddleware);

app.get('/api/data/:key', async (req, res) => {
  try {
    const result = await req.tenantClient.get(req.params.key);
    res.json({ value: bytesToString(result.value) });
  } catch (err) {
    if (err instanceof KeyNotFoundError) {
      return res.status(404).json({ error: 'Not found' });
    }
    throw err;
  }
});

app.post('/api/data/:key', async (req, res) => {
  await req.tenantClient.put(req.params.key, req.body.value);
  res.json({ success: true });
});
```

### Usage Example

```typescript
// Create tenants
await tenantManager.create('tenant-acme', 'Acme Corp', 'pro');
await tenantManager.create('tenant-widgets', 'Widgets Inc', 'enterprise');

// Get tenant-scoped clients
const acmeClient = await tenantManager.getClient('tenant-acme');
const widgetsClient = await tenantManager.getClient('tenant-widgets');

// Isolated operations
await acmeClient.put('config', 'acme-config');
await widgetsClient.put('config', 'widgets-config');

// Data is isolated
const acmeConfig = await acmeClient.get('config'); // 'acme-config'
const widgetsConfig = await widgetsClient.get('config'); // 'widgets-config'
```

## Best Practices

### 1. Always Use try-catch with async/await

```typescript
try {
  await client.put(key, value);
} catch (err) {
  console.error('Operation failed:', err);
}
```

### 2. Implement Retry Logic for CAS

```typescript
async function casRetry<T>(
  operation: () => Promise<T>,
  maxRetries: number = 10
): Promise<T> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await operation();
    } catch (err) {
      if (!(err instanceof VersionMismatchError) || attempt === maxRetries - 1) {
        throw err;
      }
      await new Promise(r => setTimeout(r, Math.pow(2, attempt) * 10));
    }
  }
  throw new Error('Unreachable');
}
```

### 3. Use TypeScript Types

```typescript
interface UserData {
  id: string;
  name: string;
  email: string;
}

async function getUser(key: string): Promise<UserData> {
  const result = await client.get(key);
  return JSON.parse(bytesToString(result.value)) as UserData;
}
```

### 4. Clean Up Resources

```typescript
const client = new NoriKVClient(config);
await client.connect();

try {
  // Use client
} finally {
  await client.close();
}
```

### 5. Use Idempotency Keys for Critical Operations

```typescript
await client.put(key, value, {
  idempotencyKey: `operation-${operationId}`,
});
```

## Next Steps

- [API Guide](API_GUIDE.html) - Core API reference
- [Architecture Guide](ARCHITECTURE.html) - Internal design
- [Troubleshooting Guide](TROUBLESHOOTING.html) - Common issues
- [GitHub Examples](https://github.com/j-haj/nori/tree/main/sdks/typescript/examples) - More code samples
