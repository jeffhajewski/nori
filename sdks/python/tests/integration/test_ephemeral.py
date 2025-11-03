"""Integration tests for ephemeral NoriKV server."""

import pytest
import shutil

from norikv import create_ephemeral, EphemeralNoriKV, EphemeralServerError


# Skip all tests in this module if norikv-server is not available
pytestmark = pytest.mark.skipif(
    shutil.which("norikv-server") is None,
    reason="norikv-server binary not found in PATH"
)


@pytest.mark.integration
@pytest.mark.e2e
class TestEphemeralNoriKV:
    """Tests for ephemeral server functionality."""

    async def test_create_ephemeral_context_manager(self):
        """Test creating ephemeral server using context manager."""
        async with create_ephemeral() as cluster:
            # Should get a client successfully
            client = cluster.get_client()

            # Client should be able to connect and perform operations
            async with client:
                # Put a value
                version = await client.put(b"test-key", b"test-value")
                assert version is not None
                assert version.term > 0
                assert version.index > 0

                # Get the value back
                result = await client.get(b"test-key")
                assert result.value == b"test-value"
                assert result.version == version

                # Delete the value
                deleted = await client.delete(b"test-key")
                assert deleted is True

                # Verify deletion
                result = await client.get(b"test-key")
                assert result.value is None

    async def test_ephemeral_manual_lifecycle(self):
        """Test manual start/stop of ephemeral server."""
        cluster = EphemeralNoriKV()

        try:
            # Start server
            await cluster.start()

            # Get client and perform operations
            client = cluster.get_client()
            async with client:
                await client.put(b"key1", b"value1")
                result = await client.get(b"key1")
                assert result.value == b"value1"

        finally:
            # Cleanup
            await cluster.stop()

    async def test_multiple_ephemeral_instances(self):
        """Test running multiple ephemeral servers concurrently."""
        async with create_ephemeral() as cluster1:
            async with create_ephemeral() as cluster2:
                # Ensure they're on different ports
                assert cluster1.port != cluster2.port

                # Each should be independent
                client1 = cluster1.get_client()
                client2 = cluster2.get_client()

                async with client1, client2:
                    # Write to cluster 1
                    await client1.put(b"key", b"value1")

                    # Write to cluster 2
                    await client2.put(b"key", b"value2")

                    # Values should be independent
                    result1 = await client1.get(b"key")
                    result2 = await client2.get(b"key")

                    assert result1.value == b"value1"
                    assert result2.value == b"value2"

    async def test_ephemeral_crud_operations(self):
        """Test basic CRUD operations on ephemeral server."""
        async with create_ephemeral() as cluster:
            client = cluster.get_client()

            async with client:
                # Create
                v1 = await client.put(b"user:123", b"Alice")
                assert v1 is not None

                # Read
                result = await client.get(b"user:123")
                assert result.value == b"Alice"
                assert result.version == v1

                # Update
                v2 = await client.put(b"user:123", b"Bob")
                assert v2.index > v1.index

                # Read updated
                result = await client.get(b"user:123")
                assert result.value == b"Bob"
                assert result.version == v2

                # Delete
                deleted = await client.delete(b"user:123")
                assert deleted is True

                # Read after delete
                result = await client.get(b"user:123")
                assert result.value is None

    async def test_ephemeral_with_string_keys_values(self):
        """Test ephemeral server with string keys and values."""
        async with create_ephemeral() as cluster:
            client = cluster.get_client()

            async with client:
                # Put with string
                await client.put("name", "Alice")

                # Get back
                result = await client.get("name")
                assert result.value == b"Alice"
                assert result.value.decode() == "Alice"

    async def test_ephemeral_custom_shard_count(self):
        """Test creating ephemeral server with custom shard count."""
        async with create_ephemeral(total_shards=128) as cluster:
            assert cluster.total_shards == 128

            client = cluster.get_client()
            async with client:
                await client.put(b"test", b"value")
                result = await client.get(b"test")
                assert result.value == b"value"

    async def test_get_client_before_start_raises_error(self):
        """Test that getting client before starting raises error."""
        cluster = EphemeralNoriKV()

        with pytest.raises(EphemeralServerError, match="not started"):
            cluster.get_client()

    async def test_ephemeral_cleanup_on_exit(self):
        """Test that temp directories are cleaned up after exit."""
        temp_dir = None

        async with create_ephemeral() as cluster:
            temp_dir = cluster._temp_dir
            # Temp dir should exist while running
            if temp_dir:
                import os
                assert os.path.exists(temp_dir)

        # After exit, temp dir should be removed
        if temp_dir:
            import os
            assert not os.path.exists(temp_dir)


@pytest.mark.unit
class TestBinaryDiscovery:
    """Tests for norikv-server binary discovery."""

    def test_binary_not_found_raises_helpful_error(self):
        """Test that missing binary raises helpful error message."""
        import os

        # Temporarily clear PATH and NORIKV_SERVER_PATH
        old_path = os.environ.get('PATH', '')
        old_server_path = os.environ.get('NORIKV_SERVER_PATH')

        try:
            os.environ['PATH'] = ''
            if old_server_path:
                del os.environ['NORIKV_SERVER_PATH']

            cluster = EphemeralNoriKV()

            # Should raise helpful error when trying to start
            import asyncio
            with pytest.raises(EphemeralServerError, match="norikv-server binary not found"):
                asyncio.run(cluster.start())

        finally:
            # Restore environment
            os.environ['PATH'] = old_path
            if old_server_path:
                os.environ['NORIKV_SERVER_PATH'] = old_server_path
