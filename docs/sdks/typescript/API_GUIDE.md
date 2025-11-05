---
layout: default
title: API Guide
parent: TypeScript SDK
grand_parent: Client SDKs
nav_order: 1
---

# NoriKV TypeScript Client API Guide

Complete reference for the NoriKV TypeScript/JavaScript Client SDK.

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
npm install @norikv/client
# or
yarn add @norikv/client
# or
pnpm add @norikv/client
```

## Quick Start

```typescript
import { NoriKVClient, bytesToString } from '@norikv/client';

const client = new NoriKVClient({
  nodes: ['localhost:9001', 'localhost:9002'],
  totalShards: 1024,
  timeout: 5000,
});

await client.connect();

// Put a value
await client.put('user:alice', 'Alice');

// Get the value
const result = await client.get('user:alice');
console.log(bytesToString(result.value)); // 'Alice'

// Delete
await client.delete('user:alice');

await client.close();
```

## Client Configuration

### Basic Configuration

```typescript
import { NoriKVClient, ClientConfig } from '@norikv/client';

const config: ClientConfig = {
  nodes: ['node1:9001', 'node2:9001'],
  totalShards: 1024,
  timeout: 5000,
};

const client = new NoriKVClient(config);
```

### Configuration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `nodes` | `string[]` | **Required** | List of node addresses (host:port) |
| `totalShards` | `number` | **Required** | Total number of shards in cluster |
| `timeout` | `number` | 5000 | Request timeout in milliseconds |
| `retry` | `RetryConfig` | See below | Retry policy configuration |

### Retry Configuration

```typescript
import { RetryConfig } from '@norikv/client';

const retryConfig: RetryConfig = {
  maxAttempts: 10,
  initialDelayMs: 100,
  maxDelayMs: 5000,
  jitterMs: 100,
};

const client = new NoriKVClient({
  nodes: ['localhost:9001'],
  totalShards: 1024,
  retry: retryConfig,
});
```

**Retry Behavior:**
- Retries transient errors: `Unavailable`, `Aborted`, `DeadlineExceeded`
- Does NOT retry: `InvalidArgument`, `NotFound`, `FailedPrecondition`
- Uses exponential backoff with jitter

## Core Operations

### PUT - Write Data

#### Basic PUT

```typescript
const key = 'user:123';
const value = JSON.stringify({ name: 'Alice' });

const version = await client.put(key, value);
console.log('Written at version:', version);
```

#### PUT with Options

```typescript
import { PutOptions } from '@norikv/client';

const options: PutOptions = {
  ttlMs: 60000,                    // TTL: 60 seconds
  idempotencyKey: 'order-12345',   // Idempotency key
  ifMatchVersion: expectedVersion, // CAS
};

const version = await client.put(key, value, options);
```

**PutOptions Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `ttlMs` | `number?` | Time-to-live in milliseconds |
| `idempotencyKey` | `string?` | Key for idempotent operations |
| `ifMatchVersion` | `Version?` | Expected version for CAS |

### GET - Read Data

#### Basic GET

```typescript
const result = await client.get('user:123');

const value = bytesToString(result.value);
const version = result.version;
```

#### GET with Consistency Level

```typescript
import { GetOptions, ConsistencyLevel } from '@norikv/client';

const options: GetOptions = {
  consistency: ConsistencyLevel.LINEARIZABLE,
};

const result = await client.get(key, options);
```

**Consistency Levels:**

| Level | Description | Use Case |
|-------|-------------|----------|
| `LEASE` | Default, lease-based read | Most operations (fast, usually consistent) |
| `LINEARIZABLE` | Strictest, always up-to-date | Critical reads requiring absolute consistency |
| `STALE_OK` | May return stale data | Read-heavy workloads, caching |

### DELETE - Remove Data

#### Basic DELETE

```typescript
const deleted = await client.delete('user:123');
console.log('Deleted:', deleted);
```

#### DELETE with Options

```typescript
import { DeleteOptions } from '@norikv/client';

const options: DeleteOptions = {
  idempotencyKey: 'delete-order-12345',
  ifMatchVersion: expectedVersion,
};

await client.delete(key, options);
```

## Advanced Features

### Compare-And-Swap (CAS)

Optimistic concurrency control using version matching:

```typescript
// Read current value
const current = await client.get(key);
const value = parseInt(bytesToString(current.value));

