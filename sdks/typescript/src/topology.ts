/**
 * Cluster topology tracking and management.
 *
 * This module handles:
 * - Watching cluster view changes
 * - Tracking shard leader assignments
 * - Providing routing information for keys
 * - Handling cluster membership updates
 */

import type {
  ClusterView,
  ClusterNode,
  ShardInfo,
  ShardReplica,
  RouteInfo,
} from './types.js';

/**
 * Event emitted when cluster view changes.
 */
export interface TopologyChangeEvent {
  /** Previous epoch (or null if first view) */
  previousEpoch: bigint | null;

  /** New epoch */
  currentEpoch: bigint;

  /** Nodes that were added */
  addedNodes: string[];

  /** Nodes that were removed */
  removedNodes: string[];

  /** Shards whose leaders changed */
  leaderChanges: Map<number, { old: string | null; new: string | null }>;
}

/**
 * Manages cluster topology information.
 */
export class TopologyManager {
  private view: ClusterView | null = null;
  private shardLeaderCache = new Map<number, string | null>();
  private nodeByIdCache = new Map<string, ClusterNode>();
  private listeners: Array<(event: TopologyChangeEvent) => void> = [];

  /**
   * Update the cluster view.
   *
   * @param newView - New cluster view from server
   * @returns Event describing changes, or null if no changes
   */
  updateView(newView: ClusterView): TopologyChangeEvent | null {
    const oldView = this.view;

    // Check if this is actually a newer view
    if (oldView && newView.epoch <= oldView.epoch) {
      return null; // Stale or duplicate view
    }

    this.view = newView;

    // Rebuild caches
    this.rebuildCaches(newView);

    // Compute changes
    const event = this.computeChanges(oldView, newView);

    // Notify listeners
    for (const listener of this.listeners) {
      try {
        listener(event);
      } catch (err) {
        console.error('Topology listener error:', err);
      }
    }

    return event;
  }

  /**
   * Rebuild internal caches from cluster view.
   */
  private rebuildCaches(view: ClusterView): void {
    // Build node cache
    this.nodeByIdCache.clear();
    for (const node of view.nodes) {
      this.nodeByIdCache.set(node.id, node);
    }

    // Build shard leader cache
    this.shardLeaderCache.clear();
    for (const shard of view.shards) {
      const leader = shard.replicas.find((r) => r.leader);
      if (leader) {
        const leaderNode = this.nodeByIdCache.get(leader.nodeId);
        if (leaderNode) {
          this.shardLeaderCache.set(shard.id, leaderNode.addr);
        }
      } else {
        this.shardLeaderCache.set(shard.id, null);
      }
    }
  }

  /**
   * Compute changes between two cluster views.
   */
  private computeChanges(
    oldView: ClusterView | null,
    newView: ClusterView
  ): TopologyChangeEvent {
    const addedNodes: string[] = [];
    const removedNodes: string[] = [];
    const leaderChanges = new Map<number, { old: string | null; new: string | null }>();

    if (!oldView) {
      // First view - all nodes are "added"
      addedNodes.push(...newView.nodes.map((n) => n.addr));

      return {
        previousEpoch: null,
        currentEpoch: newView.epoch,
        addedNodes,
        removedNodes,
        leaderChanges,
      };
    }

    // Track node changes
    const oldNodeAddrs = new Set(oldView.nodes.map((n) => n.addr));
    const newNodeAddrs = new Set(newView.nodes.map((n) => n.addr));

    for (const addr of newNodeAddrs) {
      if (!oldNodeAddrs.has(addr)) {
        addedNodes.push(addr);
      }
    }

    for (const addr of oldNodeAddrs) {
      if (!newNodeAddrs.has(addr)) {
        removedNodes.push(addr);
      }
    }

    // Track leader changes
    const oldShardLeaders = new Map<number, string | null>();
    for (const shard of oldView.shards) {
      const leader = shard.replicas.find((r) => r.leader);
      if (leader) {
        const leaderNode = oldView.nodes.find((n) => n.id === leader.nodeId);
        oldShardLeaders.set(shard.id, leaderNode?.addr ?? null);
      } else {
        oldShardLeaders.set(shard.id, null);
      }
    }

    for (const shard of newView.shards) {
      const oldLeader = oldShardLeaders.get(shard.id) ?? null;
      const leader = shard.replicas.find((r) => r.leader);
      const newLeader = leader
        ? newView.nodes.find((n) => n.id === leader.nodeId)?.addr ?? null
        : null;

      if (oldLeader !== newLeader) {
        leaderChanges.set(shard.id, { old: oldLeader, new: newLeader });
      }
    }

    return {
      previousEpoch: oldView.epoch,
      currentEpoch: newView.epoch,
      addedNodes,
      removedNodes,
      leaderChanges,
    };
  }

