# NoriKV TypeScript Client

TypeScript client for **NoriKV** - a sharded, Raft-replicated, log-structured key-value store with first-class observability.

## Features

- **Smart Client**: Client-side routing with hash-based shard assignment (Jump Consistent Hash + xxhash64)
- **Leader-Aware Routing**: Direct requests to shard leaders, handle NOT_LEADER redirects automatically
- **Retry Logic**: Exponential backoff with jitter for transient failures
- **Idempotency**: Safe retries with idempotency keys
- **Conditional Operations**: Compare-and-swap (CAS) and if-not-exists semantics
- **Consistency Levels**: Lease-based (fast), linearizable (strict), or stale reads
- **Connection Pooling**: Efficient gRPC channel management per node
- **Cluster Topology Tracking**: Watch and react to cluster membership changes
- **TypeScript-First**: Comprehensive types for type-safe development
- **Dual Package**: ESM and CommonJS support

## Installation

```bash
npm install @norikv/client
# or
yarn add @norikv/client
# or
pnpm add @norikv/client
```

## Quick Start

### Ephemeral Mode (Easiest for Testing)

The fastest way to get started is with an ephemeral (in-memory) NoriKV instance:

```typescript
import { createEphemeral, bytesToString } from '@norikv/client';

// Create an ephemeral server (auto-starts and uses temp directory)
const cluster = await createEphemeral();
const client = cluster.getClient();

await client.connect();

// Put a value
await client.put('user:123', 'Alice');

// Get a value
const result = await client.get('user:123');
console.log(bytesToString(result.value)); // 'Alice'

// Delete a value
await client.delete('user:123');

await client.close();
await cluster.stop(); // Clean up
```

**Requirements**: The `norikv-server` binary must be available in your PATH. You can:
- Build it: `cargo build --release -p norikv-server`
- Or set `NORIKV_SERVER_PATH` environment variable to point to the binary

### Connecting to an Existing Cluster

```typescript
import { NoriKVClient, bytesToString } from '@norikv/client';

const client = new NoriKVClient({
  nodes: ['localhost:7447', 'localhost:7448', 'localhost:7449'],
});

await client.connect();

// Put a value
await client.put('user:123', 'Alice');

// Get a value
const result = await client.get('user:123');
console.log(bytesToString(result.value)); // 'Alice'

// Delete a value
await client.delete('user:123');

await client.close();
```

## API Reference

### Client Configuration

```typescript
interface ClientConfig {
  /** List of node addresses (at least one required) */
  nodes: string[];

  /** Total number of virtual shards (default: 1024) */
  totalShards?: number;

  /** Default request timeout in milliseconds (default: 5000) */
  timeout?: number;

  /** Retry policy configuration */
  retry?: {
    maxAttempts?: number;           // default: 3
    initialDelayMs?: number;        // default: 10
    maxDelayMs?: number;            // default: 1000
    backoffMultiplier?: number;     // default: 2
    jitterMs?: number;              // default: 100
    retryOnNotLeader?: boolean;     // default: true
  };

  /** Enable cluster view watching (default: true) */
  watchCluster?: boolean;

  /** Max connections per node (default: 10) */
  maxConnectionsPerNode?: number;
}
```

### Put Operations

```typescript
// Simple put
const version = await client.put('key', 'value');

// Put with TTL (expires in 10 seconds)
await client.put('key', 'value', { ttlMs: 10000 });

// Conditional put: only if key doesn't exist
await client.put('key', 'value', { ifNotExists: true });

// Optimistic locking (CAS): only if version matches
const result = await client.get('key');
await client.put('key', 'new-value', {
  ifMatchVersion: result.version,
});

// Idempotent put (safe to retry)
await client.put('key', 'value', {
  idempotencyKey: 'unique-request-id-123',
});
```

### Get Operations

```typescript
// Default read (lease-based, fast linearizable)
const result = await client.get('key');

// Strict linearizable read (read-index + quorum)
const result = await client.get('key', {
  consistency: 'linearizable',
});

// Stale read (eventual consistency, fastest)
const result = await client.get('key', {
  consistency: 'stale_ok',
});

// Check if key exists
if (result.value !== null) {
  console.log('Key exists:', bytesToString(result.value));
  console.log('Version:', result.version);
}
```

### Delete Operations

```typescript
// Simple delete
await client.delete('key');

// Conditional delete (CAS)
const result = await client.get('key');
await client.delete('key', {
  ifMatchVersion: result.version,
});

// Idempotent delete
await client.delete('key', {
  idempotencyKey: 'delete-key-123',
});
```

### Consistency Levels

NoriKV supports three consistency levels for reads:

| Level | Description | Use Case |
|-------|-------------|----------|
| `lease` (default) | Lease-based linearizable read. Leader serves from its lease without quorum. | Balanced performance and strong consistency |
| `linearizable` | Strict linearizable read. Uses read-index protocol with quorum confirmation. | When you need guaranteed up-to-date reads |
| `stale_ok` | Stale read from any replica. Fastest but may return stale data. | High-throughput read workloads, caching |

### Error Handling

```typescript
import {
  NotLeaderError,
  AlreadyExistsError,
  VersionMismatchError,
  UnavailableError,
  ConnectionError,
} from '@norikv/client';

try {
  await client.put('key', 'value', { ifNotExists: true });
} catch (err) {
  if (err instanceof AlreadyExistsError) {
    console.log('Key already exists');
  } else if (err instanceof VersionMismatchError) {
    console.log('Version conflict - retry with new version');
  } else if (err instanceof NotLeaderError) {
    // Automatically retried by client
    console.log('Wrong leader (should not reach here)');
  } else if (err instanceof UnavailableError) {
    console.log('Service temporarily unavailable');
  } else if (err instanceof ConnectionError) {
    console.log('Network error');
  }
}
```