// Update with CAS
const newValue = (value + 1).toString();
try {
  await client.put(key, newValue, {
    ifMatchVersion: current.version,
  });
  console.log('CAS succeeded');
} catch (err) {
  if (err instanceof VersionMismatchError) {
    console.log('CAS failed - version changed');
  }
  throw err;
}
```

### Idempotent Operations

Safe retries using idempotency keys:

```typescript
import { v4 as uuidv4 } from 'uuid';

const idempotencyKey = `payment-${uuidv4()}`;

// First attempt
const v1 = await client.put(key, value, { idempotencyKey });

// Retry with same key (safe - returns same version)
const v2 = await client.put(key, value, { idempotencyKey });

console.log(v1.equals(v2)); // true
```

### Time-To-Live (TTL)

Automatic expiration:

```typescript
await client.put(key, value, {
  ttlMs: 60000, // Expires in 60 seconds
});

// Key automatically deleted after TTL
await new Promise(resolve => setTimeout(resolve, 61000));

try {
  await client.get(key);
} catch (err) {
  if (err instanceof KeyNotFoundError) {
    console.log('Key expired');
  }
}
```

### Cluster Topology

Monitor cluster changes:

```typescript
// Get current cluster view
const view = client.getClusterView();
if (view) {
  console.log('Cluster epoch:', view.epoch);
  console.log('Nodes:', view.nodes.length);
}

// Subscribe to topology changes
const unsubscribe = client.onTopologyChange((event) => {
  console.log('Topology changed!');
  console.log('Previous epoch:', event.previousEpoch);
  console.log('Current epoch:', event.currentEpoch);
  console.log('Added nodes:', event.addedNodes);
  console.log('Removed nodes:', event.removedNodes);
});

// Later: unsubscribe
unsubscribe();
```

### Client Statistics

Monitor client performance:

```typescript
const stats = client.getStats();

console.log('Active connections:', stats.pool.activeConnections);
console.log('Total nodes:', stats.router.totalNodes);
console.log('Cached leaders:', stats.topology.cachedLeaders);
```

## Error Handling

### Error Types

```typescript
import {
  KeyNotFoundError,
  VersionMismatchError,
  AlreadyExistsError,
  ConnectionError,
  NoriKVError,
} from '@norikv/client';
```

### Handling Specific Errors

```typescript
try {
  const result = await client.get(key);
} catch (err) {
  if (err instanceof KeyNotFoundError) {
    console.log('Key not found');
  } else if (err instanceof ConnectionError) {
    console.log('Connection error:', err.message);
  } else if (err instanceof NoriKVError) {
    console.log('Error:', err.code, err.message);
  } else {
    throw err;
  }
}
```

### Retry Pattern

```typescript
async function retryOperation<T>(
  operation: () => Promise<T>,
  maxAttempts: number = 3
): Promise<T> {
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await operation();
    } catch (err) {
      if (!(err instanceof ConnectionError)) {
        throw err; // Non-retryable
      }
      
      if (attempt === maxAttempts) {
        throw err; // Give up
      }
      
      // Exponential backoff
      await new Promise(resolve => 
        setTimeout(resolve, Math.pow(2, attempt) * 100)
      );
    }
  }
  throw new Error('Unreachable');
}

// Usage
const result = await retryOperation(() => client.put(key, value));
```

### Graceful Degradation

```typescript
async function getWithFallback(
  client: NoriKVClient,
  key: string,
  defaultValue: string
): Promise<string> {
  try {
    const result = await client.get(key);
    return bytesToString(result.value);
  } catch (err) {
    console.warn('Failed to get key, using default:', err);
    return defaultValue;
  }
}
```

## Best Practices

### 1. Always await connect() and close()

```typescript
const client = new NoriKVClient(config);
await client.connect();

try {
  // Use client
} finally {
  await client.close();
}
```

### 2. Reuse Client Instances

```typescript
//  Good: Single client instance
let client: NoriKVClient;

async function init() {
  client = new NoriKVClient(config);
  await client.connect();
}

//  Bad: Creating client per request
async function handleRequest() {
  const client = new NoriKVClient(config);
  await client.connect();
  await client.close(); // Closes connections!
}
```

### 3. Use TypeScript Types

```typescript
import { GetResult, Version, PutOptions } from '@norikv/client';

