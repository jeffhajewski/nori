/**
 * Smart key routing for NoriKV client.
 *
 * This module handles:
 * - Computing shard assignments for keys (using xxhash64 + jump consistent hash)
 * - Finding the correct leader node for a shard
 * - Fallback routing when leader is unknown
 * - Handling NOT_LEADER redirects
 */

import { getShardForKey } from './hash.js';
import type { TopologyManager } from './topology.js';
import type { Key, RouteInfo } from './types.js';
import { NoNodesAvailableError } from './errors.js';

/**
 * Routing decision for a key.
 */
export interface RoutingDecision {
  /** Shard ID this key belongs to */
  shardId: number;

  /** Primary target node address (usually the leader) */
  primaryAddr: string;

  /** Fallback addresses to try if primary fails */
  fallbackAddrs: string[];

  /** Whether this is a best-effort guess (leader unknown) */
  isGuess: boolean;
}

/**
 * Router for directing requests to the correct node.
 */
export class Router {
  constructor(
    private readonly topology: TopologyManager,
    private readonly totalShards: number = 1024
  ) {}

  /**
   * Route a key to the appropriate node.
   *
   * @param key - The key to route
   * @param preferLeader - Whether to prefer leader (default: true for writes)
   * @returns Routing decision
   * @throws NoNodesAvailableError if cluster is not initialized
   */
  route(key: Key, preferLeader: boolean = true): RoutingDecision {
    // Compute shard for key
    const shardId = getShardForKey(key, this.totalShards);

    // Get routing info from topology
    const routeInfo = this.topology.getRouteInfo(shardId);

    if (!routeInfo) {
      // Topology not initialized - fall back to any node
      const allNodes = this.topology.getNodeAddresses();
      if (allNodes.length === 0) {
        throw new NoNodesAvailableError();
      }

      // Round-robin or random selection
      const primaryAddr = allNodes[shardId % allNodes.length];
      const fallbackAddrs = allNodes.filter((addr) => addr !== primaryAddr);

      return {
        shardId,
        primaryAddr,
        fallbackAddrs,
        isGuess: true,
      };
    }

    // If leader is known and we prefer leader, use it
    if (preferLeader && routeInfo.leaderAddr) {
      const fallbackAddrs = routeInfo.replicaAddrs.filter(
        (addr) => addr !== routeInfo.leaderAddr
      );

      return {
        shardId,
        primaryAddr: routeInfo.leaderAddr,
        fallbackAddrs,
        isGuess: false,
      };
    }

    // Leader unknown or don't care about leader - use any replica
    if (routeInfo.replicaAddrs.length === 0) {
      // No replicas known - fall back to any node
      const allNodes = this.topology.getNodeAddresses();
      if (allNodes.length === 0) {
        throw new NoNodesAvailableError();
      }

      const primaryAddr = allNodes[shardId % allNodes.length];
      const fallbackAddrs = allNodes.filter((addr) => addr !== primaryAddr);

      return {
        shardId,
        primaryAddr,
        fallbackAddrs,
        isGuess: true,
      };
    }

    // Use first replica as primary, rest as fallbacks
    const primaryAddr = routeInfo.replicaAddrs[0];
    const fallbackAddrs = routeInfo.replicaAddrs.slice(1);

    return {
      shardId,
      primaryAddr,
      fallbackAddrs,
      isGuess: !preferLeader || !routeInfo.leaderAddr,
    };
  }

  /**
   * Handle a NOT_LEADER redirect by updating leader hint.
   *
   * @param shardId - Shard ID from error
   * @param leaderHint - Leader address from error metadata
   */
  handleNotLeader(shardId: number, leaderHint: string): void {
    this.topology.updateLeaderHint(shardId, leaderHint);
  }

  /**
   * Get all addresses to try for a shard (leader first, then replicas).
   *
   * @param shardId - Shard ID
   * @returns Ordered list of addresses to try
   */
  getShardTargets(shardId: number): string[] {
    const routeInfo = this.topology.getRouteInfo(shardId);

    if (!routeInfo) {
      // No routing info - return all nodes
      return this.topology.getNodeAddresses();
    }

    const targets: string[] = [];

    // Leader first
    if (routeInfo.leaderAddr) {
      targets.push(routeInfo.leaderAddr);
    }

    // Then other replicas
    for (const addr of routeInfo.replicaAddrs) {
      if (addr !== routeInfo.leaderAddr) {
        targets.push(addr);
      }
    }

    // If no targets, fall back to all nodes
    if (targets.length === 0) {
      return this.topology.getNodeAddresses();
    }

    return targets;
  }

  /**
   * Get a random node address for initial discovery.
   *
   * @returns Random node address
   * @throws NoNodesAvailableError if no nodes are available
   */
  getRandomNode(): string {
    const nodes = this.topology.getNodeAddresses();
    if (nodes.length === 0) {
      throw new NoNodesAvailableError();
    }

    return nodes[Math.floor(Math.random() * nodes.length)];
  }

  /**
   * Validate that routing is consistent with server.
   *
   * This is primarily for testing - ensures our hash functions match the server.
   *
   * @param key - Test key
   * @param expectedShardId - Expected shard ID from server
   * @returns Whether routing is consistent
   */
  validateRouting(key: Key, expectedShardId: number): boolean {
    const actualShardId = getShardForKey(key, this.totalShards);
    return actualShardId === expectedShardId;
  }
}
