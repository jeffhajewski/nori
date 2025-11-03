"""Integration tests for NoriKVClient."""

import pytest

from norikv import (
    NoriKVClient,
    ClientConfig,
    PutOptions,
    GetOptions,
    DeleteOptions,
    Version,
    AlreadyExistsError,
    VersionMismatchError,
)
from tests.integration.helpers.mock_server import MockServer


@pytest.fixture
async def mock_server():
    """Create and start a mock server."""
    server = MockServer(port=0)
    port = await server.start()
    server._actual_port = port
    yield server
    await server.stop()


@pytest.fixture
async def client(mock_server):
    """Create a client connected to the mock server."""
    config = ClientConfig(
        nodes=[f"localhost:{mock_server._actual_port}"],
        total_shards=1024,
        watch_cluster=False,  # Disable cluster watching for tests
    )
    async with NoriKVClient(config) as client:
        yield client


@pytest.mark.asyncio
class TestBasicOperations:
    """Test basic CRUD operations."""

    async def test_put_and_get(self, client, mock_server):
        """Test basic put and get operations."""
        # Put a value
        version = await client.put("test-key", "test-value")
        assert isinstance(version, Version)
        assert version.term > 0
        assert version.index > 0

        # Get the value back
        result = await client.get("test-key")
        assert result.value == b"test-value"
        assert result.version == version

    async def test_get_nonexistent_key(self, client):
        """Test getting a key that doesn't exist."""
        result = await client.get("nonexistent")
        assert result.value is None
        assert result.version is None

    async def test_put_with_bytes(self, client):
        """Test putting bytes values."""
        value = b"binary\x00data\xff"
        version = await client.put(b"binary-key", value)

        result = await client.get(b"binary-key")
        assert result.value == value
        assert result.version == version

    async def test_delete(self, client):
        """Test delete operation."""
        # Put a value
        await client.put("delete-me", "value")

        # Delete it
        deleted = await client.delete("delete-me")
        assert deleted is True

        # Verify it's gone
        result = await client.get("delete-me")
        assert result.value is None

    async def test_delete_nonexistent(self, client):
        """Test deleting a nonexistent key."""
        deleted = await client.delete("does-not-exist")
        assert deleted is False

    async def test_multiple_operations(self, client):
        """Test multiple sequential operations."""
        # Put multiple keys
        keys = [f"key-{i}" for i in range(10)]
        for key in keys:
            await client.put(key, f"value-{key}")

        # Get them back
        for key in keys:
            result = await client.get(key)
            assert result.value == f"value-{key}".encode()

        # Delete them
        for key in keys:
            deleted = await client.delete(key)
            assert deleted is True


@pytest.mark.asyncio
class TestConditionalOperations:
    """Test conditional put and delete operations."""

    async def test_put_if_not_exists_success(self, client):
        """Test conditional put when key doesn't exist."""
        version = await client.put(
            "new-key",
            "value",
            PutOptions(if_not_exists=True),
        )
        assert isinstance(version, Version)

    async def test_put_if_not_exists_fails(self, client):
        """Test conditional put when key already exists."""
        # Put initial value
        await client.put("existing-key", "value1")

        # Try to put with if_not_exists
        with pytest.raises(AlreadyExistsError):
            await client.put(
                "existing-key",
                "value2",
                PutOptions(if_not_exists=True),
            )

    async def test_put_if_match_version_success(self, client):
        """Test conditional put with matching version."""
        # Put initial value
        v1 = await client.put("versioned-key", "value1")

        # Update with correct version
        v2 = await client.put(
            "versioned-key",
            "value2",
            PutOptions(if_match_version=v1),
        )
        assert v2.index > v1.index

    async def test_put_if_match_version_fails(self, client):
        """Test conditional put with mismatched version."""
        # Put initial value
        v1 = await client.put("versioned-key", "value1")

        # Update it
        await client.put("versioned-key", "value2")

        # Try to update with old version
        with pytest.raises(VersionMismatchError):
            await client.put(
                "versioned-key",
                "value3",
                PutOptions(if_match_version=v1),
            )

    async def test_delete_if_match_version_success(self, client):
        """Test conditional delete with matching version."""
        # Put a value
        version = await client.put("delete-key", "value")

        # Delete with correct version
        deleted = await client.delete(
            "delete-key",
            DeleteOptions(if_match_version=version),
        )
        assert deleted is True

    async def test_delete_if_match_version_fails(self, client):
        """Test conditional delete with mismatched version."""
        # Put initial value
        v1 = await client.put("delete-key", "value1")

        # Update it
        await client.put("delete-key", "value2")

        # Try to delete with old version
        with pytest.raises(VersionMismatchError):
            await client.delete(
                "delete-key",
                DeleteOptions(if_match_version=v1),
            )