async function updateUser(userId: string, data: UserData): Promise<Version> {
  const key = `user:${userId}`;
  const value = JSON.stringify(data);
  
  const options: PutOptions = {
    ttlMs: 3600000,
  };
  
  return await client.put(key, value, options);
}
```

### 4. Handle Errors Properly

```typescript
async function safeGet(key: string): Promise<string | null> {
  try {
    const result = await client.get(key);
    return bytesToString(result.value);
  } catch (err) {
    if (err instanceof KeyNotFoundError) {
      return null;
    }
    throw err;
  }
}
```

### 5. Use Async/Await Consistently

```typescript
//  Good: Clean async/await
async function processUser(userId: string) {
  const userData = await client.get(`user:${userId}`);
  const processed = await processData(userData.value);
  await client.put(`processed:${userId}`, processed);
}

//  Bad: Mixing promises and async/await
async function processUserBad(userId: string) {
  return client.get(`user:${userId}`).then(userData => {
    return processData(userData.value).then(processed => {
      return client.put(`processed:${userId}`, processed);
    });
  });
}
```

### 6. Use Idempotency Keys for Important Operations

```typescript
async function createOrder(orderId: string, data: OrderData) {
  await client.put(`order:${orderId}`, JSON.stringify(data), {
    idempotencyKey: `create-order-${orderId}`,
  });
}
```

### 7. Choose Appropriate Consistency

```typescript
// For critical reads
const result = await client.get(key, {
  consistency: ConsistencyLevel.LINEARIZABLE,
});

// For cache-like reads
const result = await client.get(key, {
  consistency: ConsistencyLevel.STALE_OK,
});
```

### 8. Use Proper Encoding

```typescript
import { stringToBytes, bytesToString } from '@norikv/client';

// Always use UTF-8 encoding helpers
const value = stringToBytes('Hello, World!');
await client.put(key, value);

const result = await client.get(key);
const text = bytesToString(result.value);
```

## Performance Tips

### 1. Batch Operations with Promise.all

```typescript
// Process multiple operations concurrently
await Promise.all(
  keys.map(key => client.put(key, value))
);
```

### 2. Connection Pooling (Automatic)

The client maintains connection pools internally - no external pooling needed.

### 3. Avoid Creating Clients Per Request

Reuse client instances across requests for better performance.

### 4. Use Appropriate Value Sizes

- Optimal: 100 bytes - 10 KB
- Maximum: Limited by memory and network

### 5. Monitor Performance

```typescript
const start = Date.now();
await client.put(key, value);
const duration = Date.now() - start;
console.log(`PUT took ${duration}ms`);
```

## Complete Example

```typescript
import {
  NoriKVClient,
  ClientConfig,
  PutOptions,
  GetOptions,
  ConsistencyLevel,
  bytesToString,
  stringToBytes,
} from '@norikv/client';

async function main() {
  // Configure with retry policy
  const config: ClientConfig = {
    nodes: ['localhost:9001', 'localhost:9002'],
    totalShards: 1024,
    timeout: 5000,
    retry: {
      maxAttempts: 5,
      initialDelayMs: 100,
      maxDelayMs: 2000,
    },
  };

  const client = new NoriKVClient(config);
  await client.connect();

  try {
    // Write with TTL and idempotency
    const key = 'session:abc123';
    const value = JSON.stringify({ userId: 42 });

    const putOpts: PutOptions = {
      ttlMs: 3600000, // 1 hour
      idempotencyKey: 'session-create-abc123',
    };

    const version = await client.put(key, stringToBytes(value), putOpts);
    console.log('Written:', version);

    // Read with linearizable consistency
    const getOpts: GetOptions = {
      consistency: ConsistencyLevel.LINEARIZABLE,
    };

    const result = await client.get(key, getOpts);
    console.log('Read:', bytesToString(result.value));

    // Update with CAS
    const newValue = JSON.stringify({ userId: 42, active: true });
    
    try {
      await client.put(key, stringToBytes(newValue), {
        ifMatchVersion: result.version,
      });
      console.log('CAS succeeded');
    } catch (err) {
      if (err instanceof VersionMismatchError) {
        console.log('CAS failed - retry needed');
      }
    }

    // Monitor topology
    client.onTopologyChange((event) => {
      console.log('Cluster changed: epoch', event.currentEpoch);
    });

    // Get statistics
    const stats = client.getStats();
    console.log('Stats:', stats);

  } finally {
    await client.close();
  }
}

main().catch(console.error);
```

## Next Steps

- [Architecture Guide](ARCHITECTURE.html) - Understanding client internals
- [Troubleshooting Guide](TROUBLESHOOTING.html) - Solving common issues
- [Advanced Patterns](ADVANCED_PATTERNS.html) - Complex use cases
- [Examples](https://github.com/j-haj/nori/tree/main/sdks/typescript/examples) - Working code samples
