"""Request router for directing operations to appropriate cluster nodes."""

import asyncio
from typing import Callable, Optional, TypeVar, Any

from norikv._internal.connection import ConnectionPool
from norikv._internal.topology import TopologyManager
from norikv.errors import (
    NotLeaderError,
    UnavailableError,
    from_grpc_error,
)
from norikv.hash import get_shard_for_key
from norikv.proto import KvStub, MetaStub

T = TypeVar("T")


class Router:
    """Routes requests to the appropriate cluster nodes."""

    def __init__(
        self,
        pool: ConnectionPool,
        topology: TopologyManager,
        max_redirects: int = 5,
    ):
        """Initialize the router.

        Args:
            pool: Connection pool for gRPC stubs
            topology: Topology manager for cluster view
            max_redirects: Maximum number of leader redirects to follow
        """
        self._pool = pool
        self._topology = topology
        self._max_redirects = max_redirects

    async def route_kv_request(
        self,
        key: bytes,
        request_fn: Callable[[KvStub], Any],
        prefer_leader: bool = True,
    ) -> T:
        """Route a KV request to the appropriate node.

        Args:
            key: The key being operated on
            request_fn: Async function that takes a KvStub and returns a response
            prefer_leader: If True, prefer routing to the leader node

        Returns:
            The response from the request

        Raises:
            NotLeaderError: If max redirects exceeded
            UnavailableError: If no healthy nodes available
        """
        # Calculate shard for the key
        total_shards = self._topology.get_total_shards()
        shard_id = get_shard_for_key(key, total_shards)

        # Track redirect attempts
        redirects = 0
        last_error: Optional[Exception] = None

        while redirects < self._max_redirects:
            try:
                # Select target node
                node_address = self._topology.select_node_for_shard(
                    shard_id, prefer_leader=prefer_leader
                )

                # Get stub for the node
                stub = await self._pool.get_kv_stub(node_address)

                # Execute request
                response = await request_fn(stub)

                # Mark success and return
                self._topology.mark_node_success(node_address)
                return response

            except Exception as e:
                # Convert gRPC error
                error = from_grpc_error(e)

                # Mark node failure
                if hasattr(e, "__context__") and e.__context__:
                    node_addr = getattr(e.__context__, "node_address", None)
                    if node_addr:
                        self._topology.mark_node_failure(
                            node_addr, str(error)
                        )

                # Handle NOT_LEADER errors with redirect
                if isinstance(error, NotLeaderError):
                    redirects += 1
                    last_error = error

                    # Update topology if leader hint provided
                    if error.leader_hint:
                        self._topology.add_node(error.leader_hint)

                    # Continue to retry with new leader
                    continue

                # Re-raise other errors immediately
                raise error

        # Max redirects exceeded
        raise NotLeaderError(
            f"Max redirects ({self._max_redirects}) exceeded for shard {shard_id}",
            shard_id=shard_id,
        )

    async def route_meta_request(
        self,
        request_fn: Callable[[MetaStub], Any],
        node_address: Optional[str] = None,
    ) -> T:
        """Route a Meta request to a cluster node.

        Args:
            request_fn: Async function that takes a MetaStub and returns a response
            node_address: Optional specific node to target (uses any healthy node if None)

        Returns:
            The response from the request

        Raises:
            UnavailableError: If no healthy nodes available
        """
        # Select target node
        if node_address is None:
            healthy_nodes = self._topology.get_healthy_nodes()
            if not healthy_nodes:
                raise UnavailableError(
                    "No healthy nodes available for Meta request",
                    retry_after_ms=1000,
                )
            node_address = healthy_nodes[0]

        try:
            # Get stub for the node
            stub = await self._pool.get_meta_stub(node_address)

            # Execute request
            response = await request_fn(stub)

            # Mark success and return
            self._topology.mark_node_success(node_address)
            return response

        except Exception as e:
            # Convert gRPC error
            error = from_grpc_error(e)

            # Mark node failure
            self._topology.mark_node_failure(node_address, str(error))

            # Re-raise
            raise error

    async def broadcast_meta_request(
        self,
        request_fn: Callable[[MetaStub], Any],
    ) -> list[T]:
        """Broadcast a Meta request to all healthy nodes.

        Args:
            request_fn: Async function that takes a MetaStub and returns a response

        Returns:
            List of responses from all nodes

        Raises:
            UnavailableError: If no healthy nodes available
        """
        healthy_nodes = self._topology.get_healthy_nodes()
        if not healthy_nodes:
            raise UnavailableError(
                "No healthy nodes available for broadcast",
                retry_after_ms=1000,
            )

        # Create tasks for all nodes
        tasks = []
        for node_address in healthy_nodes:
            task = self._broadcast_to_node(node_address, request_fn)
            tasks.append(task)

        # Wait for all tasks
        results = await asyncio.gather(*tasks, return_exceptions=True)

        # Filter out exceptions and return successful responses
        responses = []
        for i, result in enumerate(results):
            if isinstance(result, Exception):
                # Mark node as failed
                self._topology.mark_node_failure(
                    healthy_nodes[i], str(result)
                )
            else:
                responses.append(result)

        return responses

    async def _broadcast_to_node(
        self,
        node_address: str,
        request_fn: Callable[[MetaStub], Any],
    ) -> T:
        """Helper to broadcast to a single node.

        Args:
            node_address: Node to send request to
            request_fn: Async function that takes a MetaStub

        Returns:
            The response from the node
        """
        stub = await self._pool.get_meta_stub(node_address)
        response = await request_fn(stub)
        self._topology.mark_node_success(node_address)
        return response
