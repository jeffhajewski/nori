"""Topology manager for tracking cluster view and node discovery."""

import asyncio
from typing import Dict, List, Optional, Set
from dataclasses import dataclass, field
from datetime import datetime

from norikv.errors import UnavailableError
from norikv.types import ClusterNode, ClusterView, ShardInfo


@dataclass
class NodeHealth:
    """Health status for a cluster node."""

    address: str
    is_healthy: bool = True
    last_check: datetime = field(default_factory=datetime.now)
    consecutive_failures: int = 0
    last_error: Optional[str] = None


class TopologyManager:
    """Manages cluster topology and node health tracking."""

    def __init__(
        self,
        initial_nodes: List[str],
        total_shards: int = 1024,
        health_check_interval: float = 5.0,
        max_failures_before_unhealthy: int = 3,
    ):
        """Initialize the topology manager.

        Args:
            initial_nodes: Initial list of node addresses
            total_shards: Total number of virtual shards (default: 1024)
            health_check_interval: Seconds between health checks (default: 5.0)
            max_failures_before_unhealthy: Consecutive failures before marking unhealthy
        """
        self._initial_nodes = initial_nodes
        self._total_shards = total_shards
        self._health_check_interval = health_check_interval
        self._max_failures = max_failures_before_unhealthy

        # Cluster view
        self._cluster_view: Optional[ClusterView] = None
        self._nodes: Dict[str, ClusterNode] = {}
        self._shard_map: Dict[int, ShardInfo] = {}

        # Health tracking
        self._node_health: Dict[str, NodeHealth] = {}
        for node in initial_nodes:
            self._node_health[node] = NodeHealth(address=node)

        # Background tasks
        self._watch_task: Optional[asyncio.Task] = None
        self._health_task: Optional[asyncio.Task] = None
        self._running = False
        self._lock = asyncio.Lock()

    async def start(self) -> None:
        """Start the topology manager background tasks."""
        if self._running:
            return

        self._running = True

        # Start health checking (cluster view watching would be added here)
        self._health_task = asyncio.create_task(self._health_check_loop())

    async def stop(self) -> None:
        """Stop the topology manager background tasks."""
        if not self._running:
            return

        self._running = False

        # Cancel background tasks
        if self._watch_task:
            self._watch_task.cancel()
            try:
                await self._watch_task
            except asyncio.CancelledError:
                pass

        if self._health_task:
            self._health_task.cancel()
            try:
                await self._health_task
            except asyncio.CancelledError:
                pass

    async def _health_check_loop(self) -> None:
        """Background loop for health checking nodes."""
        while self._running:
            try:
                await asyncio.sleep(self._health_check_interval)
                # Health checking would call Meta.Health RPC here
                # For now, this is a placeholder for future implementation
            except asyncio.CancelledError:
                break
            except Exception:
                # Log error but continue checking
                pass

    def get_cluster_view(self) -> Optional[ClusterView]:
        """Get the current cluster view.

        Returns:
            Current cluster view or None if not yet retrieved
        """
        return self._cluster_view

    def update_cluster_view(self, view: ClusterView) -> None:
        """Update the cluster view.

        Args:
            view: New cluster view from the cluster
        """
        self._cluster_view = view
        self._build_shard_map(view)

    def _build_shard_map(self, view: ClusterView) -> None:
        """Build shard assignment map from cluster view.

        Args:
            view: Cluster view containing shard assignments
        """
        self._shard_map.clear()
        for shard in view.shards:
            self._shard_map[shard.id] = shard

    def get_shard_info(self, shard_id: int) -> Optional[ShardInfo]:
        """Get the info for a specific shard.

        Args:
            shard_id: The shard ID to look up

        Returns:
            ShardInfo if found, None otherwise
        """
        return self._shard_map.get(shard_id)

    def get_leader_for_shard(self, shard_id: int) -> Optional[str]:
        """Get the leader node address for a shard.

        Args:
            shard_id: The shard ID to look up

        Returns:
            Leader node address or None if not found
        """
        shard_info = self._shard_map.get(shard_id)
        if not shard_info:
            return None

        # Find the leader replica
        for replica in shard_info.replicas:
            if replica.leader:
                # Find node address by node_id
                for node_addr, node in self._nodes.items():
                    if node.id == replica.node_id:
                        return node_addr
        return None

    def get_replicas_for_shard(self, shard_id: int) -> List[str]:
        """Get all replica node addresses for a shard.

        Args:
            shard_id: The shard ID to look up

        Returns:
            List of replica node addresses
        """
        shard_info = self._shard_map.get(shard_id)
        if not shard_info:
            return []

        replicas = []
        for replica in shard_info.replicas:
            for node_addr, node in self._nodes.items():
                if node.id == replica.node_id:
                    replicas.append(node_addr)
                    break
        return replicas

    def get_healthy_nodes(self) -> List[str]:
        """Get list of currently healthy node addresses.

        Returns:
            List of healthy node addresses
        """
        return [
            addr
            for addr, health in self._node_health.items()
            if health.is_healthy
        ]

    def get_all_nodes(self) -> List[str]:
        """Get list of all known node addresses.

        Returns:
            List of all node addresses
        """
        return list(self._node_health.keys())

    def mark_node_failure(self, address: str, error: Optional[str] = None) -> None:
        """Mark a node as having failed a request.

        Args:
            address: Node address that failed
            error: Optional error message
        """
        health = self._node_health.get(address)
        if not health:
            return

        health.consecutive_failures += 1
        health.last_error = error
        health.last_check = datetime.now()

        if health.consecutive_failures >= self._max_failures:
            health.is_healthy = False

    def mark_node_success(self, address: str) -> None:
        """Mark a node as having successfully handled a request.

        Args:
            address: Node address that succeeded
        """
        health = self._node_health.get(address)
        if not health:
            return

        health.consecutive_failures = 0
        health.is_healthy = True
        health.last_check = datetime.now()
        health.last_error = None

    def add_node(self, address: str) -> None:
        """Add a new node to track.

        Args:
            address: Node address to add
        """
        if address not in self._node_health:
            self._node_health[address] = NodeHealth(address=address)

    def remove_node(self, address: str) -> None:
        """Remove a node from tracking.

        Args:
            address: Node address to remove
        """
        self._node_health.pop(address, None)

    def select_node_for_shard(
        self, shard_id: int, prefer_leader: bool = True
    ) -> str:
        """Select a node to handle a request for a shard.

        Args:
            shard_id: The shard ID
            prefer_leader: If True, prefer the leader node

        Returns:
            Selected node address

        Raises:
            UnavailableError: If no healthy nodes are available
        """
        # Try to get leader if preferred and available
        if prefer_leader:
            leader = self.get_leader_for_shard(shard_id)
            if leader and self._is_node_healthy(leader):
                return leader

        # Fall back to any healthy replica
        replicas = self.get_replicas_for_shard(shard_id)
        healthy_replicas = [r for r in replicas if self._is_node_healthy(r)]
        if healthy_replicas:
            return healthy_replicas[0]

        # Last resort: any healthy node
        healthy_nodes = self.get_healthy_nodes()
        if healthy_nodes:
            return healthy_nodes[0]

        # No healthy nodes available
        raise UnavailableError(
            f"No healthy nodes available for shard {shard_id}",
            retry_after_ms=1000,
        )

    def _is_node_healthy(self, address: str) -> bool:
        """Check if a node is currently healthy.

        Args:
            address: Node address to check

        Returns:
            True if node is healthy
        """
        health = self._node_health.get(address)
        return health.is_healthy if health else False

    def get_total_shards(self) -> int:
        """Get the total number of virtual shards.

        Returns:
            Total number of shards
        """
        return self._total_shards
