# NoriKV TypeScript SDK Integration Tests

This directory contains integration tests for the NoriKV TypeScript client SDK. The tests are organized into two categories: **mock-based tests** and **E2E tests**.

## Test Structure

```
tests/integration/
├── README.md                      # This file
├── helpers/                        # Shared test utilities
│   ├── assertions.ts              # Custom assertion helpers
│   ├── mock-server.ts             # Mock gRPC server implementation
│   └── test-server.ts             # Real server test harness
├── mock/                          # Mock-based tests (fast, no dependencies)
│   ├── crud.test.ts               # CRUD operations
│   ├── errors.test.ts             # Error handling
│   ├── topology.test.ts           # Cluster topology management
│   └── advanced.test.ts           # Advanced features (TTL, consistency, etc.)
└── e2e/                           # E2E tests (real server required)
    ├── basic.test.ts              # Basic operations with real server
    └── cluster.test.ts            # Multi-node cluster tests
```

## Running Tests

### Prerequisites

**For all tests:**
```bash
npm install
npm run build
```

**For E2E tests only:**
```bash
# Build the NoriKV server binary (from repository root)
cd ../../../../
cargo build -p norikv-server
cd sdks/typescript
```

### Test Commands

```bash
# Run all integration tests (mock + E2E)
npm run test:integration

# Run only mock-based tests (fast, no server needed)
npm run test:integration:mock

# Run only E2E tests (requires compiled server)
npm run test:integration:e2e

# Watch mode for integration tests
npm run test:integration:watch

# Run all tests (unit + integration)
npm run test:all
```

## Mock-Based Tests

Mock-based tests use an in-memory mock gRPC server and run quickly without external dependencies. They're ideal for:
- Fast CI/CD pipelines
- Local development
- Testing client behavior in isolation
- Testing error handling scenarios

### Test Coverage

**`crud.test.ts`** - Basic CRUD operations:
- Put operations (string/Buffer keys and values, overwrite, empty values, large values)
- Get operations (all consistency levels)
- Delete operations
- Version tracking
- Concurrent operations
- Edge cases (Unicode, special characters, binary data)

**`errors.test.ts`** - Error handling:
- NOT_LEADER errors and redirects
- Network errors (timeout, unavailable, connection failures)
- Invalid arguments
- Conditional operation failures (ifNotExists, ifMatchVersion)
- Resource exhaustion
- Retry logic and backoff
- Connection pool errors
- Client state errors
- Error recovery

**`topology.test.ts`** - Cluster topology:
- Initial cluster view
- Cluster view updates and epoch tracking
- Node discovery and removal
- Shard assignments and leader changes
- Shard rebalancing
- Topology change listeners
- Multi-node scenarios
- Edge cases (rapid changes, out-of-order updates)

**`advanced.test.ts`** - Advanced features:
- TTL/expiration
- Conditional puts (ifNotExists, ifMatchVersion)
- Conditional deletes
- All consistency levels
- Idempotency keys
- Metadata
- Large-scale operations (100+ concurrent ops)
- Complex workflows:
  - Read-modify-write patterns
  - Transaction-like patterns
  - Caching with TTL
  - Distributed counter pattern
- Performance characteristics

### Running Mock Tests

```bash
# Run all mock tests
npm run test:integration:mock

# Run specific test file
npx vitest run tests/integration/mock/crud.test.ts

# Watch mode for specific file
npx vitest tests/integration/mock/crud.test.ts
```

## E2E Tests

E2E tests use real NoriKV server instances and provide end-to-end validation. They're ideal for:
- Validating against real server behavior
- Testing multi-node clusters
- Testing failover and replication
- Pre-release validation

### Prerequisites

E2E tests require a compiled `norikv-server` binary:

```bash
# From repository root
cargo build -p norikv-server

# Or for release build
cargo build -p norikv-server --release
```

### Test Coverage

**`basic.test.ts`** - Single-node operations:
- Basic CRUD operations
- All consistency levels (lease, linearizable, stale_ok)
- Concurrent operations
- Version tracking
- Client state management

**`cluster.test.ts`** - Multi-node cluster:
- Cluster connectivity
- Data replication
- Node failures and recovery
- Consistency levels in cluster
- Load distribution across shards
- Topology change tracking

### Running E2E Tests

```bash
# Run all E2E tests (requires server binary)
npm run test:integration:e2e

# Run specific test file
npx vitest run --config vitest.integration.config.ts tests/integration/e2e/basic.test.ts

# Run with verbose server logs
NORIKV_VERBOSE=1 npm run test:integration:e2e
```

### E2E Test Configuration

E2E tests use longer timeouts to accommodate server startup:

- **Test timeout**: 30 seconds (single node), 60 seconds (cluster)
- **Hook timeout**: 30 seconds
- **Teardown timeout**: 10 seconds

Tests run sequentially (`singleFork: true`) to avoid port conflicts.

## Test Helpers

### Assertions (`helpers/assertions.ts`)

Custom assertion functions for cleaner test code:

```typescript
// Version assertions
expectValidVersion(version);
expectVersionsEqual(v1, v2);

// Key existence
await expectKeyExists(client, key, expectedValue);
await expectKeyNotExists(client, key);

// Eventual consistency
await expectEventualConsistency(client, key, value, {
  timeoutMs: 5000,
  pollIntervalMs: 100,
});

// Utilities
await waitFor(() => condition, { timeoutMs: 5000 });
await sleep(1000);
const key = randomKey('prefix');
const value = randomValue(16);
```

