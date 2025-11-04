# NoriKV Java Client

Java client SDK for **NoriKV** - a sharded, Raft-replicated, log-structured key-value store with first-class observability.

## Status

**CORE IMPLEMENTATION COMPLETE** - Ready for protobuf generation and integration testing

### Completed
- Maven project structure with pom.xml
- Dependencies configured (gRPC 1.59.0, Protobuf 3.25.0, JUnit 5)
- Shared proto file at repository root
- Build configuration
- Implementation plan documented
- Hash functions (XXHash64 + Jump Consistent Hash) with comprehensive tests (25+ tests)
- Core types (Version, Options, Results, Config, ClusterView, ShardInfo, etc.)
- Exception hierarchy (6 exception types: NotLeaderException, AlreadyExistsException, etc.)
- Retry policy with exponential backoff (13 tests, 287 lines)
- Connection pool with gRPC channel management (180 lines)
- Router with leader caching and shard assignment (22 tests, 380 lines)
- Topology manager with cluster watching (27 tests, 450+ lines)
- Client API (put, get, delete, close) with AutoCloseable support (18 tests)
- Comprehensive unit tests for all components (100+ tests total)

### Pending
- Protocol buffer code generation (run `mvn protobuf:compile protobuf:compile-custom`)
- Wire up gRPC calls in NoriKVClient (currently placeholders)
- Ephemeral server for testing
- Integration tests with ephemeral server
- Working examples

## Features (Planned)

Following the same feature set as Go/TypeScript/Python SDKs:

- **Smart Client**: Client-side routing with hash-based shard assignment
- **Leader-Aware Routing**: Direct requests to shard leaders with automatic failover
- **Retry Logic**: Exponential backoff with jitter
- **Idempotency**: Safe retries with idempotency keys
- **Conditional Operations**: Compare-and-swap (CAS) with version matching
- **Consistency Levels**: Lease-based, linearizable, or stale reads
- **Connection Pooling**: Efficient gRPC channel management
- **Cluster Topology Tracking**: Watch and react to membership changes
- **Ephemeral Server**: In-memory server for testing

## Prerequisites

- **Java**: 11 or higher
- **Maven**: 3.6 or higher

Check versions:
```bash
java -version
mvn -version
```

## Installation (Coming Soon)

Maven:
```xml
<dependency>
    <groupId>com.norikv</groupId>
    <artifactId>norikv-client</artifactId>
    <version>0.1.0</version>
</dependency>
```

Gradle:
```gradle
implementation 'com.norikv:norikv-client:0.1.0'
```

## Quick Start (Preview)

```java
import com.norikv.client.NoriKVClient;
import com.norikv.client.NoriKVClientConfig;
import com.norikv.client.types.*;

import java.util.Arrays;

public class Example {
    public static void main(String[] args) throws Exception {
        // Configure client
        NoriKVClientConfig config = NoriKVClientConfig.builder()
            .nodes(Arrays.asList("localhost:9001", "localhost:9002"))
            .timeoutMs(5000)
            .totalShards(1024)
            .build();

        // Create client (implements AutoCloseable)
        try (NoriKVClient client = new NoriKVClient(config)) {
            // Put a value
            byte[] key = "user:123".getBytes();
            byte[] value = "{\"name\":\"Alice\"}".getBytes();

            Version version = client.put(key, value, null);
            System.out.println("Wrote version: " + version);

            // Get the value
            GetResult result = client.get(key, null);
            System.out.println("Read value: " + new String(result.getValue()));

            // Conditional update (CAS)
            byte[] newValue = "{\"name\":\"Alice\",\"age\":30}".getBytes();
            PutOptions options = PutOptions.builder()
                .ifMatchVersion(version)
                .build();

            client.put(key, newValue, options);

            // Delete
            client.delete(key, null);
        }
    }
}
```

## Hash Function Compatibility

**CRITICAL**: Hash functions must produce identical results to other SDKs.

Test vectors (from cross-SDK validation):
```java
// xxhash64("hello") = 2794345569481354659
// xxhash64("world") = 16679358290033791471
// getShardForKey("hello", 1024) = 309
// getShardForKey("world", 1024) = 752
```

## Development

### Building from Source

```bash
# Clone the repository
git clone https://github.com/norikv/norikv.git
cd norikv/sdks/java

# Generate protobuf stubs
mvn protobuf:compile protobuf:compile-custom

# Compile
mvn compile

# Run tests
mvn test

# Package
mvn package

# Install locally
mvn install
```

### Project Structure

```
src/
├── main/
│   ├── java/
│   │   └── com/norikv/client/
│   │       ├── NoriKVClient.java          # Main client API
│   │       ├── hash/                       # Hash functions
│   │       ├── internal/                   # Internal components
│   │       │   ├── conn/                   # Connection pool
│   │       │   ├── retry/                  # Retry policy
│   │       │   ├── router/                 # Routing logic
│   │       │   └── topology/               # Cluster topology
│   │       └── types/                      # Types and errors
│   ├── proto/
│   │   └── norikv.proto                    # Protocol definitions
│   └── resources/
└── test/
    ├── java/
    │   └── com/norikv/client/
    │       ├── hash/                       # Hash tests
    │       ├── internal/                   # Internal tests
    │       └── integration/                # Integration tests
    └── resources/
```

### Running Tests

```bash
# All tests
mvn test

# Specific test class
mvn test -Dtest=HashFunctionsTest

# With coverage
mvn test jacoco:report
```

### Generating Documentation

```bash
mvn javadoc:javadoc
open target/site/apidocs/index.html
```

## Implementation Progress

See [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for detailed progress tracking.

Current phase: **Project Setup & Hash Functions**

## Design Principles

### Thread Safety
All client operations are thread-safe. A single client instance can be safely shared across multiple threads.

### Resource Management
The client implements `AutoCloseable` for proper resource cleanup:
```java
try (NoriKVClient client = new NoriKVClient(config)) {
    // Use client
} // Automatically closed
```

### Error Handling
Checked exceptions for recoverable errors:
- `KeyNotFoundException`: Key does not exist
- `VersionMismatchException`: Conditional operation failed
- `ConnectionException`: Network or cluster issues

Runtime exceptions for programming errors.

## Contributing

See the main [CONTRIBUTING.md](../../CONTRIBUTING.md) for guidelines.

### Areas for Contribution
- Complete hash function implementation
- Implement client components (pool, retry, router)
- Write comprehensive tests
- Add examples and documentation
- Performance testing and optimization

## License

MIT OR Apache-2.0

## Related

- Go SDK: `../go/`
- TypeScript SDK: `../typescript/`
- Python SDK: `../python/`
- Main Project: `../../`

## Acknowledgments

Follows the design patterns established in the Go, TypeScript, and Python SDKs.

---

**Status**: Active development | **Target Version**: 0.1.0 | **Java**: 11+ | **Build Tool**: Maven
