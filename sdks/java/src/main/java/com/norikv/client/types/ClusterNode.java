package com.norikv.client.types;

/**
 * Represents a node in the NoriKV cluster.
 */
public final class ClusterNode {
    private final String id;
    private final String addr;

    /**
     * Creates a new ClusterNode.
     *
     * @param id the unique node ID
     * @param addr the node address (host:port)
     */
    public ClusterNode(String id, String addr) {
        if (id == null || id.isEmpty()) {
            throw new IllegalArgumentException("id cannot be null or empty");
        }
        if (addr == null || addr.isEmpty()) {
            throw new IllegalArgumentException("addr cannot be null or empty");
        }

        this.id = id;
        this.addr = addr;
    }

    /**
     * Gets the node ID.
     *
     * @return the node ID
     */
    public String getId() {
        return id;
    }

    /**
     * Gets the node address.
     *
     * @return the node address
     */
    public String getAddr() {
        return addr;
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        ClusterNode that = (ClusterNode) o;
        return id.equals(that.id) && addr.equals(that.addr);
    }

    @Override
    public int hashCode() {
        return id.hashCode() * 31 + addr.hashCode();
    }

    @Override
    public String toString() {
        return "ClusterNode{" +
                "id='" + id + '\'' +
                ", addr='" + addr + '\'' +
                '}';
    }
}
