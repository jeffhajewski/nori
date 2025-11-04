package com.norikv.client.internal.router;

import com.norikv.client.hash.HashFunctions;
import com.norikv.client.internal.conn.ConnectionPool;
import com.norikv.client.types.ConnectionException;
import com.norikv.client.types.NotLeaderException;
import io.grpc.ManagedChannel;

import java.util.List;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.locks.ReadWriteLock;
import java.util.concurrent.locks.ReentrantReadWriteLock;

/**
 * Routes requests to the correct shard leader with caching.
 *
 * <p>This class provides smart routing by:
 * <ul>
 * <li>Computing shard assignments using consistent hashing</li>
 * <li>Caching leader information for each shard</li>
 * <li>Falling back to random node on cache miss</li>
 * <li>Updating cache on NotLeaderException hints</li>
 * </ul>
 *
 * <p>Thread-safe for concurrent use.
 */
public final class Router {
    private final List<String> nodes;
    private final int totalShards;
    private final ConnectionPool connectionPool;

    // Leader cache: shard_id -> node_address
    private final ConcurrentHashMap<Integer, String> leaderCache;

    // Lock for coordinating cache updates
    private final ReadWriteLock cacheLock;

    // Round-robin index for fallback node selection
    private int fallbackIndex;

    /**
     * Creates a new Router.
     *
     * @param nodes list of cluster node addresses
     * @param totalShards total number of shards in the cluster
     * @param connectionPool connection pool for gRPC channels
     */
    public Router(List<String> nodes, int totalShards, ConnectionPool connectionPool) {
        if (nodes == null || nodes.isEmpty()) {
            throw new IllegalArgumentException("nodes cannot be null or empty");
        }
        if (totalShards <= 0) {
            throw new IllegalArgumentException("totalShards must be > 0");
        }
        if (connectionPool == null) {
            throw new IllegalArgumentException("connectionPool cannot be null");
        }

        this.nodes = nodes;
        this.totalShards = totalShards;
        this.connectionPool = connectionPool;
        this.leaderCache = new ConcurrentHashMap<>();
        this.cacheLock = new ReentrantReadWriteLock();
        this.fallbackIndex = 0;
    }

    /**
     * Gets the shard ID for a given key.
     *
     * <p>Uses xxhash64 + jump consistent hash to deterministically map the key to a shard.
     *
     * @param key the key
     * @return the shard ID (0-based)
     */
    public int getShardForKey(byte[] key) {
        return HashFunctions.getShardForKey(key, totalShards);
    }

    /**
     * Gets a channel to the leader for the given shard.
     *
     * <p>Attempts to use cached leader information. If no leader is cached,
     * falls back to a random node from the cluster.
     *
     * @param shardId the shard ID
     * @return routing info containing channel and whether leader was cached
     * @throws ConnectionException if unable to get channel
     */
    public RouteInfo getChannelForShard(int shardId) throws ConnectionException {
        // Try cached leader first
        String cachedLeader = leaderCache.get(shardId);
        if (cachedLeader != null) {
            ManagedChannel channel = connectionPool.getChannel(cachedLeader);
            return new RouteInfo(channel, cachedLeader, shardId, true);
        }

        // No cached leader, use fallback node
        String fallbackNode = getFallbackNode();
        ManagedChannel channel = connectionPool.getChannel(fallbackNode);
        return new RouteInfo(channel, fallbackNode, shardId, false);
    }

    /**
     * Gets a channel to the leader for the given key.
     *
     * <p>First computes the shard ID from the key, then gets the channel for that shard.
     *
     * @param key the key
     * @return routing info containing channel and shard information
     * @throws ConnectionException if unable to get channel
     */
    public RouteInfo getChannelForKey(byte[] key) throws ConnectionException {
        int shardId = getShardForKey(key);
        return getChannelForShard(shardId);
    }

    /**
     * Updates the cached leader for a shard.
     *
     * <p>Called when receiving NotLeaderException with a leader hint.
     *
     * @param shardId the shard ID
     * @param leaderAddress the leader node address
     */
    public void updateLeader(int shardId, String leaderAddress) {
        if (leaderAddress == null || leaderAddress.isEmpty()) {
            return;
        }

        cacheLock.writeLock().lock();
        try {
            leaderCache.put(shardId, leaderAddress);
        } finally {
            cacheLock.writeLock().unlock();
        }
    }

    /**
     * Handles a NotLeaderException by updating the leader cache.
     *
     * <p>Extracts the leader hint from the exception and updates the cache.
     *
     * @param error the NotLeaderException
     */
    public void handleNotLeader(NotLeaderException error) {
        String leaderHint = error.getLeaderHint();
        int shardId = error.getShardId();

        if (leaderHint != null && !leaderHint.isEmpty()) {
            updateLeader(shardId, leaderHint);
        } else {
            // No hint provided, invalidate cache entry
            invalidateLeader(shardId);
        }
    }

