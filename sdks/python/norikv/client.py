"""NoriKV Python client implementation."""

import asyncio
from typing import List, Optional

from norikv._internal.connection import ConnectionPool
from norikv._internal.topology import TopologyManager
from norikv._internal.router import Router
from norikv._internal.retry import RetryPolicy
from norikv.errors import InvalidArgumentError, NoNodesAvailableError, from_grpc_error
from norikv.hash import get_shard_for_key, key_to_bytes, value_to_bytes
from norikv.proto import (
    DeleteRequest,
    GetRequest,
    PutRequest,
    # Vector types
    CreateVectorIndexRequest,
    DropVectorIndexRequest,
    VectorInsertRequest,
    VectorDeleteRequest,
    VectorSearchRequest,
    VectorGetRequest,
    VectorStub,
    DistanceFunction as ProtoDistanceFunction,
    VectorIndexType as ProtoVectorIndexType,
)
from norikv.types import (
    ClientConfig,
    DeleteOptions,
    GetOptions,
    GetResult,
    PutOptions,
    Version,
    # Vector types
    DistanceFunction,
    VectorIndexType,
    VectorMatch,
    VectorSearchResult,
    CreateVectorIndexOptions,
    DropVectorIndexOptions,
    VectorInsertOptions,
    VectorDeleteOptions,
    VectorSearchOptions,
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

    # =========================================================================
    # Vector Operations
    # =========================================================================

    async def vector_create_index(
        self,
        namespace: str,
        dimensions: int,
        distance: DistanceFunction,
        index_type: VectorIndexType,
        options: Optional[CreateVectorIndexOptions] = None,
    ) -> bool:
        """Create a vector index.

        Args:
            namespace: The namespace/index name
            dimensions: Number of dimensions for vectors
            distance: Distance function (euclidean, cosine, inner_product)
            index_type: Index type (brute_force, hnsw)
            options: Create options

        Returns:
            True if created, False if already existed
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        if dimensions <= 0:
            raise InvalidArgumentError("dimensions must be greater than 0")

        opts = options or CreateVectorIndexOptions()

        # Map to proto enum values
        proto_distance = self._to_proto_distance_function(distance)
        proto_index_type = self._to_proto_vector_index_type(index_type)

        request = CreateVectorIndexRequest(
            namespace=namespace,
            dimensions=dimensions,
            distance=proto_distance,
            index_type=proto_index_type,
            idempotency_key=opts.idempotency_key or "",
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.CreateIndex(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorCreateIndex"
        )
        return response.created

    async def vector_drop_index(
        self,
        namespace: str,
        options: Optional[DropVectorIndexOptions] = None,
    ) -> bool:
        """Drop a vector index.

        Args:
            namespace: The namespace/index name to drop
            options: Drop options

        Returns:
            True if dropped, False if didn't exist
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        opts = options or DropVectorIndexOptions()

        request = DropVectorIndexRequest(
            namespace=namespace,
            idempotency_key=opts.idempotency_key or "",
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.DropIndex(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorDropIndex"
        )
        return response.dropped

    async def vector_insert(
        self,
        namespace: str,
        id: str,
        vector: List[float],
        options: Optional[VectorInsertOptions] = None,
    ) -> Version:
        """Insert a vector into an index.

        Args:
            namespace: The namespace/index name
            id: Unique ID for the vector
            vector: The vector data
            options: Insert options

        Returns:
            Version of the inserted vector
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        if not id:
            raise InvalidArgumentError("id cannot be empty")

        if not vector:
            raise InvalidArgumentError("vector cannot be empty")

        opts = options or VectorInsertOptions()

        request = VectorInsertRequest(
            namespace=namespace,
            id=id,
            vector=vector,
            idempotency_key=opts.idempotency_key or "",
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.Insert(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorInsert"
        )
        return Version(term=response.version.term, index=response.version.index)

    async def vector_delete(
        self,
        namespace: str,
        id: str,
        options: Optional[VectorDeleteOptions] = None,
    ) -> bool:
        """Delete a vector from an index.

        Args:
            namespace: The namespace/index name
            id: ID of the vector to delete
            options: Delete options

        Returns:
            True if deleted, False if didn't exist
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        if not id:
            raise InvalidArgumentError("id cannot be empty")

        opts = options or VectorDeleteOptions()

        request = VectorDeleteRequest(
            namespace=namespace,
            id=id,
            idempotency_key=opts.idempotency_key or "",
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.Delete(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorDelete"
        )
        return response.deleted

    async def vector_search(
        self,
        namespace: str,
        query: List[float],
        k: int,
        options: Optional[VectorSearchOptions] = None,
    ) -> VectorSearchResult:
        """Search for nearest neighbors.

        Args:
            namespace: The namespace/index name
            query: Query vector
            k: Number of nearest neighbors to return
            options: Search options

        Returns:
            Search results with matches and timing
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        if not query:
            raise InvalidArgumentError("query vector cannot be empty")

        if k <= 0:
            raise InvalidArgumentError("k must be greater than 0")

        opts = options or VectorSearchOptions()

        request = VectorSearchRequest(
            namespace=namespace,
            query=query,
            k=k,
            include_vectors=opts.include_vectors,
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.Search(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorSearch"
        )

        matches = [
            VectorMatch(
                id=m.id,
                distance=m.distance,
                vector=list(m.vector) if m.vector else None,
            )
            for m in response.matches
        ]

        return VectorSearchResult(
            matches=matches,
            search_time_us=response.search_time_us,
        )

    async def vector_get(
        self,
        namespace: str,
        id: str,
    ) -> Optional[List[float]]:
        """Get a vector by ID.

        Args:
            namespace: The namespace/index name
            id: ID of the vector

        Returns:
            The vector data, or None if not found
        """
        if not self._connected:
            raise InvalidArgumentError("Client not connected")

        if not namespace:
            raise InvalidArgumentError("namespace cannot be empty")

        if not id:
            raise InvalidArgumentError("id cannot be empty")

        request = VectorGetRequest(
            namespace=namespace,
            id=id,
        )

        async def operation():
            channel = await self._get_any_channel()
            stub = VectorStub(channel)
            return await stub.Get(
                request, timeout=self._config.timeout / 1000.0
            )

        response = await self._retry_policy.execute_with_retry(
            operation, operation_name="VectorGet"
        )

        if not response.found:
            return None
        return list(response.vector) if response.vector else None

    async def _get_any_channel(self):
        """Get any available gRPC channel for non-key-routed operations."""
        nodes = self._topology.get_nodes()
        if not nodes:
            nodes = self._config.nodes

        for addr in nodes:
            try:
                return await self._pool.get_channel(addr)
            except Exception:
                continue

        raise NoNodesAvailableError()

    def _to_proto_distance_function(self, distance: DistanceFunction) -> int:
        """Convert SDK DistanceFunction to proto enum value."""
        if distance == DistanceFunction.EUCLIDEAN:
            return ProtoDistanceFunction.DISTANCE_FUNCTION_EUCLIDEAN
        elif distance == DistanceFunction.COSINE:
            return ProtoDistanceFunction.DISTANCE_FUNCTION_COSINE
        elif distance == DistanceFunction.INNER_PRODUCT:
            return ProtoDistanceFunction.DISTANCE_FUNCTION_INNER_PRODUCT
        else:
            return ProtoDistanceFunction.DISTANCE_FUNCTION_UNSPECIFIED

    def _to_proto_vector_index_type(self, index_type: VectorIndexType) -> int:
        """Convert SDK VectorIndexType to proto enum value."""
        if index_type == VectorIndexType.BRUTE_FORCE:
            return ProtoVectorIndexType.VECTOR_INDEX_TYPE_BRUTE_FORCE
        elif index_type == VectorIndexType.HNSW:
            return ProtoVectorIndexType.VECTOR_INDEX_TYPE_HNSW
        else:
            return ProtoVectorIndexType.VECTOR_INDEX_TYPE_UNSPECIFIED
