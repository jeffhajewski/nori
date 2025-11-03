"""Unit tests for type definitions."""

import pytest
from norikv.types import (
    ClientConfig,
    ClusterNode,
    ClusterView,
    DeleteOptions,
    GetOptions,
    GetResult,
    NodeConnection,
    NodeStatus,
    PutOptions,
    RetryConfig,
    RouteInfo,
    ShardInfo,
    ShardReplica,
    Version,
)


class TestVersion:
    """Tests for Version type."""

    def test_version_creation(self):
        """Should create version with term and index."""
        version = Version(term=1, index=100)
        assert version.term == 1
        assert version.index == 100

    def test_version_equality(self):
        """Versions with same term and index should be equal."""
        v1 = Version(term=1, index=100)
        v2 = Version(term=1, index=100)
        v3 = Version(term=1, index=101)
        v4 = Version(term=2, index=100)

        assert v1 == v2
        assert v1 != v3
        assert v1 != v4

    def test_version_hashable(self):
        """Versions should be hashable for use in sets/dicts."""
        v1 = Version(term=1, index=100)
        v2 = Version(term=1, index=100)
        v3 = Version(term=1, index=101)

        version_set = {v1, v2, v3}
        assert len(version_set) == 2  # v1 and v2 are equal

    def test_version_repr(self):
        """Version should have readable string representation."""
        version = Version(term=1, index=100)
        assert "Version" in repr(version)
        assert "term=1" in repr(version)
        assert "index=100" in repr(version)

    def test_version_immutable(self):
        """Version should be immutable (frozen dataclass)."""
        version = Version(term=1, index=100)
        with pytest.raises(Exception):  # FrozenInstanceError
            version.term = 2  # type: ignore


class TestOptions:
    """Tests for option types."""

    def test_put_options_defaults(self):
        """PutOptions should have sensible defaults."""
        opts = PutOptions()
        assert opts.ttl_ms is None
        assert opts.if_not_exists is False
        assert opts.if_match_version is None
        assert opts.idempotency_key is None

    def test_put_options_custom(self):
        """PutOptions should accept custom values."""
        version = Version(term=1, index=100)
        opts = PutOptions(
            ttl_ms=5000,
            if_not_exists=True,
            if_match_version=version,
            idempotency_key="test-key",
        )
        assert opts.ttl_ms == 5000
        assert opts.if_not_exists is True
        assert opts.if_match_version == version
        assert opts.idempotency_key == "test-key"

    def test_get_options_defaults(self):
        """GetOptions should have default consistency level."""
        opts = GetOptions()
        assert opts.consistency == "lease"

    def test_get_options_consistency_levels(self):
        """GetOptions should accept all consistency levels."""
        opts1 = GetOptions(consistency="lease")
        opts2 = GetOptions(consistency="linearizable")
        opts3 = GetOptions(consistency="stale_ok")

        assert opts1.consistency == "lease"
        assert opts2.consistency == "linearizable"
        assert opts3.consistency == "stale_ok"

    def test_delete_options_defaults(self):
        """DeleteOptions should have sensible defaults."""
        opts = DeleteOptions()
        assert opts.if_match_version is None
        assert opts.idempotency_key is None


class TestResults:
    """Tests for result types."""

    def test_get_result_with_value(self):
        """GetResult should hold value and version."""
        version = Version(term=1, index=100)
        result = GetResult(
            value=b"test-value", version=version, metadata={"key": "value"}
        )

        assert result.value == b"test-value"
        assert result.version == version
        assert result.metadata == {"key": "value"}

    def test_get_result_not_found(self):
        """GetResult should represent missing keys."""
        result = GetResult(value=None, version=None)

        assert result.value is None
        assert result.version is None


