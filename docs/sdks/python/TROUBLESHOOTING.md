# NoriKV Python Client Troubleshooting Guide

Solutions to common issues when using the Python client SDK.

## Connection Issues

### "Connection refused" or ConnectionRefusedError

**Symptoms:**
```python
await client.connect()
# ConnectionRefusedError: [Errno 61] Connection refused
```

**Solutions:**

1. Verify server is running:
```bash
netstat -an | grep 9001
lsof -i :9001
```

2. Check client configuration:
```python
config = ClientConfig(
    nodes=["localhost:9001"],  # Verify this address
    total_shards=1024,
)
```

3. Test connectivity:
```bash
telnet localhost 9001
nc -zv localhost 9001
```

### "Deadline exceeded" or timeout errors

**Symptoms:**
```python
result = await client.get(key)
# grpc.aio._call.AioRpcError: Deadline exceeded
```

**Solutions:**

1. Increase timeout:
```python
config = ClientConfig(
    nodes=["localhost:9001"],
    total_shards=1024,
    timeout=10000,  # 10 seconds
)
```

2. Enable retries:
```python
from norikv import RetryConfig

config = ClientConfig(
    nodes=["localhost:9001"],
    total_shards=1024,
    retry=RetryConfig(
        max_attempts=10,
        initial_delay_ms=100,
        max_delay_ms=5000,
    ),
)
```

## Performance Problems

### Slow operations

**Diagnosis:**
```python
import time

start = time.time()
await client.put(key, value)
duration = time.time() - start
print(f"PUT took {duration * 1000:.2f}ms")  # > 100ms consistently
```

**Solutions:**

1. Use appropriate consistency level:
```python
from norikv import GetOptions, ConsistencyLevel

result = await client.get(
    key,
    GetOptions(consistency=ConsistencyLevel.STALE_OK),  # Fastest
)
```

2. Batch operations:
```python
await asyncio.gather(
    *[client.put(k, value) for k in keys]
)
```

3. Check value sizes:
```python
print(f"Value size: {len(value)} bytes")
# Optimal: 100 bytes - 10 KB
```

### High memory usage

**Solutions:**

1. Close client when done:
```python
await client.close()  # Important!
```

2. Clean up topology listeners:
```python
unsubscribe = client.on_topology_change(handler)
unsubscribe()  # Clean up when done
```

3. Use memory profiler:
```bash
pip install memory_profiler
python -m memory_profiler script.py
```

## Version Conflicts

### Frequent VersionMismatchError

**Symptoms:**
```python
await client.put(key, new_value, PutOptions(if_match_version=version))
# VersionMismatchError: Version mismatch
```

**Solution - Implement retry loop:**
```python
from norikv import VersionMismatchError

async def cas_with_retry(
    key: str,
    transform: callable,
    max_retries: int = 10,
) -> None:
    for attempt in range(max_retries):
        try:
            result = await client.get(key)
            current_value = result.value.decode()
            new_value = transform(current_value)

            await client.put(
                key,
                new_value.encode(),
                PutOptions(if_match_version=result.version),
            )
            return  # Success

        except VersionMismatchError:
            if attempt == max_retries - 1:
                raise RuntimeError("CAS failed after retries")

            # Exponential backoff
            await asyncio.sleep((2 ** attempt) * 0.01)
```

## Error Messages

### "Key not found"

**Handling:**
```python
from norikv import KeyNotFoundError

try:
    result = await client.get(key)
except KeyNotFoundError:
    # Use default value or create key
    return default_value
```

### "Version mismatch"