### Cluster Topology

```typescript
// Get current cluster view
const clusterView = client.getClusterView();
console.log(`Epoch: ${clusterView.epoch}`);
console.log(`Nodes: ${clusterView.nodes.length}`);
console.log(`Shards: ${clusterView.shards.length}`);

// Watch topology changes
const unsubscribe = client.onTopologyChange((event) => {
  console.log(`Cluster changed: epoch ${event.previousEpoch} → ${event.currentEpoch}`);
  console.log(`Added nodes: ${event.addedNodes}`);
  console.log(`Removed nodes: ${event.removedNodes}`);
  console.log(`Leader changes: ${event.leaderChanges.size}`);
});

// Stop watching
unsubscribe();
```

## Advanced Usage

### Custom Retry Policy

```typescript
import { retryPolicy } from '@norikv/client';

const client = new NoriKVClient({
  nodes: ['localhost:50051'],
  retry: retryPolicy()
    .maxAttempts(5)
    .initialDelay(50)
    .maxDelay(5000)
    .backoffMultiplier(3)
    .jitter(200)
    .build(),
});
```

### Working with Binary Data

```typescript
// Keys and values can be Uint8Array or string
const binaryKey = new Uint8Array([1, 2, 3, 4]);
const binaryValue = new Uint8Array([10, 20, 30, 40]);

await client.put(binaryKey, binaryValue);

const result = await client.get(binaryKey);
console.log(result.value); // Uint8Array

// Convert between string and bytes
import { keyToBytes, valueToBytes, bytesToString } from '@norikv/client';

const keyBytes = keyToBytes('my-key');
const valueBytes = valueToBytes('my-value');
const str = bytesToString(valueBytes);
```

### Sharding and Routing

```typescript
import { getShardForKey, xxhash64, jumpConsistentHash } from '@norikv/client';

// Determine which shard a key belongs to (same logic as server)
const shardId = getShardForKey('user:123', 1024);
console.log(`Key belongs to shard ${shardId}`);

// Low-level hash functions (for testing/debugging)
const hash = xxhash64('my-key'); // 64-bit hash
const bucket = jumpConsistentHash(hash, 1024); // Consistent hash bucket
```

## Testing

```bash
# Run unit tests
npm test

# Run integration tests (requires running cluster)
npm run test:integration

# Run tests with coverage
npm run test:coverage
```

## Building

```bash
# Install dependencies
npm install

# Build (ESM + CJS + TypeScript declarations)
npm run build

# Watch mode
npm run build:watch

# Type check
npm run type-check

# Lint
npm run lint

# Format
npm run format
```

## Architecture

The TypeScript client implements a **smart client** architecture:

1. **Hash-Based Routing**: Uses xxhash64 + Jump Consistent Hash to compute shard assignments (identical to server logic)
2. **Leader Tracking**: Maintains cluster topology and routes requests directly to shard leaders
3. **NOT_LEADER Handling**: Automatically retries on the correct leader when redirected
4. **Connection Pooling**: Maintains gRPC channels to all nodes with automatic reconnection
5. **Exponential Backoff**: Retries transient failures with jitter to avoid thundering herds

```
┌───────────────────────────────────────────┐
│          NoriKVClient                     │
│  - put(key, value, options)               │
│  - get(key, options)                      │
│  - delete(key, options)                   │
└───────────┬───────────────────────────────┘
            │
    ┌───────┴────────┬────────────────┐
    │                │                │
┌───▼───┐    ┌──────▼─────┐   ┌─────▼──────┐
│Router │    │  Topology  │   │Connection  │
│       │    │  Manager   │   │    Pool    │
└───┬───┘    └──────┬─────┘   └─────┬──────┘
    │               │               │
    │      ┌────────┴────────┐      │
    │      │   ClusterView   │      │
    │      │  - epoch        │      │
    │      │  - nodes[]      │      │
    │      │  - shards[]     │      │
    │      └─────────────────┘      │
    │                               │
    └───────────┬───────────────────┘
                │
        ┌───────▼────────┐
        │  gRPC Channels │
        │  (per node)    │
        └────────────────┘
```

## Hash Function Compatibility

**CRITICAL**: The client's hash functions (xxhash64 + jumpConsistentHash) **MUST** produce identical results to the server's Rust implementation. Any deviation will cause routing failures and data loss.

Both implementations:
- Use xxhash64 with seed=0
- Use the Jump Consistent Hash algorithm from the reference paper
- Default to 1024 virtual shards

Test vectors are validated in `tests/unit/hash.test.ts`.

## Protocol Buffers

The client uses gRPC for communication with the server. Protocol buffer definitions are in `context/40_protocols.proto`:

- `service Kv`: Put/Get/Delete operations
- `service Meta`: Cluster topology watching
- `service Admin`: Administrative operations

## License

MIT OR Apache-2.0

## Contributing

See the main [NoriKV repository](https://github.com/norikv/norikv) for contribution guidelines.

## Status

**Alpha** - Core functionality implemented, pending:
- [ ] gRPC stub generation (currently using manual proto types)
- [ ] Integration tests against running cluster
- [ ] Cross-language hash validation tests
- [ ] Performance benchmarks

## Examples

See [examples/](./examples/) directory:
- `basic.ts` - Complete walkthrough of all features

## Support

For issues, questions, or contributions, please visit:
- GitHub: https://github.com/norikv/norikv
- Documentation: https://norikv.dev/docs
