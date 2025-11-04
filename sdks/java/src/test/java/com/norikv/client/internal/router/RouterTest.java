package com.norikv.client.internal.router;

import com.norikv.client.internal.conn.ConnectionPool;
import com.norikv.client.types.ConnectionException;
import com.norikv.client.types.NotLeaderException;
import io.grpc.ManagedChannel;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DisplayName;
import static org.junit.jupiter.api.Assertions.*;

import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * Test suite for Router.
 */
public class RouterTest {
    private ConnectionPool pool;
    private Router router;
    private List<String> nodes;

    @BeforeEach
    public void setUp() {
        nodes = Arrays.asList("localhost:9001", "localhost:9002", "localhost:9003");
        pool = new ConnectionPool(1);
        router = new Router(nodes, 1024, pool);
    }

    @AfterEach
    public void tearDown() {
        if (pool != null) {
            pool.close();
        }
    }

    @Test
    @DisplayName("Router: constructor validates arguments")
    public void testConstructorValidation() {
        assertThrows(IllegalArgumentException.class,
                () -> new Router(null, 1024, pool));
        assertThrows(IllegalArgumentException.class,
                () -> new Router(Arrays.asList(), 1024, pool));
        assertThrows(IllegalArgumentException.class,
                () -> new Router(nodes, 0, pool));
        assertThrows(IllegalArgumentException.class,
                () -> new Router(nodes, -1, pool));
        assertThrows(IllegalArgumentException.class,
                () -> new Router(nodes, 1024, null));
    }

    @Test
    @DisplayName("Router: getShardForKey uses hash functions")
    public void testGetShardForKey() {
        // Test known mappings from hash function tests
        assertEquals(309, router.getShardForKey("hello".getBytes()));
        assertEquals(752, router.getShardForKey("world".getBytes()));
        assertEquals(928, router.getShardForKey("user:123".getBytes()));
    }

    @Test
    @DisplayName("Router: getShardForKey is deterministic")
    public void testGetShardForKeyDeterministic() {
        byte[] key = "test-key".getBytes();
        int shard1 = router.getShardForKey(key);
        int shard2 = router.getShardForKey(key);
        int shard3 = router.getShardForKey(key);

        assertEquals(shard1, shard2);
        assertEquals(shard2, shard3);
    }

    @Test
    @DisplayName("Router: getShardForKey returns valid shard IDs")
    public void testGetShardForKeyRange() {
        for (int i = 0; i < 100; i++) {
            byte[] key = ("key-" + i).getBytes();
            int shard = router.getShardForKey(key);
            assertTrue(shard >= 0 && shard < 1024,
                    "Shard " + shard + " out of range [0, 1024)");
        }
    }

    @Test
    @DisplayName("Router: getChannelForShard uses fallback when no leader cached")
    public void testGetChannelForShardFallback() throws ConnectionException {
        Router.RouteInfo route = router.getChannelForShard(0);

        assertNotNull(route);
        assertNotNull(route.getChannel());
        assertNotNull(route.getNodeAddress());
        assertEquals(0, route.getShardId());
        assertFalse(route.isLeaderCached(), "Should use fallback when no leader cached");
        assertTrue(nodes.contains(route.getNodeAddress()),
                "Should use one of the configured nodes");
    }

    @Test
    @DisplayName("Router: getChannelForShard uses cached leader")
    public void testGetChannelForShardCached() throws ConnectionException {
        // Cache a leader
        router.updateLeader(0, "localhost:9002");

        Router.RouteInfo route = router.getChannelForShard(0);

        assertNotNull(route);
        assertEquals("localhost:9002", route.getNodeAddress());
        assertEquals(0, route.getShardId());
        assertTrue(route.isLeaderCached(), "Should use cached leader");
    }

    @Test
    @DisplayName("Router: getChannelForKey computes shard and routes")
    public void testGetChannelForKey() throws ConnectionException {
        byte[] key = "hello".getBytes();
        Router.RouteInfo route = router.getChannelForKey(key);

        assertNotNull(route);
        assertEquals(309, route.getShardId(), "Should route 'hello' to shard 309");
        assertFalse(route.isLeaderCached());
    }

