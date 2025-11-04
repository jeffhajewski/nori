package com.norikv.client.types;

import java.util.List;

/**
 * Information about a shard and its replica assignments.
 */
public final class ShardInfo {
    private final int id;
    private final List<ShardReplica> replicas;

    /**
     * Creates a new ShardInfo.
     *
     * @param id the shard ID
     * @param replicas the list of replicas for this shard
     */
    public ShardInfo(int id, List<ShardReplica> replicas) {
        if (replicas == null) {
            throw new IllegalArgumentException("replicas cannot be null");
        }

        this.id = id;
        this.replicas = replicas;
    }

    /**
     * Gets the shard ID.
     *
     * @return the shard ID
     */
    public int getId() {
        return id;
    }

    /**
     * Gets the list of replicas.
     *
     * @return the replicas
     */
    public List<ShardReplica> getReplicas() {
        return replicas;
    }

    @Override
    public String toString() {
        return "ShardInfo{" +
                "id=" + id +
                ", replicas=" + replicas.size() +
                '}';
    }
}
