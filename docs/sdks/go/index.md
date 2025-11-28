# NoriKV Go Client SDK

High-performance Go client for NoriKV with zero-allocation routing and comprehensive documentation.

## Status

** PRODUCTION READY** - Fully functional Go SDK

-  **102+ tests passing** with full coverage
-  Zero-allocation routing hot path (23ns/op, 0 allocs)
-  Complete API with all operations
-  Comprehensive documentation (4 detailed guides)
-  Cross-SDK hash validation passing

## Quick Start

### Installation

```bash
go get github.com/norikv/norikv-go
```

### Basic Usage

```go
package main

import (
    "context"
    "fmt"
    "log"

    norikv "github.com/norikv/norikv-go"
)

func main() {
    ctx := context.Background()
    config := norikv.DefaultClientConfig([]string{"localhost:9001"})

    client, err := norikv.NewClient(ctx, config)
    if err != nil {
        log.Fatal(err)
    }
    defer client.Close()

    // Put a value
    key := []byte("user:123")
    value := []byte(`{"name":"Alice"}`)
    version, _ := client.Put(ctx, key, value, nil)

    // Get the value
    result, _ := client.Get(ctx, key, nil)
    fmt.Printf("Value: %s\n", result.Value)

    // Delete
    client.Delete(ctx, key, nil)
}
```

## Documentation

The Go SDK includes comprehensive documentation:

### Core Guides

- **[API Guide](./API_GUIDE.md)** - Complete API reference
  - Installation and setup
  - Client configuration with context
  - All operations with Go idioms
  - Advanced features
  - Error handling with errors.Is
  - Best practices

- **[Architecture Guide](./ARCHITECTURE.md)** - Internal design
  - Component architecture
  - Zero-allocation routing
  - Goroutine safety
  - Single-flight pattern
  - Connection management
  - Performance optimizations

- **[Troubleshooting Guide](./TROUBLESHOOTING.md)** - Common issues
  - Connection problems
  - Performance with pprof
  - Version conflicts
  - Error solutions
  - Debugging goroutines

- **[Advanced Patterns](./ADVANCED_PATTERNS.md)** - Production patterns
  - Distributed Counter
  - Session Management
  - Inventory Management
  - Caching Layer
  - Rate Limiting
  - Leader Election
  - Event Sourcing
  - Multi-Tenancy

## Features

### Core Features
-  Smart client-side routing
-  Leader-aware routing with failover
-  Retry logic with exponential backoff
-  Idempotency support
-  Conditional operations (CAS)
-  Multiple consistency levels
-  Connection pooling
-  Topology tracking
-  Goroutine-safe operations

### Go-Specific
-  Context-aware operations
-  Zero-allocation routing (0 B/op, 0 allocs/op)
-  Single-flight pattern for leader discovery
-  defer-based resource management
-  Native error wrapping
-  Efficient concurrency with goroutines

## Performance

**Hash Function Benchmarks:**
- XXHash64: ~2.5ns per operation (0 allocs)
- JumpConsistentHash: ~14ns per operation (0 allocs)
- GetShardForKey: ~23ns per operation (0 allocs)

**Zero Allocations** in routing hot path ensures:
- No garbage collection pressure
- Consistent low latency
- High throughput under load

## Requirements

- **Go**: 1.19 or higher
- **NoriKV Server**: 0.1.x

## Examples

See the [source repository](https://github.com/jeffhajewski/norikv/tree/main/sdks/go/examples) for complete working examples:

- `basic/` - Complete basic usage patterns
- `ephemeral/` - Testing with in-memory server

## Support

- **Issues**: [GitHub Issues](https://github.com/jeffhajewski/norikv/issues)
- **Source**: [GitHub Repository](https://github.com/jeffhajewski/norikv/tree/main/sdks/go)
- **pkg.go.dev**: [API Documentation](https://pkg.go.dev/github.com/norikv/norikv-go)

## License

MIT OR Apache-2.0

---

{: .note }
> This SDK is optimized for high performance with zero-allocation routing and native Go concurrency patterns.