class TestClusterTypes:
    """Tests for cluster-related types."""

    def test_cluster_node(self):
        """ClusterNode should hold node information."""
        node = ClusterNode(id="node-1", addr="localhost:50051", role="leader")

        assert node.id == "node-1"
        assert node.addr == "localhost:50051"
        assert node.role == "leader"

    def test_shard_replica(self):
        """ShardReplica should hold replica information."""
        replica = ShardReplica(node_id="node-1", leader=True)

        assert replica.node_id == "node-1"
        assert replica.leader is True

    def test_shard_info(self):
        """ShardInfo should hold shard and replica information."""
        replicas = [
            ShardReplica(node_id="node-1", leader=True),
            ShardReplica(node_id="node-2", leader=False),
        ]
        shard = ShardInfo(id=42, replicas=replicas)

        assert shard.id == 42
        assert len(shard.replicas) == 2
        assert shard.replicas[0].leader is True

    def test_cluster_view(self):
        """ClusterView should hold complete cluster state."""
        nodes = [
            ClusterNode(id="node-1", addr="localhost:50051", role="leader"),
            ClusterNode(id="node-2", addr="localhost:50052", role="follower"),
        ]
        shards = [
            ShardInfo(
                id=0,
                replicas=[
                    ShardReplica(node_id="node-1", leader=True),
                    ShardReplica(node_id="node-2", leader=False),
                ],
            )
        ]
        view = ClusterView(epoch=1, nodes=nodes, shards=shards)

        assert view.epoch == 1
        assert len(view.nodes) == 2
        assert len(view.shards) == 1


class TestNodeConnection:
    """Tests for NodeConnection type."""

    def test_node_connection_defaults(self):
        """NodeConnection should have sensible defaults."""
        conn = NodeConnection(address="localhost:50051")

        assert conn.address == "localhost:50051"
        assert conn.status == NodeStatus.DISCONNECTED
        assert conn.channel is None
        assert conn.last_health_check is None

    def test_node_status_enum(self):
        """NodeStatus should be an enum."""
        assert NodeStatus.CONNECTED == "connected"
        assert NodeStatus.CONNECTING == "connecting"
        assert NodeStatus.DISCONNECTED == "disconnected"
        assert NodeStatus.FAILED == "failed"


class TestClientConfig:
    """Tests for ClientConfig type."""

    def test_client_config_minimal(self):
        """ClientConfig should require only nodes."""
        config = ClientConfig(nodes=["localhost:50051"])

        assert config.nodes == ["localhost:50051"]
        assert config.total_shards == 1024
        assert config.timeout == 5000
        assert config.watch_cluster is True
        assert isinstance(config.retry, RetryConfig)

    def test_client_config_custom(self):
        """ClientConfig should accept custom values."""
        retry = RetryConfig(max_attempts=5)
        config = ClientConfig(
            nodes=["localhost:50051", "localhost:50052"],
            total_shards=2048,
            timeout=10000,
            retry=retry,
            watch_cluster=False,
            max_connections_per_node=20,
        )

        assert len(config.nodes) == 2
        assert config.total_shards == 2048
        assert config.timeout == 10000
        assert config.retry.max_attempts == 5
        assert config.watch_cluster is False
        assert config.max_connections_per_node == 20


class TestRetryConfig:
    """Tests for RetryConfig type."""

    def test_retry_config_defaults(self):
        """RetryConfig should have sensible defaults."""
        config = RetryConfig()

        assert config.max_attempts == 3
        assert config.initial_delay_ms == 10
        assert config.max_delay_ms == 1000
        assert config.backoff_multiplier == 2.0
        assert config.jitter_ms == 100
        assert config.retry_on_not_leader is True

    def test_retry_config_custom(self):
        """RetryConfig should accept custom values."""
        config = RetryConfig(
            max_attempts=5,
            initial_delay_ms=50,
            max_delay_ms=5000,
            backoff_multiplier=3.0,
            jitter_ms=200,
            retry_on_not_leader=False,
        )

        assert config.max_attempts == 5
        assert config.initial_delay_ms == 50
        assert config.max_delay_ms == 5000
        assert config.backoff_multiplier == 3.0
        assert config.jitter_ms == 200
        assert config.retry_on_not_leader is False


class TestRouteInfo:
    """Tests for RouteInfo type."""

    def test_route_info_with_leader(self):
        """RouteInfo should hold routing information."""
        route = RouteInfo(
            shard_id=42,
            leader_addr="localhost:50051",
            replica_addrs=["localhost:50051", "localhost:50052"],
        )

        assert route.shard_id == 42
        assert route.leader_addr == "localhost:50051"
        assert len(route.replica_addrs) == 2

    def test_route_info_no_leader(self):
        """RouteInfo should handle missing leader."""
        route = RouteInfo(
            shard_id=42, leader_addr=None, replica_addrs=["localhost:50051"]
        )

        assert route.shard_id == 42
        assert route.leader_addr is None
        assert len(route.replica_addrs) == 1


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