    @Test
    @DisplayName("Router: updateLeader caches leader address")
    public void testUpdateLeader() {
        assertNull(router.getCachedLeader(5));
        assertFalse(router.hasLeaderCached(5));

        router.updateLeader(5, "localhost:9003");

        assertEquals("localhost:9003", router.getCachedLeader(5));
        assertTrue(router.hasLeaderCached(5));
        assertEquals(1, router.getCachedLeaderCount());
    }

    @Test
    @DisplayName("Router: updateLeader ignores null or empty addresses")
    public void testUpdateLeaderIgnoresInvalid() {
        router.updateLeader(5, null);
        assertNull(router.getCachedLeader(5));

        router.updateLeader(5, "");
        assertNull(router.getCachedLeader(5));
    }

    @Test
    @DisplayName("Router: updateLeader replaces existing cache entry")
    public void testUpdateLeaderReplaces() {
        router.updateLeader(5, "localhost:9001");
        assertEquals("localhost:9001", router.getCachedLeader(5));

        router.updateLeader(5, "localhost:9002");
        assertEquals("localhost:9002", router.getCachedLeader(5));
        assertEquals(1, router.getCachedLeaderCount());
    }

    @Test
    @DisplayName("Router: handleNotLeader updates cache with hint")
    public void testHandleNotLeaderWithHint() {
        NotLeaderException error = new NotLeaderException(
                "Not leader", "localhost:9003", 10);

        router.handleNotLeader(error);

        assertEquals("localhost:9003", router.getCachedLeader(10));
    }

    @Test
    @DisplayName("Router: handleNotLeader invalidates cache without hint")
    public void testHandleNotLeaderWithoutHint() {
        // Cache a leader
        router.updateLeader(10, "localhost:9001");
        assertTrue(router.hasLeaderCached(10));

        // Receive NotLeaderException without hint
        NotLeaderException error = new NotLeaderException(
                "Not leader", null, 10);

        router.handleNotLeader(error);

        assertFalse(router.hasLeaderCached(10));
    }

    @Test
    @DisplayName("Router: invalidateLeader removes cache entry")
    public void testInvalidateLeader() {
        router.updateLeader(5, "localhost:9001");
        assertTrue(router.hasLeaderCached(5));

        router.invalidateLeader(5);

        assertFalse(router.hasLeaderCached(5));
        assertNull(router.getCachedLeader(5));
    }

    @Test
    @DisplayName("Router: invalidateAllLeaders clears entire cache")
    public void testInvalidateAllLeaders() {
        router.updateLeader(1, "localhost:9001");
        router.updateLeader(2, "localhost:9002");
        router.updateLeader(3, "localhost:9003");
        assertEquals(3, router.getCachedLeaderCount());

        router.invalidateAllLeaders();

        assertEquals(0, router.getCachedLeaderCount());
        assertNull(router.getCachedLeader(1));
        assertNull(router.getCachedLeader(2));
        assertNull(router.getCachedLeader(3));
    }

    @Test
    @DisplayName("Router: fallback uses round-robin")
    public void testFallbackRoundRobin() throws ConnectionException {
        Set<String> usedNodes = new HashSet<>();

        // Request routes for multiple shards without cached leaders
        for (int i = 0; i < 10; i++) {
            Router.RouteInfo route = router.getChannelForShard(i);
            usedNodes.add(route.getNodeAddress());
        }

        // Should have used multiple nodes (round-robin)
        assertTrue(usedNodes.size() >= 2,
                "Should distribute across multiple nodes: " + usedNodes);
    }

    @Test
    @DisplayName("Router: getStats returns accurate statistics")
    public void testGetStats() {
        Router.RouterStats stats = router.getStats();

        assertEquals(3, stats.getTotalNodes());
        assertEquals(1024, stats.getTotalShards());
        assertEquals(0, stats.getCachedLeaders());
        assertEquals(0.0, stats.getCacheHitRatio(), 0.001);

        // Cache some leaders
        router.updateLeader(1, "localhost:9001");
        router.updateLeader(2, "localhost:9002");

        stats = router.getStats();
        assertEquals(2, stats.getCachedLeaders());
        assertEquals(2.0 / 1024, stats.getCacheHitRatio(), 0.001);
    }

