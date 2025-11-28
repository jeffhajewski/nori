# NoriKV TypeScript Client Troubleshooting Guide

Solutions to common issues when using the TypeScript/JavaScript client SDK.

## Connection Issues

### "Connection refused" or ECONNREFUSED

**Symptoms:**
```typescript
await client.connect();
// Error: connect ECONNREFUSED 127.0.0.1:9001
```

**Solutions:**

1. Verify server is running:
```bash
netstat -an | grep 9001
lsof -i :9001
```

2. Check client configuration:
```typescript
const client = new NoriKVClient({
  nodes: ['localhost:9001'], // Verify this address
  totalShards: 1024,
});
```

3. Test connectivity:
```bash
telnet localhost 9001
nc -zv localhost 9001
```

### "Deadline exceeded" or timeout errors

**Symptoms:**
```typescript
const result = await client.get(key);
// Error: Deadline exceeded
```

**Solutions:**

1. Increase timeout:
```typescript
const client = new NoriKVClient({
  nodes: ['localhost:9001'],
  timeout: 10000, // 10 seconds
});
```

2. Enable retries:
```typescript
const client = new NoriKVClient({
  nodes: ['localhost:9001'],
  retry: {
    maxAttempts: 10,
    initialDelayMs: 100,
    maxDelayMs: 5000,
  },
});
```

## Performance Problems

### Slow operations

**Diagnosis:**
```typescript
console.time('put');
await client.put(key, value);
console.timeEnd('put'); // > 100ms consistently
```

**Solutions:**

1. Use appropriate consistency level:
```typescript
const result = await client.get(key, {
  consistency: ConsistencyLevel.STALE_OK, // Fastest
});
```

2. Batch operations:
```typescript
await Promise.all(
  keys.map(k => client.put(k, value))
);
```

3. Check value sizes:
```typescript
console.log('Value size:', value.length, 'bytes');
// Optimal: 100 bytes - 10 KB
```

### High memory usage

**Solutions:**

1. Close client when done:
```typescript
await client.close(); // Important!
```

2. Clean up topology listeners:
```typescript
const unsubscribe = client.onTopologyChange(handler);
unsubscribe(); // Clean up when done
```

3. Use Node.js profiling:
```bash
node --inspect index.js
# Open chrome://inspect
```

## Version Conflicts

### Frequent VersionMismatchError

**Symptoms:**
```typescript
await client.put(key, newValue, {
  ifMatchVersion: version,
});
// Error: Version mismatch
```

**Solution - Implement retry loop:**
```typescript
async function casWithRetry(
  key: string,
  transform: (value: string) => string,
  maxRetries: number = 10
): Promise<void> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const result = await client.get(key);
      const newValue = transform(bytesToString(result.value));
      
      await client.put(key, newValue, {
        ifMatchVersion: result.version,
      });
      return; // Success
    } catch (err) {
      if (!(err instanceof VersionMismatchError)) {
        throw err;
      }
      if (i === maxRetries - 1) {
        throw new Error('CAS failed after retries');
      }
      // Exponential backoff
      await new Promise(r => setTimeout(r, Math.pow(2, i) * 10));
    }
  }
}
```

## Error Messages

### "Key not found"

**Handling:**
```typescript
try {
  const result = await client.get(key);
} catch (err) {
  if (err instanceof KeyNotFoundError) {
    // Use default value or create key
    return defaultValue;
  }
  throw err;
}
```

### "Version mismatch"

**Handling:**
See [Version Conflicts](#version-conflicts) above.

### "Connection error"

**Handling:**
```typescript
async function withRetry<T>(
  operation: () => Promise<T>
): Promise<T> {
  const maxAttempts = 3;
  for (let i = 0; i < maxAttempts; i++) {
    try {
      return await operation();
    } catch (err) {
      if (!(err instanceof ConnectionError) || i === maxAttempts - 1) {
        throw err;
      }
      await new Promise(r => setTimeout(r, Math.pow(2, i) * 100));
    }
  }
  throw new Error('Unreachable');
}
```

## TypeScript-Specific Issues

### Type errors with buffers

**Problem:**
```typescript
const value = Buffer.from('hello');
await client.put(key, value); // Type error
```

**Solution:**
```typescript
import { stringToBytes } from '@norikv/client';

const value = stringToBytes('hello'); // Uint8Array
await client.put(key, value);
```

### Async/await not working

**Problem:**
```typescript
client.put(key, value); // Promise not awaited
console.log('Done'); // Runs before put completes
```

**Solution:**
```typescript
await client.put(key, value); // Await the promise
console.log('Done'); // Runs after put completes
```

### Module resolution errors

**Problem:**
```typescript
import { NoriKVClient } from '@norikv/client';
// Error: Cannot find module
```

**Solution:**

1. Install package:
```bash
npm install @norikv/client
```

2. Check tsconfig.json:
```json
{
  "compilerOptions": {
    "moduleResolution": "node",
    "esModuleInterop": true
  }
}
```

## Browser Issues

### "Buffer is not defined"

**Solution - Add buffer polyfill:**
```bash
npm install buffer
```

```typescript
import { Buffer } from 'buffer';
globalThis.Buffer = Buffer;
```

### gRPC not working in browser

**Solution - Use gRPC-Web:**
```typescript
import { GrpcWebFetchTransport } from '@protobuf-ts/grpcweb-transport';

// Configure client for browser
const transport = new GrpcWebFetchTransport({
  baseUrl: 'http://localhost:8080'
});
```

## Common Pitfalls

### 1. Not awaiting promises

```typescript
//  Bad
client.put(key, value); // Promise ignored

//  Good
await client.put(key, value);
```

### 2. Creating client per request

```typescript
//  Bad
async function handleRequest() {
  const client = new NoriKVClient(config);
  await client.connect();
  await client.put(key, value);
  await client.close(); // Expensive!
}

//  Good
const client = new NoriKVClient(config);
await client.connect();
// Reuse client across requests
```

### 3. Not handling errors

```typescript
//  Bad
const result = await client.get(key); // May throw

//  Good
try {
  const result = await client.get(key);
} catch (err) {
  if (err instanceof KeyNotFoundError) {
    return null;
  }
  throw err;
}
```

### 4. Mixing callbacks and async/await

```typescript
//  Bad
client.put(key, value).then(() => {
  client.get(key).then(result => {
    console.log(result);
  });
});

//  Good
const version = await client.put(key, value);
const result = await client.get(key);
console.log(result);
```

## Debugging Tips

### Enable debug logging

```typescript
// Set environment variable
process.env.DEBUG = 'norikv:*';
```

### Use Node.js debugger

```bash
node --inspect-brk index.js
# Open chrome://inspect
```

### Monitor operations

```typescript
const start = Date.now();
await client.put(key, value);
console.log(`PUT took ${Date.now() - start}ms`);
```

### Check client stats

```typescript
const stats = client.getStats();
console.log('Stats:', JSON.stringify(stats, null, 2));
```

## Getting Help

- **Issues**: [GitHub Issues](https://github.com/jeffhajewski/norikv/issues)
- **Documentation**: [API Guide](API_GUIDE.md), [Architecture Guide](ARCHITECTURE.md)
- **Examples**: [GitHub Examples](https://github.com/jeffhajewski/norikv/tree/main/sdks/typescript/examples)
