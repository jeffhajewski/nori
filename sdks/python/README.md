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
