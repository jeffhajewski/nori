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

from norikv.client import NoriKVClient
from norikv.errors import (
    AlreadyExistsError,
    ConnectionError,
    DeadlineExceededError,
    InvalidArgumentError,
    NoNodesAvailableError,
    NoriKVError,
    NotLeaderError,
    RetryExhaustedError,
    UnavailableError,
    VersionMismatchError,
)
from norikv.types import (
    ClientConfig,
    DeleteOptions,
    GetOptions,
    GetResult,
    PutOptions,
    RetryConfig,
    Version,
)

__all__ = [
    "__version__",
    # Client
    "NoriKVClient",
    # Configuration
    "ClientConfig",
    "RetryConfig",
    # Options
    "GetOptions",
    "PutOptions",
    "DeleteOptions",
    # Results and types
    "GetResult",
    "Version",
    # Errors
    "NoriKVError",
    "NotLeaderError",
    "AlreadyExistsError",
    "VersionMismatchError",
    "UnavailableError",
    "DeadlineExceededError",
    "InvalidArgumentError",
    "ConnectionError",
    "NoNodesAvailableError",
    "RetryExhaustedError",
]
