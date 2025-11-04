package com.norikv.client.types;

import java.util.List;

/**
 * Represents a snapshot of the cluster topology at a specific epoch.
 *
 * <p>The cluster view includes all nodes and shard assignments,
 * versioned by an epoch number that increments on topology changes.
 */
public final class ClusterView {
    private final long epoch;
    private final List<ClusterNode> nodes;
    private final List<ShardInfo> shards;

    /**
     * Creates a new ClusterView.
     *
     * @param epoch the epoch number
     * @param nodes the list of cluster nodes
     * @param shards the list of shard assignments
     */
    public ClusterView(long epoch, List<ClusterNode> nodes, List<ShardInfo> shards) {
        if (nodes == null) {
            throw new IllegalArgumentException("nodes cannot be null");
        }
        if (shards == null) {
            throw new IllegalArgumentException("shards cannot be null");
        }

        this.epoch = epoch;
        this.nodes = nodes;
        this.shards = shards;
    }

    /**
     * Gets the epoch number.
     *
     * @return the epoch
     */
    public long getEpoch() {
        return epoch;
    }

    /**
     * Gets the list of cluster nodes.
     *
     * @return the nodes
     */
    public List<ClusterNode> getNodes() {
        return nodes;
    }

    /**
     * Gets the list of shard assignments.
     *
     * @return the shards
     */
    public List<ShardInfo> getShards() {
        return shards;
    }

    @Override
    public String toString() {
        return "ClusterView{" +
                "epoch=" + epoch +
                ", nodes=" + nodes.size() +
                ", shards=" + shards.size() +
                '}';
    }
}
