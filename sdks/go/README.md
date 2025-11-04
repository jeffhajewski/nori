# NoriKV Go Client

Go client SDK for **NoriKV** - a sharded, Raft-replicated, log-structured key-value store with first-class observability.

## Status

**Core Implementation Complete** - Ready for integration testing:

- ✅ Hash functions (xxhash64 + Jump Consistent Hash) with cross-SDK compatibility
- ✅ Protocol buffer definitions and gRPC stubs
- ✅ Type system (Version, Options, Results, ClusterView)
- ✅ Comprehensive error handling
- ✅ Retry policy with exponential backoff
- ✅ Connection pooling with health checking
- ✅ Router with single-flight pattern and leader caching
- ✅ Topology manager with cluster watching
- ✅ Client API implementation (Put, Get, Delete, Close)
- ✅ Comprehensive unit tests (all passing)
- 🚧 Ephemeral server support (pending)
- 🚧 Integration tests with live server (pending)

## Features

### Implemented
- ✅ **Smart Client**: Client-side routing with hash-based shard assignment (Jump Consistent Hash + xxhash64)
- ✅ **Leader-Aware Routing**: Direct requests to shard leaders, handle NOT_LEADER redirects automatically
- ✅ **Retry Logic**: Exponential backoff with jitter for transient failures
- ✅ **Idempotency**: Safe retries with idempotency keys
- ✅ **Conditional Operations**: Compare-and-swap (CAS) with version matching
- ✅ **Consistency Levels**: Lease-based (fast), linearizable (strict), or stale reads
- ✅ **Connection Pooling**: Efficient gRPC connection management per node with health checking
- ✅ **Cluster Topology Tracking**: Watch and react to cluster membership changes
- ✅ **Zero-Allocation Routing**: Optimized hot path with no heap allocations
- ✅ **Single-Flight Pattern**: Deduplicate concurrent leader discovery for same shard

### Planned
- 🚧 **If-Not-Exists semantics**: Pending proto field addition
- 🚧 **Ephemeral server**: In-process server for testing
- 🚧 **Integration tests**: End-to-end tests with live server

## Installation

```bash
go get github.com/norikv/norikv-go
```

## Hash Function Compatibility

**CRITICAL**: The hash functions (xxhash64 + jumpConsistentHash) **MUST** produce identical results to the server's Rust implementation and other SDKs. Any deviation will cause routing failures and data loss.

### Validation

All hash functions are validated against test vectors from Python and TypeScript SDKs:

```bash
go test -v ./hash/...
```

Performance benchmarks:

```bash
go test -bench=. -benchmem ./hash/...
```

Results:
- XXHash64: ~2.5ns per operation (0 allocs)
- JumpConsistentHash: ~14ns per operation (0 allocs)
- GetShardForKey: ~23ns per operation (0 allocs)

## Development

### Building

```bash
# Build all packages
go build ./...

# Run tests
go test ./...

# Run benchmarks
go test -bench=. -benchmem ./...
```

### Generating Protobuf Code

```bash
# Add to PATH if needed
export PATH=$PATH:$HOME/go/bin

# Generate
cd proto
protoc --go_out=. --go_opt=paths=source_relative \
       --go-grpc_out=. --go-grpc_opt=paths=source_relative \
       norikv.proto
```

Or use go generate:

```bash
go generate ./proto/...
```

## License

MIT OR Apache-2.0

## Related

- Python SDK: `../python/`
- TypeScript SDK: `../typescript/`
- Java SDK: `../java/`