    /**
     * Invalidates the cached leader for a shard.
     *
     * <p>Called when we receive NotLeaderException without a hint,
     * or when we suspect the cached leader is stale.
     *
     * @param shardId the shard ID
     */
    public void invalidateLeader(int shardId) {
        cacheLock.writeLock().lock();
        try {
            leaderCache.remove(shardId);
        } finally {
            cacheLock.writeLock().unlock();
        }
    }

    /**
     * Invalidates all cached leader information.
     *
     * <p>Useful when the cluster topology changes significantly.
     */
    public void invalidateAllLeaders() {
        cacheLock.writeLock().lock();
        try {
            leaderCache.clear();
        } finally {
            cacheLock.writeLock().unlock();
        }
    }

    /**
     * Gets the cached leader for a shard, if any.
     *
     * @param shardId the shard ID
     * @return the leader address, or null if not cached
     */
    public String getCachedLeader(int shardId) {
        return leaderCache.get(shardId);
    }

    /**
     * Checks if a leader is cached for the given shard.
     *
     * @param shardId the shard ID
     * @return true if a leader is cached
     */
    public boolean hasLeaderCached(int shardId) {
        return leaderCache.containsKey(shardId);
    }

    /**
     * Gets the number of cached leaders.
     *
     * @return the number of cached leaders
     */
    public int getCachedLeaderCount() {
        return leaderCache.size();
    }

    /**
     * Gets a fallback node using round-robin selection.
     *
     * <p>Used when no leader is cached for a shard.
     *
     * @return a node address
     */
    private synchronized String getFallbackNode() {
        String node = nodes.get(fallbackIndex % nodes.size());
        fallbackIndex++;
        return node;
    }

    /**
     * Gets the list of cluster nodes.
     *
     * @return the node addresses
     */
    public List<String> getNodes() {
        return nodes;
    }

    /**
     * Gets the total number of shards.
     *
     * @return the total shards
     */
    public int getTotalShards() {
        return totalShards;
    }

    /**
     * Gets router statistics.
     *
     * @return router statistics
     */
    public RouterStats getStats() {
        return new RouterStats(
                nodes.size(),
                totalShards,
                leaderCache.size()
        );
    }

    /**
     * Information about a route to a shard.
     */
    public static final class RouteInfo {
        private final ManagedChannel channel;
        private final String nodeAddress;
        private final int shardId;
        private final boolean leaderCached;

        RouteInfo(ManagedChannel channel, String nodeAddress, int shardId, boolean leaderCached) {
            this.channel = channel;
            this.nodeAddress = nodeAddress;
            this.shardId = shardId;
            this.leaderCached = leaderCached;
        }

        /**
         * Gets the gRPC channel for this route.
         *
         * @return the managed channel
         */
        public ManagedChannel getChannel() {
            return channel;
        }

        /**
         * Gets the node address for this route.
         *
         * @return the node address
         */
        public String getNodeAddress() {
            return nodeAddress;
        }

        /**
         * Gets the shard ID for this route.
         *
         * @return the shard ID
         */
        public int getShardId() {
            return shardId;
        }

        /**
         * Checks if the leader was cached.
         *
         * @return true if leader was cached, false if using fallback
         */
        public boolean isLeaderCached() {
            return leaderCached;
        }

        @Override
        public String toString() {
            return "RouteInfo{" +
                    "nodeAddress='" + nodeAddress + '\'' +
                    ", shardId=" + shardId +
                    ", leaderCached=" + leaderCached +
                    '}';
        }
    }

    /**
     * Statistics about the router.
     */
    public static final class RouterStats {
        private final int totalNodes;
        private final int totalShards;
        private final int cachedLeaders;

        RouterStats(int totalNodes, int totalShards, int cachedLeaders) {
            this.totalNodes = totalNodes;
            this.totalShards = totalShards;
            this.cachedLeaders = cachedLeaders;
        }

        public int getTotalNodes() {
            return totalNodes;
        }

        public int getTotalShards() {
            return totalShards;
        }

        public int getCachedLeaders() {
            return cachedLeaders;
        }

        /**
         * Gets the cache hit ratio (cached leaders / total shards).
         *
         * @return cache hit ratio (0.0 to 1.0)
         */
        public double getCacheHitRatio() {
            return totalShards > 0 ? (double) cachedLeaders / totalShards : 0.0;
        }

        @Override
        public String toString() {
            return "RouterStats{" +
                    "totalNodes=" + totalNodes +
                    ", totalShards=" + totalShards +
                    ", cachedLeaders=" + cachedLeaders +
                    ", cacheHitRatio=" + String.format("%.2f", getCacheHitRatio()) +
                    '}';
        }
    }
}
