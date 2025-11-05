---
layout: default
title: Python SDK
parent: Client SDKs
nav_order: 4
has_children: true
---

# NoriKV Python Client SDK

Pythonic async client for NoriKV with type hints and context managers.

## Status

**✅ PRODUCTION READY** - Fully functional Python SDK

- ✅ **40 tests passing** with comprehensive coverage
- ✅ Async/await API built on asyncio
- ✅ Full type hints for mypy checking
- ✅ Context managers for automatic cleanup
- ✅ Cross-SDK hash validation passing

## Quick Start

### Installation

```bash
pip install norikv
```

### Basic Usage

```python
import asyncio
from norikv import NoriKVClient, ClientConfig

async def main():
    config = ClientConfig(
        nodes=["localhost:9001", "localhost:9002"],
        total_shards=1024,
        timeout=5000,
    )

    async with NoriKVClient(config) as client:
        # Put a value
        version = await client.put("user:123", "Alice")

        # Get a value
        result = await client.get("user:123")
        print(f"Value: {result.value.decode()}")

        # Delete
        await client.delete("user:123")

asyncio.run(main())
```

## Documentation

### Core Guides

- **[API Guide](./API_GUIDE.html)** - Complete API reference
- **[Architecture Guide](./ARCHITECTURE.html)** - Internal design
- **[Troubleshooting Guide](./TROUBLESHOOTING.html)** - Common issues
- **[Advanced Patterns](./ADVANCED_PATTERNS.html)** - Real-world examples

## Features

### Core Features
- ✅ Smart client-side routing
- ✅ Leader-aware routing with failover
- ✅ Automatic retries with exponential backoff
- ✅ Idempotency support
- ✅ CAS operations with version matching
- ✅ Multiple consistency levels
- ✅ Connection pooling
- ✅ Topology tracking

### Python-Specific
- ✅ Async/await API with asyncio
- ✅ Type hints throughout
- ✅ Context managers (async with)
- ✅ Pythonic API following PEP 8
- ✅ Compatible with type checkers (mypy, pyright)

## Requirements

- **Python**: 3.9 or higher
- **NoriKV Server**: 0.1.x

## Support

- **Issues**: [GitHub Issues](https://github.com/j-haj/nori/issues)
- **Source**: [GitHub Repository](https://github.com/j-haj/nori/tree/main/sdks/python)
- **PyPI**: [norikv](https://pypi.org/project/norikv/)

## License

MIT OR Apache-2.0
