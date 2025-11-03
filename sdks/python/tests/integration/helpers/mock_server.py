"""Mock gRPC server for integration testing."""

import asyncio
from typing import Dict, Optional
from dataclasses import dataclass, field
from datetime import datetime

import grpc
from grpc import aio

from norikv.proto import (
    KvServicer,
    MetaServicer,
    PutRequest,
    PutResponse,
    GetRequest,
    GetResponse,
    DeleteRequest,
    DeleteResponse,
    Version as ProtoVersion,
)


@dataclass
class StoredValue:
    """Value stored in mock server."""

    value: bytes
    version: ProtoVersion
    ttl_ms: int = 0
    created_at: datetime = field(default_factory=datetime.now)


class MockKvServicer(KvServicer):
    """Mock implementation of KV service."""

    def __init__(self):
        """Initialize the mock KV servicer."""
        self._store: Dict[bytes, StoredValue] = {}
        self._version_counter = 0
        self._is_leader = True
        self._leader_hint: Optional[str] = None

    def set_leader_status(self, is_leader: bool, leader_hint: Optional[str] = None):
        """Set whether this node is the leader."""
        self._is_leader = is_leader
        self._leader_hint = leader_hint

    def clear(self):
        """Clear all stored data."""
        self._store.clear()
        self._version_counter = 0

    def _next_version(self) -> ProtoVersion:
        """Generate next version."""
        self._version_counter += 1
        return ProtoVersion(term=1, index=self._version_counter)

    async def Put(self, request: PutRequest, context: grpc.aio.ServicerContext) -> PutResponse:
        """Handle Put request."""
        # Check if we're the leader
        if not self._is_leader:
            await context.abort(
                grpc.StatusCode.FAILED_PRECONDITION,
                f"NOT_LEADER: {self._leader_hint or 'unknown'}",
            )

        # Check if_not_exists
        if request.HasField("if_not_exists") and request.key in self._store:
            await context.abort(
                grpc.StatusCode.ALREADY_EXISTS,
                "Key already exists",
            )

        # Check if_match version
        if request.HasField("if_match"):
            stored = self._store.get(request.key)
            if not stored:
                await context.abort(
                    grpc.StatusCode.NOT_FOUND,
                    "Key not found for conditional update",
                )
            if (
                stored.version.term != request.if_match.term
                or stored.version.index != request.if_match.index
            ):
                await context.abort(
                    grpc.StatusCode.FAILED_PRECONDITION,
                    "Version mismatch",
                )

        # Store the value
        version = self._next_version()
        self._store[request.key] = StoredValue(
            value=request.value,
            version=version,
            ttl_ms=request.ttl_ms,
        )

        return PutResponse(version=version)

    async def Get(self, request: GetRequest, context: grpc.aio.ServicerContext) -> GetResponse:
        """Handle Get request."""
        stored = self._store.get(request.key)
        if not stored:
            return GetResponse()

        # Check TTL expiration
        if stored.ttl_ms > 0:
            elapsed_ms = (datetime.now() - stored.created_at).total_seconds() * 1000
            if elapsed_ms > stored.ttl_ms:
                # Expired - remove and return not found
                del self._store[request.key]
                return GetResponse()

        return GetResponse(
            value=stored.value,
            version=stored.version,
        )

    async def Delete(
        self, request: DeleteRequest, context: grpc.aio.ServicerContext
    ) -> DeleteResponse:
        """Handle Delete request."""
        # Check if we're the leader
        if not self._is_leader:
            await context.abort(
                grpc.StatusCode.FAILED_PRECONDITION,
                f"NOT_LEADER: {self._leader_hint or 'unknown'}",
            )

        # Check if_match version
        if request.HasField("if_match"):
            stored = self._store.get(request.key)
            if not stored:
                return DeleteResponse(tombstoned=False)
            if (
                stored.version.term != request.if_match.term
                or stored.version.index != request.if_match.index
            ):
                await context.abort(
                    grpc.StatusCode.FAILED_PRECONDITION,
                    "Version mismatch",
                )

        # Delete the key
        if request.key in self._store:
            del self._store[request.key]
            return DeleteResponse(tombstoned=True)

        return DeleteResponse(tombstoned=False)


class MockMetaServicer(MetaServicer):
    """Mock implementation of Meta service."""

    def __init__(self):
        """Initialize the mock Meta servicer."""
        self._is_healthy = True

    def set_health(self, is_healthy: bool):
        """Set the health status."""
        self._is_healthy = is_healthy


class MockServer:
    """Mock gRPC server for testing."""

    def __init__(self, port: int = 0):
        """Initialize the mock server.

        Args:
            port: Port to listen on (0 for random available port)
        """
        self._port = port
        self._server: Optional[aio.Server] = None
        self._kv_servicer = MockKvServicer()
        self._meta_servicer = MockMetaServicer()

    async def start(self) -> int:
        """Start the mock server.

        Returns:
            The port the server is listening on
        """
        from norikv.proto import add_KvServicer_to_server, add_MetaServicer_to_server

        self._server = aio.server()
        add_KvServicer_to_server(self._kv_servicer, self._server)
        add_MetaServicer_to_server(self._meta_servicer, self._server)

        port = self._server.add_insecure_port(f"[::]:{self._port}")
        await self._server.start()

        return port

    async def stop(self):
        """Stop the mock server."""
        if self._server:
            await self._server.stop(grace=1.0)
            self._server = None

    def get_kv_servicer(self) -> MockKvServicer:
        """Get the KV servicer for test control."""
        return self._kv_servicer

    def get_meta_servicer(self) -> MockMetaServicer:
        """Get the Meta servicer for test control."""
        return self._meta_servicer

    async def __aenter__(self) -> "MockServer":
        """Async context manager entry."""
        await self.start()
        return self

    async def __aexit__(self, exc_type, exc_val, exc_tb):
        """Async context manager exit."""
        await self.stop()