### Mock Server (`helpers/mock-server.ts`)

In-memory gRPC server for testing:

```typescript
const server = await createMockServer();
const client = new NoriKVClient({
  nodes: [server.getAddress()],
  watchCluster: false,
});

// Test operations...

// Inject errors
server.setNotLeader('other-node-addr');
server.setError(grpcError);

// Verify behavior
const calls = server.getCallsForMethod('Put');

// Cleanup
await server.stop();
```

### Test Server (`helpers/test-server.ts`)

Real server lifecycle management:

```typescript
// Single server
const server = await startTestServer({
  grpcPort: 50051,
  totalShards: 1024,
  verbose: false,
});
await server.cleanup();

// Cluster
const cluster = await startTestCluster({
  nodeCount: 3,
  totalShards: 1024,
});
await cluster.stopNode('node-1');
await cluster.restartNode('node-1');
await cluster.stop();

// With helpers
await withTestServer({ verbose: true }, async (server) => {
  // Test code
});

await withTestCluster({ nodeCount: 3 }, async (cluster) => {
  // Test code
});
```

## Writing New Tests

### Mock-Based Test Template

```typescript
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import { MockNoriKVServer, createMockServer } from '../helpers/mock-server';
import { expectKeyExists, randomKey, randomValue } from '../helpers/assertions';

describe('My Feature', () => {
  let server: MockNoriKVServer;
  let client: NoriKVClient;

  beforeEach(async () => {
    server = await createMockServer();
    client = new NoriKVClient({
      nodes: [server.getAddress()],
      watchCluster: false,
    });
    await client.connect();
  });

  afterEach(async () => {
    await client.close();
    await server.stop();
  });

  it('should do something', async () => {
    const key = randomKey('test');
    const value = randomValue();

    await client.put(key, value);
    await expectKeyExists(client, key, value);
  });
});
```

### E2E Test Template

```typescript
import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import { NoriKVClient } from '@norikv/client/client';
import { startTestServer, type TestServerInstance } from '../helpers/test-server';
import { expectKeyExists, randomKey, randomValue } from '../helpers/assertions';

describe('My E2E Feature', () => {
  let server: TestServerInstance;
  let client: NoriKVClient;

  beforeAll(async () => {
    server = await startTestServer();
    client = new NoriKVClient({
      nodes: [server.grpcAddr],
      watchCluster: true,
    });
    await client.connect();
  }, 30000);

  afterAll(async () => {
    await client?.close();
    await server?.cleanup();
  });

  it('should do something', async () => {
    const key = randomKey('e2e');
    const value = randomValue();

    await client.put(key, value);
    await expectKeyExists(client, key, value);
  });
});
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: Integration Tests

on: [push, pull_request]

jobs:
  mock-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions/setup-node@v3
        with:
          node-version: '18'
      - run: npm ci
      - run: npm run build
      - run: npm run test:integration:mock

  e2e-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions/setup-node@v3
        with:
          node-version: '18'
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
      - run: cargo build -p norikv-server
      - run: npm ci
      - run: npm run build
      - run: npm run test:integration:e2e
```

### Test Strategy Recommendations

1. **PR Validation**: Run mock tests on every PR
2. **Nightly Builds**: Run full E2E tests nightly
3. **Pre-Release**: Run all tests (unit + integration) before release
4. **Local Development**: Use mock tests for rapid iteration

## Debugging Tests

### Enable Verbose Logging

```bash
# For E2E tests with server logs
RUST_LOG=debug npm run test:integration:e2e

# For specific test with vitest debug
DEBUG=1 npx vitest run tests/integration/mock/crud.test.ts
```

### Common Issues

**"Server binary not found"**
- Build the server: `cargo build -p norikv-server`
- Check binary location matches test-server.ts `findServerBinary()`

**"Port already in use"**
- Tests use random ports by default
- Ensure previous test runs cleaned up properly
- Kill any stray processes: `pkill norikv-server`

**"Connection timeout"**
- Increase timeout in test configuration
- Check firewall/network settings
- Verify server starts successfully

**Mock vs Real Behavior Mismatch**
- Mock server doesn't implement all server features
- Some behaviors (like exact error messages) may differ
- Use E2E tests to validate critical paths

## Performance

### Mock Tests
- **Total runtime**: ~1-2 seconds for all mock tests
- **Concurrent execution**: Yes (tests are isolated)
- **Resource usage**: Minimal (in-memory only)

### E2E Tests
- **Single node startup**: ~2-5 seconds
- **Cluster startup**: ~5-10 seconds
- **Total runtime**: ~30-60 seconds for all E2E tests
- **Concurrent execution**: No (sequential to avoid port conflicts)
- **Resource usage**: Moderate (spawns real processes)

## Contributing

When adding new features to the SDK:

1. **Add mock tests** for the new functionality
2. **Add E2E tests** if the feature requires server validation
3. **Update this README** if adding new test categories
4. **Follow existing patterns** for consistency

## Test Coverage Goals

- **Unit tests**: >80% code coverage
- **Integration tests**: All major API paths covered
- **E2E tests**: Critical user journeys validated

Current coverage can be checked with:
```bash
npm run test:integration -- --coverage
```
