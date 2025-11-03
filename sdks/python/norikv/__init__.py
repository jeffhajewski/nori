"""NoriKV Python Client SDK.

A Python client for NoriKV - a sharded, Raft-replicated key-value store.

Example:
    >>> import asyncio
    >>> from norikv import NoriKVClient, ClientConfig
    >>>
    >>> async def main():
    ...     config = ClientConfig(nodes=["localhost:50051"])
    ...     async with NoriKVClient(config) as client:
    ...         # Put a value
    ...         version = await client.put("my-key", "my-value")
    ...
    ...         # Get a value
    ...         result = await client.get("my-key")
    ...         print(result.value)  # b"my-value"
    ...
    ...         # Delete a value
    ...         await client.delete("my-key")
    >>>
    >>> asyncio.run(main())
"""

__version__ = "0.1.0"

# Main client will be imported once implemented
# from norikv.client import NoriKVClient
# from norikv.types import ClientConfig, GetOptions, PutOptions, DeleteOptions
# from norikv.errors import NoriKVError, NotLeaderError, NetworkError

__all__ = [
    "__version__",
    # Will be uncommented as modules are implemented:
    # "NoriKVClient",
    # "ClientConfig",
    # "GetOptions",
    # "PutOptions",
    # "DeleteOptions",
    # "NoriKVError",
    # "NotLeaderError",
    # "NetworkError",
]