  /**
   * Get current cluster view.
   */
  getView(): ClusterView | null {
    return this.view;
  }

  /**
   * Get current epoch.
   */
  getEpoch(): bigint | null {
    return this.view?.epoch ?? null;
  }

  /**
   * Get leader address for a shard.
   *
   * @param shardId - Shard ID
   * @returns Leader address, or null if no leader
   */
  getShardLeader(shardId: number): string | null {
    return this.shardLeaderCache.get(shardId) ?? null;
  }

  /**
   * Update leader hint for a shard (from NOT_LEADER error).
   *
   * This is an optimization to avoid waiting for the next cluster view update.
   *
   * @param shardId - Shard ID
   * @param leaderAddr - Leader address from error metadata
   */
  updateLeaderHint(shardId: number, leaderAddr: string): void {
    this.shardLeaderCache.set(shardId, leaderAddr);
  }

  /**
   * Get routing information for a shard.
   *
   * @param shardId - Shard ID
   * @returns Routing info, or null if shard not found
   */
  getRouteInfo(shardId: number): RouteInfo | null {
    if (!this.view) return null;

    const shard = this.view.shards.find((s) => s.id === shardId);
    if (!shard) return null;

    const leaderAddr = this.shardLeaderCache.get(shardId) ?? null;

    const replicaAddrs: string[] = [];
    for (const replica of shard.replicas) {
      const node = this.nodeByIdCache.get(replica.nodeId);
      if (node) {
        replicaAddrs.push(node.addr);
      }
    }

    return {
      shardId,
      leaderAddr,
      replicaAddrs,
    };
  }

  /**
   * Get all node addresses.
   */
  getNodeAddresses(): string[] {
    if (!this.view) return [];
    return this.view.nodes.map((n) => n.addr);
  }

  /**
   * Get a node by address.
   */
  getNodeByAddress(address: string): ClusterNode | null {
    if (!this.view) return null;
    return this.view.nodes.find((n) => n.addr === address) ?? null;
  }

  /**
   * Get all shards and their replica assignments.
   */
  getShards(): ShardInfo[] {
    return this.view?.shards ?? [];
  }

  /**
   * Get shard info by ID.
   */
  getShard(shardId: number): ShardInfo | null {
    if (!this.view) return null;
    return this.view.shards.find((s) => s.id === shardId) ?? null;
  }

  /**
   * Register a listener for topology changes.
   *
   * @param listener - Callback to invoke on topology changes
   * @returns Unsubscribe function
   */
  onTopologyChange(listener: (event: TopologyChangeEvent) => void): () => void {
    this.listeners.push(listener);
    return () => {
      const index = this.listeners.indexOf(listener);
      if (index >= 0) {
        this.listeners.splice(index, 1);
      }
    };
  }

  /**
   * Check if topology is initialized.
   */
  isInitialized(): boolean {
    return this.view !== null;
  }

  /**
   * Clear all cached topology information.
   */
  clear(): void {
    this.view = null;
    this.shardLeaderCache.clear();
    this.nodeByIdCache.clear();
  }
}
