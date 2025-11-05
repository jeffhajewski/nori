# NoriKV TypeScript Client Documentation

Complete documentation for the NoriKV TypeScript/JavaScript client SDK.

## Documentation Guides

### [API Guide](API_GUIDE.md)
Complete API reference with TypeScript examples covering:
- Installation and setup (npm/yarn/pnpm)
- Client configuration and connection management
- Core operations (put, get, delete) with type safety
- Advanced features (CAS, idempotency, TTL, consistency levels)
- Error handling with TypeScript types
- Best practices for async/await patterns
- Complete working examples

### [Architecture Guide](ARCHITECTURE.md)
Internal design and implementation details:
- Component architecture and request flow
- Async/Promise-based concurrency model
- Connection pooling with gRPC channels
- Routing and sharding algorithms (XXHash64 + Jump Consistent Hash)
- Retry logic with exponential backoff
- Error handling and type safety
- Performance considerations and optimizations

### [Troubleshooting Guide](TROUBLESHOOTING.md)
Solutions to common issues:
- Connection and network problems
- Performance optimization tips
- Version conflicts and CAS failures
- All error messages with solutions
- Configuration issues
- Debugging with TypeScript
- Common pitfalls and how to avoid them

### [Advanced Patterns](ADVANCED_PATTERNS.md)
Real-world usage patterns with complete TypeScript implementations:
- Distributed Counter (with sharding)
- Session Management (with TTL)
- Inventory Management (preventing overselling)
- Caching Layer (write-through cache)
- Rate Limiting (sliding window)
- Leader Election (distributed coordination)
- Event Sourcing (event store)
- Multi-Tenancy (data isolation)

## Quick Links

- [TypeScript SDK README](../README.md) - Project overview and quick start
- [Examples](../examples/) - Working code samples
- [Tests](../tests/) - Integration test examples

## Language-Specific Features

The TypeScript SDK provides:
- **Full TypeScript types** for type-safe development
- **Async/await** for modern JavaScript patterns
- **Promise-based API** for easy composition
- **Dual package** support (ESM + CommonJS)
- **Browser compatibility** (with appropriate polyfills)
- **Node.js optimized** with native gRPC bindings

## Getting Started

1. Start with the [API Guide](API_GUIDE.md) for basic usage
2. Review [Architecture Guide](ARCHITECTURE.md) to understand internals
3. Check [Troubleshooting Guide](TROUBLESHOOTING.md) if you encounter issues
4. Explore [Advanced Patterns](ADVANCED_PATTERNS.md) for complex use cases

## Support

- **Issues**: [GitHub Issues](https://github.com/norikv/norikv/issues)
- **Discussions**: [GitHub Discussions](https://github.com/norikv/norikv/discussions)
- **Documentation**: This guide and inline JSDoc comments
