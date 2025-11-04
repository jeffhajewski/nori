package com.norikv.client.internal.conn;

import com.norikv.client.types.ConnectionException;
import io.grpc.ManagedChannel;
import io.grpc.ManagedChannelBuilder;

import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * Manages a pool of gRPC channels to cluster nodes.
 *
 * <p>This class maintains persistent connections to NoriKV cluster nodes,
 * providing:
 * <ul>
 * <li>Connection pooling and reuse</li>
 * <li>Automatic connection management</li>
 * <li>Thread-safe concurrent access</li>
 * <li>Proper cleanup on shutdown</li>
 * </ul>
 *
 * <p>Thread-safe for concurrent use.
 */
public final class ConnectionPool implements AutoCloseable {
    private final ConcurrentHashMap<String, ManagedChannel> channels;
    private final int maxConnectionsPerNode;
    private final AtomicBoolean closed;

    /**
     * Creates a new ConnectionPool.
     *
     * @param maxConnectionsPerNode maximum connections per node (currently creates 1 channel per node)
     */
    public ConnectionPool(int maxConnectionsPerNode) {
        if (maxConnectionsPerNode <= 0) {
            throw new IllegalArgumentException("maxConnectionsPerNode must be > 0");
        }
        this.channels = new ConcurrentHashMap<>();
        this.maxConnectionsPerNode = maxConnectionsPerNode;
        this.closed = new AtomicBoolean(false);
    }

    /**
     * Gets or creates a channel to the specified address.
     *
     * <p>Channels are cached and reused for subsequent requests to the same address.
     *
     * @param address the node address (host:port)
     * @return the managed channel
     * @throws ConnectionException if the pool is closed
     */
    public ManagedChannel getChannel(String address) throws ConnectionException {
        if (closed.get()) {
            throw new ConnectionException("Connection pool is closed", address);
        }

        if (address == null || address.isEmpty()) {
            throw new IllegalArgumentException("address cannot be null or empty");
        }

        // Get or create channel (thread-safe)
        return channels.computeIfAbsent(address, this::createChannel);
    }

    /**
     * Creates a new gRPC channel to the specified address.
     *
     * @param address the node address (host:port)
     * @return the new managed channel
     */
    private ManagedChannel createChannel(String address) {
        return ManagedChannelBuilder.forTarget(address)
                .usePlaintext() // TODO: Add TLS support
                .keepAliveTime(30, TimeUnit.SECONDS)
                .keepAliveTimeout(10, TimeUnit.SECONDS)
                .keepAliveWithoutCalls(true)
                .maxInboundMessageSize(4 * 1024 * 1024) // 4MB max message size
                .build();
    }

    /**
     * Removes and closes the channel for the specified address.
     *
     * <p>This is useful when a node becomes unavailable or is removed from the cluster.
     *
     * @param address the node address
     * @return true if a channel was removed
     */
    public boolean removeChannel(String address) {
        ManagedChannel channel = channels.remove(address);
        if (channel != null) {
            shutdownChannel(channel);
            return true;
        }
        return false;
    }

    /**
     * Checks if a channel exists for the specified address.
     *
     * @param address the node address
     * @return true if a channel exists
     */
    public boolean hasChannel(String address) {
        return channels.containsKey(address);
    }

    /**
     * Gets the number of active channels.
     *
     * @return the number of channels
     */
    public int size() {
        return channels.size();
    }

    /**
     * Checks if the pool is closed.
     *
     * @return true if the pool is closed
     */
    public boolean isClosed() {
        return closed.get();
    }

    /**
     * Closes all channels and shuts down the pool.
     *
     * <p>This method is idempotent - calling it multiple times has no additional effect.
     */
    @Override
    public void close() {
        if (closed.compareAndSet(false, true)) {
            // Shutdown all channels
            for (ManagedChannel channel : channels.values()) {
                shutdownChannel(channel);
            }
            channels.clear();
        }
    }

    /**
     * Gracefully shuts down a channel.
     *
     * @param channel the channel to shutdown
     */
    private void shutdownChannel(ManagedChannel channel) {
        try {
            channel.shutdown();
            // Wait for termination with timeout
            if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                // Force shutdown if graceful shutdown times out
                channel.shutdownNow();
                // Wait again with shorter timeout
                if (!channel.awaitTermination(2, TimeUnit.SECONDS)) {
                    // Log warning but continue (don't block indefinitely)
                    System.err.println("Warning: Channel did not terminate cleanly");
                }
            }
        } catch (InterruptedException e) {
            // Restore interrupt status
            Thread.currentThread().interrupt();
            // Force shutdown
            channel.shutdownNow();
        }
    }

    /**
     * Gets statistics about the connection pool.
     *
     * @return pool statistics
     */
    public PoolStats getStats() {
        return new PoolStats(
                channels.size(),
                maxConnectionsPerNode,
                closed.get()
        );
    }

    /**
     * Statistics about the connection pool.
     */
    public static final class PoolStats {
        private final int activeConnections;
        private final int maxConnectionsPerNode;
        private final boolean closed;

        PoolStats(int activeConnections, int maxConnectionsPerNode, boolean closed) {
            this.activeConnections = activeConnections;
            this.maxConnectionsPerNode = maxConnectionsPerNode;
            this.closed = closed;
        }

        public int getActiveConnections() {
            return activeConnections;
        }

        public int getMaxConnectionsPerNode() {
            return maxConnectionsPerNode;
        }

        public boolean isClosed() {
            return closed;
        }

        @Override
        public String toString() {
            return "PoolStats{" +
                    "activeConnections=" + activeConnections +
                    ", maxConnectionsPerNode=" + maxConnectionsPerNode +
                    ", closed=" + closed +
                    '}';
        }
    }
}
