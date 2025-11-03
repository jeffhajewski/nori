"""Connection pool management for NoriKV cluster nodes."""

import asyncio
import grpc
from typing import Dict, Optional

from norikv.errors import ConnectionError as NoriConnectionError
from norikv.types import NodeConnection, NodeStatus
from norikv.proto import KvStub, MetaStub


class ConnectionPool:
    """Manages gRPC connections to cluster nodes."""

    def __init__(
        self,
        max_connections_per_node: int = 10,
        connect_timeout: int = 5000,
    ):
        self._connections: Dict[str, NodeConnection] = {}
        self._channels: Dict[str, grpc.aio.Channel] = {}
        self._kv_stubs: Dict[str, KvStub] = {}
        self._meta_stubs: Dict[str, MetaStub] = {}
        self._closed = False
        self._lock = asyncio.Lock()
        self._max_connections = max_connections_per_node
        self._connect_timeout = connect_timeout / 1000.0  # Convert to seconds

    async def get_kv_stub(self, address: str) -> KvStub:
        """Get or create a KV service stub for the given address."""
        if self._closed:
            raise NoriConnectionError("Connection pool is closed", address)

        async with self._lock:
            if address not in self._kv_stubs:
                channel = await self._get_or_create_channel(address)
                self._kv_stubs[address] = KvStub(channel)
            return self._kv_stubs[address]

    async def get_meta_stub(self, address: str) -> MetaStub:
        """Get or create a Meta service stub for the given address."""
        if self._closed:
            raise NoriConnectionError("Connection pool is closed", address)

        async with self._lock:
            if address not in self._meta_stubs:
                channel = await self._get_or_create_channel(address)
                self._meta_stubs[address] = MetaStub(channel)
            return self._meta_stubs[address]

    async def _get_or_create_channel(
        self, address: str
    ) -> grpc.aio.Channel:
        """Get or create a gRPC channel for the given address."""
        if address not in self._channels:
            # Create a secure channel (use insecure for development)
            channel = grpc.aio.insecure_channel(
                address,
                options=[
                    ("grpc.max_send_message_length", 100 * 1024 * 1024),
                    ("grpc.max_receive_message_length", 100 * 1024 * 1024),
                    ("grpc.keepalive_time_ms", 30000),
                    ("grpc.keepalive_timeout_ms", 10000),
                ],
            )

            self._channels[address] = channel
            self._connections[address] = NodeConnection(
                address=address, status=NodeStatus.CONNECTED
            )

        return self._channels[address]

    async def close(self) -> None:
        """Close all connections in the pool."""
        if self._closed:
            return

        self._closed = True

        async with self._lock:
            # Close all channels
            for channel in self._channels.values():
                await channel.close()

            self._channels.clear()
            self._kv_stubs.clear()
            self._meta_stubs.clear()
            self._connections.clear()

    def is_closed(self) -> bool:
        """Check if the connection pool is closed."""
        return self._closed

    def get_connection_status(self, address: str) -> Optional[NodeStatus]:
        """Get the connection status for a node."""
        conn = self._connections.get(address)
        return conn.status if conn else None
