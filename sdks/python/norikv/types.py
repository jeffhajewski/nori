"""Type definitions for NoriKV Python client."""

from dataclasses import dataclass, field
from enum import Enum
from typing import Dict, List, Literal, Optional, Union


# Type aliases
Key = Union[bytes, str]
"""Key type - can be bytes or string."""

Value = Union[bytes, bytearray, str]
"""Value type - can be bytes, bytearray, or string."""


@dataclass(frozen=True)
class Version:
    """Version identifies a specific version of a key-value pair.

    Corresponds to the Raft log index where it was committed.
    """

    term: int
    """Raft term when this version was written."""

    index: int
    """Raft log index."""

    def __eq__(self, other: object) -> bool:
        if not isinstance(other, Version):
            return NotImplemented
        return self.term == other.term and self.index == other.index

    def __hash__(self) -> int:
        return hash((self.term, self.index))

    def __repr__(self) -> str:
        return f"Version(term={self.term}, index={self.index})"


ConsistencyLevel = Literal["lease", "linearizable", "stale_ok"]
"""Consistency level for Get operations.

- lease: Read from leader with lease (linearizable, fast)
- linearizable: Full linearizable read (read-index + quorum)
- stale_ok: Read from any replica (eventual consistency, fastest)
"""


@dataclass
class PutOptions:
    """Options for Put operations."""

    ttl_ms: Optional[int] = None
    """Time-to-live in milliseconds. None or 0 means no expiration."""

    if_not_exists: bool = False
    """Only write if key doesn't exist. Returns error if key exists."""

    if_match_version: Optional[Version] = None
    """Only write if current version matches. For optimistic concurrency control."""

    idempotency_key: Optional[str] = None
    """Idempotency key for retries. Same key + idempotency_key = same result."""


@dataclass
class GetOptions:
    """Options for Get operations."""

    consistency: ConsistencyLevel = "lease"
    """Consistency level for this read."""


@dataclass
class DeleteOptions:
    """Options for Delete operations."""

    if_match_version: Optional[Version] = None
    """Only delete if current version matches."""

    idempotency_key: Optional[str] = None
    """Idempotency key for retries."""


@dataclass
class GetResult:
    """Result of a Get operation."""

    value: Optional[bytes]
    """The value, or None if key doesn't exist."""

    version: Optional[Version]
    """Version of the value, or None if key doesn't exist."""

    metadata: Optional[Dict[str, str]] = None
    """Metadata from the server."""


@dataclass
class ClusterNode:
    """Cluster node information."""

    id: str
    """Unique node ID."""

    addr: str
    """gRPC address (host:port)."""

    role: str
    """Node role: "leader" | "follower" | "candidate"."""


@dataclass
class ShardReplica:
    """Shard replica information."""

    node_id: str
    """Node ID hosting this replica."""

    leader: bool
    """Whether this replica is the leader for this shard."""


@dataclass
class ShardInfo:
    """Shard information."""

    id: int
    """Shard ID (0 to total_shards-1)."""

    replicas: List[ShardReplica]
    """Replicas for this shard."""


@dataclass
class ClusterView:
    """Complete cluster view."""

    epoch: int
    """Monotonically increasing epoch number."""

    nodes: List[ClusterNode]
    """All nodes in the cluster."""

    shards: List[ShardInfo]
    """All shards and their replica assignments."""


class NodeStatus(str, Enum):
    """Node connection status."""

    CONNECTED = "connected"
    CONNECTING = "connecting"
    DISCONNECTED = "disconnected"
    FAILED = "failed"


@dataclass
class NodeConnection:
    """Connection configuration for a single node."""

    address: str
    """Node address (host:port)."""

    status: NodeStatus = NodeStatus.DISCONNECTED
    """Last known status."""

    channel: Optional[object] = None
    """gRPC channel (managed internally)."""

    last_health_check: Optional[float] = None
    """Last successful health check timestamp (Unix timestamp)."""


@dataclass
class RetryConfig:
    """Retry policy configuration."""

    max_attempts: int = 3
    """Maximum number of retry attempts."""

    initial_delay_ms: int = 10
    """Initial backoff delay in milliseconds."""

    max_delay_ms: int = 1000
    """Maximum backoff delay in milliseconds."""

    backoff_multiplier: float = 2.0
    """Backoff multiplier."""

    jitter_ms: int = 100
    """Maximum jitter in milliseconds."""

    retry_on_not_leader: bool = True
    """Whether to retry on NOT_LEADER errors."""


@dataclass
class ClientConfig:
    """Configuration for NoriKVClient."""

    nodes: List[str]
    """List of node addresses to connect to (at least one required)."""

    total_shards: int = 1024
    """Total number of virtual shards."""

    timeout: int = 5000
    """Default request timeout in milliseconds."""

    retry: RetryConfig = field(default_factory=RetryConfig)
    """Retry policy configuration."""

    watch_cluster: bool = True
    """Enable cluster view watching."""

    max_connections_per_node: int = 10
    """Max connections per node."""


@dataclass
class RouteInfo:
    """Routing information for a key."""

    shard_id: int
    """Shard ID this key belongs to."""

    leader_addr: Optional[str]
    """Leader node address for this shard (if known)."""

    replica_addrs: List[str]
    """All replica addresses for this shard."""
