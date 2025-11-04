package com.norikv.client.types;

/**
 * Represents a replica of a shard on a specific node.
 */
public final class ShardReplica {
    private final String nodeId;
    private final boolean leader;

    /**
     * Creates a new ShardReplica.
     *
     * @param nodeId the ID of the node hosting this replica
     * @param leader whether this replica is the leader
     */
    public ShardReplica(String nodeId, boolean leader) {
        if (nodeId == null || nodeId.isEmpty()) {
            throw new IllegalArgumentException("nodeId cannot be null or empty");
        }

        this.nodeId = nodeId;
        this.leader = leader;
    }

    /**
     * Gets the node ID.
     *
     * @return the node ID
     */
    public String getNodeId() {
        return nodeId;
    }

    /**
     * Checks if this replica is the leader.
     *
     * @return true if this replica is the leader
     */
    public boolean isLeader() {
        return leader;
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        ShardReplica that = (ShardReplica) o;
        return leader == that.leader && nodeId.equals(that.nodeId);
    }

    @Override
    public int hashCode() {
        return nodeId.hashCode() * 31 + (leader ? 1 : 0);
    }

    @Override
    public String toString() {
        return "ShardReplica{" +
                "nodeId='" + nodeId + '\'' +
                ", leader=" + leader +
                '}';
    }
}