    @Test
    @DisplayName("Router: getNodes returns configured nodes")
    public void testGetNodes() {
        List<String> returnedNodes = router.getNodes();
        assertEquals(nodes, returnedNodes);
    }

    @Test
    @DisplayName("Router: getTotalShards returns configured shards")
    public void testGetTotalShards() {
        assertEquals(1024, router.getTotalShards());
    }

    @Test
    @DisplayName("Router: concurrent leader cache updates")
    public void testConcurrentCacheUpdates() throws InterruptedException {
        int numThreads = 10;
        int numShards = 100;
        Thread[] threads = new Thread[numThreads];

        for (int i = 0; i < numThreads; i++) {
            final int threadId = i;
            threads[i] = new Thread(() -> {
                for (int shard = 0; shard < numShards; shard++) {
                    String leader = "localhost:900" + (threadId % 3 + 1);
                    router.updateLeader(shard, leader);
                }
            });
            threads[i].start();
        }

        for (Thread thread : threads) {
            thread.join();
        }

        // Should have cached all shards without errors
        assertEquals(numShards, router.getCachedLeaderCount());
    }

    @Test
    @DisplayName("Router: concurrent reads and writes")
    public void testConcurrentReadsAndWrites() throws InterruptedException {
        int numReaders = 5;
        int numWriters = 5;
        Thread[] threads = new Thread[numReaders + numWriters];

        // Writer threads
        for (int i = 0; i < numWriters; i++) {
            final int writerId = i;
            threads[i] = new Thread(() -> {
                for (int j = 0; j < 100; j++) {
                    int shard = (writerId * 100 + j) % 1024;
                    router.updateLeader(shard, "localhost:9001");
                }
            });
            threads[i].start();
        }

        // Reader threads
        for (int i = 0; i < numReaders; i++) {
            final int readerId = i;
            threads[numWriters + i] = new Thread(() -> {
                for (int j = 0; j < 100; j++) {
                    int shard = (readerId * 100 + j) % 1024;
                    router.getCachedLeader(shard);
                    router.hasLeaderCached(shard);
                }
            });
            threads[numWriters + i].start();
        }

        for (Thread thread : threads) {
            thread.join();
        }

        // All operations should complete without exceptions
        assertTrue(router.getCachedLeaderCount() > 0);
    }

    @Test
    @DisplayName("Router: RouteInfo toString")
    public void testRouteInfoToString() throws ConnectionException {
        router.updateLeader(5, "localhost:9001");
        Router.RouteInfo route = router.getChannelForShard(5);

        String str = route.toString();
        assertTrue(str.contains("localhost:9001"));
        assertTrue(str.contains("shardId=5"));
        assertTrue(str.contains("leaderCached=true"));
    }

    @Test
    @DisplayName("Router: RouterStats toString")
    public void testRouterStatsToString() {
        router.updateLeader(1, "localhost:9001");
        Router.RouterStats stats = router.getStats();

        String str = stats.toString();
        assertTrue(str.contains("totalNodes=3"));
        assertTrue(str.contains("totalShards=1024"));
        assertTrue(str.contains("cachedLeaders=1"));
        assertTrue(str.contains("cacheHitRatio="));
    }

    @Test
    @DisplayName("Router: cache hit ratio calculation")
    public void testCacheHitRatio() {
        Router.RouterStats stats = router.getStats();
        assertEquals(0.0, stats.getCacheHitRatio(), 0.001);

        // Cache 512 leaders (half of shards)
        for (int i = 0; i < 512; i++) {
            router.updateLeader(i, "localhost:9001");
        }

        stats = router.getStats();
        assertEquals(0.5, stats.getCacheHitRatio(), 0.001);

        // Cache all shards
        for (int i = 512; i < 1024; i++) {
            router.updateLeader(i, "localhost:9001");
        }

        stats = router.getStats();
        assertEquals(1.0, stats.getCacheHitRatio(), 0.001);
    }
}
