---
layout: default
title: TypeScript SDK
parent: Client SDKs
nav_order: 3
has_children: true
---

# NoriKV TypeScript Client SDK

Type-safe TypeScript/JavaScript client for NoriKV with full async/await support.

## Status

** PRODUCTION READY** - Fully functional TypeScript SDK

-  **100+ tests passing** with comprehensive coverage
-  Full TypeScript types for compile-time safety
-  Async/await Promise-based API
-  Dual package support (ESM + CommonJS)
-  Browser compatible with polyfills

## Quick Start

### Installation

```bash
npm install @norikv/client
# or
yarn add @norikv/client
# or
pnpm add @norikv/client
```

### Basic Usage

```typescript
import { NoriKVClient, bytesToString } from '@norikv/client';

const client = new NoriKVClient({
  nodes: ['localhost:9001', 'localhost:9002'],
  totalShards: 1024,
  timeout: 5000,
});

await client.connect();

// Put a value
await client.put('user:123', 'Alice');

// Get a value
const result = await client.get('user:123');
console.log(bytesToString(result.value)); // 'Alice'

// Delete
await client.delete('user:123');

await client.close();
```

## Documentation

### Core Guides

- **[API Guide](./API_GUIDE.html)** - Complete API reference
- **[Architecture Guide](./ARCHITECTURE.html)** - Internal design
- **[Troubleshooting Guide](./TROUBLESHOOTING.html)** - Common issues
- **[Advanced Patterns](./ADVANCED_PATTERNS.html)** - Real-world examples

## Features

### Core Features
-  Smart client-side routing
-  Leader-aware routing with failover
-  Automatic retries with exponential backoff
-  Idempotency support
-  CAS operations with version matching
-  Multiple consistency levels
-  Connection pooling
-  Topology tracking

### TypeScript-Specific
-  Full TypeScript types
-  Async/await Promise API
-  ESM + CommonJS dual package
-  Browser compatible
-  JSDoc inline documentation
-  Tree-shakeable exports

## Requirements

- **Node.js**: 16 or higher
- **TypeScript**: 4.5 or higher (if using TypeScript)
- **NoriKV Server**: 0.1.x

## Support

- **Issues**: [GitHub Issues](https://github.com/j-haj/nori/issues)
- **Source**: [GitHub Repository](https://github.com/j-haj/nori/tree/main/sdks/typescript)
- **npm**: [@norikv/client](https://www.npmjs.com/package/@norikv/client)

## License

MIT OR Apache-2.0
