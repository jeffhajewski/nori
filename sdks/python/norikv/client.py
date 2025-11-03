"""NoriKV Python client implementation."""

import asyncio
from typing import Optional

from norikv._internal.connection import ConnectionPool
from norikv._internal.topology import TopologyManager
from norikv._internal.router import Router
from norikv._internal.retry import RetryPolicy
from norikv.errors import InvalidArgumentError, from_grpc_error
from norikv.hash import get_shard_for_key, key_to_bytes, value_to_bytes
from norikv.proto import (
    DeleteRequest,
    GetRequest,
    PutRequest,
)
from norikv.types import (
    ClientConfig,
    DeleteOptions,
    GetOptions,
    GetResult,
    PutOptions,
    Version,
)


class NoriKVClient:
    """Async NoriKV client for Python.

    Example:
        >>> async with NoriKVClient(ClientConfig(nodes=["localhost:50051"])) as client:
        ...     version = await client.put("my-key", "my-value")
        ...     result = await client.get("my-key")
        ...     print(result.value)
    """

    def __init__(self, config: ClientConfig):
        """Initialize the NoriKV client.

        Args:
            config: Client configuration
        """
        if not config.nodes:
            raise InvalidArgumentError("At least one node address is required")

        self._config = config
        self._pool = ConnectionPool(
            max_connections_per_node=config.max_connections_per_node,
            connect_timeout=config.timeout,
        )
        self._topology = TopologyManager(
            initial_nodes=config.nodes,
            total_shards=config.total_shards,
        )
        self._router = Router(
            pool=self._pool,
            topology=self._topology,
        )
        self._retry_policy = RetryPolicy(config.retry)
        self._connected = False

    async def connect(self) -> None:
        """Connect to the cluster.

        This is called automatically when using the client as a context manager.
        """
        if self._connected:
            return

        # Start topology manager
        if self._config.watch_cluster:
            await self._topology.start()

        self._connected = True

    async def close(self) -> None:
        """Close all connections to the cluster."""
        if not self._connected:
            return

        # Stop topology manager
        await self._topology.stop()

        # Close connection pool
        await self._pool.close()
        self._connected = False

    async def __aenter__(self) -> "NoriKVClient":
        """Async context manager entry."""
        await self.connect()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb) -> None:  # type: ignore
        """Async context manager exit."""
        await self.close()

    def is_connected(self) -> bool:
        """Check if the client is connected."""
        return self._connected and not self._pool.is_closed()

    async def put(
        self,
        key: str | bytes,
        value: str | bytes | bytearray,
        options: Optional[PutOptions] = None,
    ) -> Version:
        """Store a key-value pair.

        Args:
            key: The key to store
            value: The value to store
            options: Optional put options (TTL, conditional writes, etc.)

        Returns:
            Version of the written value

        Raises:
            InvalidArgumentError: If key or value is invalid
            AlreadyExistsError: If key exists and if_not_exists=True
            VersionMismatchError: If version doesn't match if_match_version
            UnavailableError: If the service is unavailable
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        opts = options or PutOptions()
        key_bytes = key_to_bytes(key)
        value_bytes = value_to_bytes(value)

        # Create protobuf request
        request = PutRequest(
            key=key_bytes,
            value=value_bytes,
            ttl_ms=opts.ttl_ms or 0,
            idempotency_key=opts.idempotency_key or "",
        )

        if opts.if_match_version:
            request.if_match.term = opts.if_match_version.term
            request.if_match.index = opts.if_match_version.index

        # Execute with retry and routing
        async def operation():
            return await self._router.route_kv_request(
                key=key_bytes,
                request_fn=lambda stub: stub.Put(
                    request, timeout=self._config.timeout / 1000.0
                ),
                prefer_leader=True,
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="Put"
        )
        return Version(term=response.version.term, index=response.version.index)

    async def get(
        self, key: str | bytes, options: Optional[GetOptions] = None
    ) -> GetResult:
        """Retrieve a value by key.

        Args:
            key: The key to retrieve
            options: Optional get options (consistency level)

        Returns:
            GetResult with value and version, or None if key doesn't exist

        Raises:
            InvalidArgumentError: If key is invalid
            UnavailableError: If the service is unavailable
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        opts = options or GetOptions()
        key_bytes = key_to_bytes(key)

        # Create protobuf request
        request = GetRequest(key=key_bytes, consistency=opts.consistency)

        # Execute with retry and routing
        # For reads, we can use replicas if consistency allows
        prefer_leader = opts.consistency != "stale_ok"

        async def operation():
            return await self._router.route_kv_request(
                key=key_bytes,
                request_fn=lambda stub: stub.Get(
                    request, timeout=self._config.timeout / 1000.0
                ),
                prefer_leader=prefer_leader,
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="Get"
        )

        if not response.value:
            return GetResult(value=None, version=None)

        return GetResult(
            value=response.value,
            version=Version(
                term=response.version.term, index=response.version.index
            ),
            metadata=dict(response.meta) if response.meta else None,
        )

    async def delete(
        self, key: str | bytes, options: Optional[DeleteOptions] = None
    ) -> bool:
        """Delete a key-value pair.

        Args:
            key: The key to delete
            options: Optional delete options (conditional delete)

        Returns:
            True if the key was deleted, False if it didn't exist

        Raises:
            InvalidArgumentError: If key is invalid
            VersionMismatchError: If version doesn't match if_match_version
            UnavailableError: If the service is unavailable
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        opts = options or DeleteOptions()
        key_bytes = key_to_bytes(key)

        # Create protobuf request
        request = DeleteRequest(
            key=key_bytes, idempotency_key=opts.idempotency_key or ""
        )

        if opts.if_match_version:
            request.if_match.term = opts.if_match_version.term
            request.if_match.index = opts.if_match_version.index

        # Execute with retry and routing
        async def operation():
            return await self._router.route_kv_request(
                key=key_bytes,
                request_fn=lambda stub: stub.Delete(
                    request, timeout=self._config.timeout / 1000.0
                ),
                prefer_leader=True,
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="Delete"
        )
        return response.tombstoned

    def get_shard_for_key(self, key: str | bytes) -> int:
        """Calculate which shard a key belongs to.

        Args:
            key: The key to calculate shard for

        Returns:
            Shard ID (0 to total_shards-1)
        """
        return get_shard_for_key(key, self._config.total_shards)
