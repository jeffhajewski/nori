"""Basic integration tests for NoriKVClient (no mock server required)."""

import pytest

from norikv import NoriKVClient, ClientConfig


@pytest.mark.asyncio
class TestClientBasic:
    """Test basic client functionality without a server."""

    async def test_client_creation(self):
        """Test creating a client."""
        config = ClientConfig(
            nodes=["localhost:50051"],
            watch_cluster=False,
        )
        client = NoriKVClient(config)
        assert client is not None
        assert not client.is_connected()

    async def test_client_context_manager_structure(self):
        """Test client can be used as context manager (structure only)."""
        config = ClientConfig(
            nodes=["localhost:50051"],
            watch_cluster=False,
        )

        # Just test that the context manager protocol works
        # (Don't actually connect)
        client = NoriKVClient(config)
        assert hasattr(client, "__aenter__")
        assert hasattr(client, "__aexit__")

    def test_shard_calculation(self):
        """Test shard calculation works without connection."""
        config = ClientConfig(
            nodes=["localhost:50051"],
            total_shards=1024,
            watch_cluster=False,
        )
        client = NoriKVClient(config)

        # Test shard calculation
        shard = client.get_shard_for_key("test-key")
        assert 0 <= shard < 1024

        # Same key should always map to same shard
        shard2 = client.get_shard_for_key("test-key")
        assert shard == shard2

    def test_shard_distribution(self):
        """Test that keys are distributed across shards."""
        config = ClientConfig(
            nodes=["localhost:50051"],
            total_shards=1024,
            watch_cluster=False,
        )
        client = NoriKVClient(config)

        shards = set()
        for i in range(100):
            shard = client.get_shard_for_key(f"key-{i}")
            shards.add(shard)

        # Should use multiple shards
        assert len(shards) > 10

    async def test_configuration_values(self):
        """Test client configuration is stored correctly."""
        config = ClientConfig(
            nodes=["node1:50051", "node2:50051"],
            total_shards=2048,
            timeout=10000,
            watch_cluster=False,
        )
        client = NoriKVClient(config)

        assert client._config.nodes == ["node1:50051", "node2:50051"]
        assert client._config.total_shards == 2048
        assert client._config.timeout == 10000
        assert client._config.watch_cluster is False