@pytest.mark.asyncio
class TestTTL:
    """Test TTL functionality."""

    async def test_put_with_ttl(self, client):
        """Test putting a value with TTL."""
        # Put with 5 second TTL
        version = await client.put(
            "ttl-key",
            "temporary",
            PutOptions(ttl_ms=5000),
        )
        assert isinstance(version, Version)

        # Value should exist immediately
        result = await client.get("ttl-key")
        assert result.value == b"temporary"

    async def test_ttl_expiration(self, client, mock_server):
        """Test that values with TTL expire."""
        import asyncio

        # Put with very short TTL
        await client.put(
            "expire-key",
            "will-expire",
            PutOptions(ttl_ms=100),
        )

        # Wait for expiration
        await asyncio.sleep(0.2)

        # Value should be gone
        result = await client.get("expire-key")
        assert result.value is None


@pytest.mark.asyncio
class TestConsistency:
    """Test consistency level options."""

    async def test_get_with_lease_consistency(self, client):
        """Test get with lease consistency."""
        await client.put("consistency-key", "value")

        result = await client.get(
            "consistency-key",
            GetOptions(consistency="lease"),
        )
        assert result.value == b"value"

    async def test_get_with_linearizable_consistency(self, client):
        """Test get with linearizable consistency."""
        await client.put("consistency-key", "value")

        result = await client.get(
            "consistency-key",
            GetOptions(consistency="linearizable"),
        )
        assert result.value == b"value"

    async def test_get_with_stale_ok_consistency(self, client):
        """Test get with stale_ok consistency."""
        await client.put("consistency-key", "value")

        result = await client.get(
            "consistency-key",
            GetOptions(consistency="stale_ok"),
        )
        assert result.value == b"value"


@pytest.mark.asyncio
class TestIdempotency:
    """Test idempotency key functionality."""

    async def test_put_with_idempotency_key(self, client):
        """Test put with idempotency key."""
        version = await client.put(
            "idempotent-key",
            "value",
            PutOptions(idempotency_key="request-123"),
        )
        assert isinstance(version, Version)

    async def test_delete_with_idempotency_key(self, client):
        """Test delete with idempotency key."""
        await client.put("idempotent-key", "value")

        deleted = await client.delete(
            "idempotent-key",
            DeleteOptions(idempotency_key="delete-123"),
        )
        assert deleted is True


@pytest.mark.asyncio
class TestSharding:
    """Test shard calculation."""

    def test_get_shard_for_key(self, client):
        """Test shard calculation for keys."""
        shard = client.get_shard_for_key("test-key")
        assert 0 <= shard < 1024

        # Same key should always map to same shard
        shard2 = client.get_shard_for_key("test-key")
        assert shard == shard2

    def test_shard_distribution(self, client):
        """Test that keys are distributed across shards."""
        shards = set()
        for i in range(100):
            shard = client.get_shard_for_key(f"key-{i}")
            shards.add(shard)

        # Should use multiple shards
        assert len(shards) > 10


@pytest.mark.asyncio
class TestClientLifecycle:
    """Test client connection lifecycle."""

    async def test_client_context_manager(self, mock_server):
        """Test using client as context manager."""
        config = ClientConfig(
            nodes=[f"localhost:{mock_server._actual_port}"],
            watch_cluster=False,
        )

        async with NoriKVClient(config) as client:
            assert client.is_connected()
            await client.put("test", "value")

        # Client should be closed after context exit
        assert not client.is_connected()

    async def test_manual_connect_close(self, mock_server):
        """Test manual connect and close."""
        config = ClientConfig(
            nodes=[f"localhost:{mock_server._actual_port}"],
            watch_cluster=False,
        )

        client = NoriKVClient(config)
        assert not client.is_connected()

        await client.connect()
        assert client.is_connected()

        await client.close()
        assert not client.is_connected()