**Handling:**
See [Version Conflicts](#version-conflicts) above.

### "Connection error"

**Handling:**
```python
from norikv import ConnectionError

async def with_retry(operation: callable):
    max_attempts = 3
    for attempt in range(max_attempts):
        try:
            return await operation()
        except ConnectionError:
            if attempt == max_attempts - 1:
                raise
            await asyncio.sleep((2 ** attempt) * 0.1)
```

## Python-Specific Issues

### asyncio.run() issues

**Problem:**
```python
asyncio.run(main())
# RuntimeError: asyncio.run() cannot be called from a running event loop
```

**Solution:**
```python
# In Jupyter notebooks or existing event loop
await main()

# Or use nest_asyncio
import nest_asyncio
nest_asyncio.apply()
asyncio.run(main())
```

### Type errors with mypy

**Problem:**
```python
value = b"hello"
await client.put(key, value)
# mypy error: Argument 2 has incompatible type "bytes"
```

**Solution:**
```python
# Check type hints in library
from norikv import NoriKVClient

# Verify signature accepts bytes
async def put(self, key: str | bytes, value: str | bytes, ...) -> Version
```

### Context manager not cleaning up

**Problem:**
```python
async with NoriKVClient(config) as client:
    await client.put(key, value)
# Client not properly closed
```

**Solution:**
```python
# Ensure __aexit__ is called
try:
    async with NoriKVClient(config) as client:
        await client.put(key, value)
except Exception as err:
    print(f"Error: {err}")
    raise
```

### Module import errors

**Problem:**
```python
from norikv import NoriKVClient
# ModuleNotFoundError: No module named 'norikv'
```

**Solution:**

1. Install package:
```bash
pip install norikv
```

2. Check virtual environment:
```bash
which python
pip list | grep norikv
```

3. Verify PYTHONPATH:
```bash
echo $PYTHONPATH
```

## Asyncio Event Loop Issues

### "Event loop is closed"

**Problem:**
```python
await client.put(key, value)
# RuntimeError: Event loop is closed
```

**Solution:**
```python
# Create new event loop
loop = asyncio.new_event_loop()
asyncio.set_event_loop(loop)

try:
    loop.run_until_complete(main())
finally:
    loop.close()
```

### "Task was destroyed but it is pending"

**Problem:**
```python
# Warning: Task was destroyed but it is pending!
```

**Solution:**
```python
# Properly await all tasks
task = asyncio.create_task(client.put(key, value))
await task

# Or cancel and wait
task.cancel()
try:
    await task
except asyncio.CancelledError:
    pass
```

### "coroutine was never awaited"

**Problem:**
```python
client.put(key, value)  # Not awaited
# RuntimeWarning: coroutine 'NoriKVClient.put' was never awaited
```

**Solution:**
```python
# Always await async functions
await client.put(key, value)
```

## Dependency Issues

### gRPC installation problems

**Problem:**
```bash
pip install grpcio
# ERROR: Failed building wheel for grpcio
```

**Solution:**

1. Install build dependencies:
```bash
# macOS
brew install cmake

# Ubuntu/Debian
sudo apt-get install build-essential python3-dev

# Fedora
sudo dnf install gcc-c++ python3-devel
```

2. Use binary wheels:
```bash
pip install --only-binary :all: grpcio
```

### xxhash installation problems

**Problem:**
```bash
pip install xxhash
# ERROR: Failed building wheel for xxhash
```

**Solution:**

1. Install from binary:
```bash
pip install --only-binary :all: xxhash
```

2. Or install build tools:
```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt-get install python3-dev
```

## Virtual Environment Issues

### Wrong Python version

**Problem:**
```bash
python --version
# Python 3.8.0  (need 3.9+)
```

**Solution:**
```bash
# Install Python 3.9+
pyenv install 3.9.0
pyenv local 3.9.0

# Or use system Python
python3.9 -m venv venv
source venv/bin/activate
```

### Package not found in venv

**Problem:**
```python
import norikv
# ModuleNotFoundError
```

**Solution:**
```bash
# Ensure venv is activated
source venv/bin/activate

# Install in venv
pip install norikv

# Verify installation
pip show norikv
```

## Common Pitfalls

### 1. Not awaiting async functions

```python
#  Bad
client.put(key, value)  # Returns coroutine, not executed

#  Good
await client.put(key, value)
```

### 2. Creating client per request

```python
#  Bad
async def handle_request():
    async with NoriKVClient(config) as client:
        await client.put(key, value)
    # Closes connections!

#  Good
client = NoriKVClient(config)
await client.connect()
# Reuse client across requests
```

### 3. Not handling errors

```python
#  Bad
result = await client.get(key)  # May raise

#  Good
try:
    result = await client.get(key)
except KeyNotFoundError:
    return None
```

### 4. Blocking the event loop

```python
#  Bad
async def handler():
    time.sleep(1)  # Blocks event loop
    await client.put(key, value)

#  Good
async def handler():
    await asyncio.sleep(1)  # Non-blocking
    await client.put(key, value)
```

### 5. Not using context managers

```python
#  Bad
client = NoriKVClient(config)
await client.connect()
await client.put(key, value)
# Forgot to close!

#  Good
async with NoriKVClient(config) as client:
    await client.put(key, value)
```

## Debugging Tips

### Enable debug logging

```python
import logging

logging.basicConfig(level=logging.DEBUG)
logger = logging.getLogger("norikv")
logger.setLevel(logging.DEBUG)
```

### Use asyncio debug mode

```python
import asyncio

asyncio.run(main(), debug=True)
```

### Profile async operations

```python
import time

async def profile_operation(name: str, operation):
    start = time.time()
    result = await operation
    duration = time.time() - start
    print(f"{name} took {duration * 1000:.2f}ms")
    return result

await profile_operation("PUT", client.put(key, value))
```

### Check client stats

```python
stats = client.get_stats()
print(f"Stats: {stats}")
```

### Use Python debugger

```python
import pdb

async def debug_example():
    pdb.set_trace()
    await client.put(key, value)

asyncio.run(debug_example())
```

## Type Checking Issues

### mypy strict mode

**Problem:**
```bash
mypy script.py --strict
# error: Missing type annotation
```

**Solution:**
```python
from norikv import NoriKVClient, ClientConfig, Version

client: NoriKVClient = NoriKVClient(config)
version: Version = await client.put(key, value)
```

### pyright issues

**Problem:**
```bash
pyright script.py
# Type "bytes" cannot be assigned to type "str"
```

**Solution:**
```python
# Use union types
key: str | bytes = b"key"
value: str | bytes = b"value"
await client.put(key, value)
```

## Jupyter Notebook Issues

### Event loop conflicts

**Problem:**
```python
asyncio.run(main())
# RuntimeError: asyncio.run() cannot be called from a running event loop
```

**Solution:**
```python
# Jupyter already has event loop running
await main()

# Or use nest_asyncio
import nest_asyncio
nest_asyncio.apply()
```

### Kernel restart needed

**Problem:**
```python
# Client not responding after changes
```

**Solution:**
```python
# Restart kernel
# Jupyter: Kernel → Restart

# Or recreate client
await client.close()
client = NoriKVClient(config)
await client.connect()
```

## Production Debugging

### Monitor connection pool

```python
stats = client.get_stats()
print(f"Active connections: {stats.pool.active_connections}")
print(f"Total nodes: {stats.router.total_nodes}")
```

### Track operation latency

```python
import time
from collections import defaultdict

latencies = defaultdict(list)

async def tracked_put(key: str, value: str):
    start = time.time()
    try:
        version = await client.put(key, value)
        latencies["put"].append(time.time() - start)
        return version
    except Exception as err:
        print(f"PUT failed: {err}")
        raise

# Analyze latencies
import statistics
print(f"p50: {statistics.median(latencies['put']) * 1000:.2f}ms")
print(f"p95: {statistics.quantiles(latencies['put'], n=20)[18] * 1000:.2f}ms")
```

### Log errors with context

```python
import logging
import traceback

logger = logging.getLogger(__name__)

try:
    await client.put(key, value)
except Exception as err:
    logger.error(
        "PUT failed",
        extra={
            "key": key,
            "error": str(err),
            "traceback": traceback.format_exc(),
        },
    )
    raise
```

## Getting Help

- **Issues**: [GitHub Issues](https://github.com/jeffhajewski/norikv/issues)
- **Documentation**: [API Guide](API_GUIDE.md), [Architecture Guide](ARCHITECTURE.md)
- **Examples**: [GitHub Examples](https://github.com/jeffhajewski/norikv/tree/main/sdks/python/examples)
