# NoriKV Python SDK

Python client library for [NoriKV](https://github.com/norikv/norikv) - a sharded, Raft-replicated, log-structured key-value store.

## Features

- **Async/await API** - Built on Python's asyncio for high-performance concurrent operations
- **Automatic retry logic** - Exponential backoff with configurable policies
- **Smart routing** - Automatic routing to shard leaders with failover support
- **Connection pooling** - Efficient gRPC connection management
- **Type hints** - Full type annotations for IDE support and type checking
- **Cross-SDK compatibility** - Hash parity with TypeScript, Go, and Java SDKs

## Installation

```bash
pip install norikv
```

## Quick Start

### Ephemeral Mode (Easiest for Testing)

The fastest way to get started is with an ephemeral (in-memory) NoriKV instance:

```python
import asyncio
from norikv import create_ephemeral

async def main():
    # Create an ephemeral server (auto-starts and auto-cleans up)
    async with create_ephemeral() as cluster:
        client = cluster.get_client()

        async with client:
            # Put a value
            version = await client.put("user:123", "Alice")
            print(f"Stored with version: {version}")

            # Get a value
            result = await client.get("user:123")
            print(f"Retrieved: {result.value.decode()}")

            # Delete a value
            deleted = await client.delete("user:123")
            print(f"Deleted: {deleted}")

asyncio.run(main())
```

**Requirements**: The `norikv-server` binary must be available in your PATH. You can:
- Build it: `cargo build --release -p norikv-server`
- Or set `NORIKV_SERVER_PATH` environment variable to point to the binary

### Connecting to an Existing Cluster

```python
import asyncio
from norikv import NoriKVClient, ClientConfig

async def main():
    # Configure the client
    config = ClientConfig(
        nodes=["localhost:7447", "localhost:7448", "localhost:7449"],
        total_shards=1024,           # Number of virtual shards
        timeout=5000,                # Request timeout in milliseconds
    )

    # Use async context manager for automatic connection management
    async with NoriKVClient(config) as client:
        # Put a value
        version = await client.put("user:123", "Alice")
        print(f"Stored with version: {version}")

        # Get a value
        result = await client.get("user:123")
        if result.value:
            print(f"Retrieved: {result.value.decode('utf-8')}")

        # Delete a value
        deleted = await client.delete("user:123")
        print(f"Deleted: {deleted}")

asyncio.run(main())
```

See [`examples/basic_usage.py`](examples/basic_usage.py) for a complete example.

## Documentation

Comprehensive guides are available for the NoriKV SDKs. While Python-specific detailed guides are being developed, you can reference the comprehensive documentation from the Java and Go SDKs, which cover the same concepts and can be easily adapted to Python's async/await patterns:

### Core Documentation
- **[Java SDK Documentation](../java/docs/)** - Extremely comprehensive guides covering all patterns
  - [API Guide](../java/docs/API_GUIDE.md) - Adapt Java examples to Python async/await
  - [Architecture Guide](../java/docs/ARCHITECTURE.md) - Component design applies to all SDKs
  - [Troubleshooting Guide](../java/docs/TROUBLESHOOTING.md) - Common issues and solutions
  - [Advanced Patterns](../java/docs/ADVANCED_PATTERNS.md) - 8 real-world patterns with full implementations

- **[Go SDK Documentation](../go/docs/)** - Comprehensive guides with concurrency patterns
  - [API Guide](../go/docs/API_GUIDE.md) - Similar patterns to Python asyncio
  - [Architecture Guide](../go/docs/ARCHITECTURE.md) - Internal design and performance
  - [Troubleshooting Guide](../go/docs/TROUBLESHOOTING.md) - Debugging and solutions
  - [Advanced Patterns](../go/docs/ADVANCED_PATTERNS.md) - Production-ready patterns

### Python-Specific Features
The Python SDK provides:
- **Async/await API** built on asyncio for high performance
- **Type hints** throughout for IDE support and mypy checking
- **Context managers** for automatic resource cleanup
- **Pythonic API** following PEP 8 and best practices
- **Both sync and async** support (async recommended)

### Quick Reference
```python
import asyncio
from norikv import NoriKVClient, ClientConfig, PutOptions, GetOptions, ConsistencyLevel

async def examples():
    config = ClientConfig(nodes=["localhost:9001"])

    async with NoriKVClient(config) as client:
        # CAS (Compare-And-Swap) - see Java/Go docs for detailed patterns
        result = await client.get("counter")
        await client.put(
            "counter",
            new_value,
            PutOptions(if_match_version=result.version)  # Atomic update
        )

        # TTL expiration
        await client.put(
            "session",
            session_data,
            PutOptions(ttl_ms=3600000)  # 1 hour
        )

        # Consistency levels
        result = await client.get(
            "key",
            GetOptions(consistency=ConsistencyLevel.LINEARIZABLE)
        )

asyncio.run(examples())
```

All advanced patterns (distributed counters, session management, inventory control, caching, rate limiting, leader election, event sourcing, multi-tenancy) from the Java/Go documentation can be directly adapted to Python using async/await and context managers.

## Development

### Setup

```bash
# Create virtual environment
uv venv
source .venv/bin/activate  # On Windows: .venv\Scripts\activate

# Install dependencies
uv pip install --python .venv/bin/python -e .
```

### Running Tests

```bash
# Run all tests
.venv/bin/python -m pytest

# Run unit tests only
.venv/bin/python -m pytest tests/unit

# Run integration tests
.venv/bin/python -m pytest tests/integration
```

### Code Quality

```bash
# Format code
make format

# Run linters
make lint
```

## Test Results

- ✓ 35/35 hash tests passing (including cross-SDK validation)
- ✓ 5/5 integration tests passing
- ✓ Hash parity verified with TypeScript SDK

## Compatibility

- **Python**: 3.9+
- **NoriKV Server**: 0.1.x
- **Protocol**: gRPC with Protobuf

## License

MIT
