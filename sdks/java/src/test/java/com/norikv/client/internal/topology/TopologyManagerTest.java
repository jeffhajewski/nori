package com.norikv.client.internal.topology;

import com.norikv.client.types.*;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DisplayName;
import static org.junit.jupiter.api.Assertions.*;

import java.util.*;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Test suite for TopologyManager.
 */
public class TopologyManagerTest {
    private TopologyManager manager;

    @BeforeEach
    public void setUp() {
        manager = new TopologyManager();
    }

    @Test
    @DisplayName("TopologyManager: initially not initialized")
    public void testInitiallyNotInitialized() {
        assertFalse(manager.isInitialized());
        assertNull(manager.getView());
        assertNull(manager.getEpoch());
        assertTrue(manager.getNodeAddresses().isEmpty());
        assertTrue(manager.getShards().isEmpty());
    }

    @Test
    @DisplayName("TopologyManager: updateView sets first view")
    public void testUpdateViewFirst() {
        ClusterView view = createTestView(1, 3, 2);

        TopologyManager.TopologyChangeEvent event = manager.updateView(view);

        assertNotNull(event);
        assertNull(event.getPreviousEpoch());
        assertEquals(1, event.getCurrentEpoch());
        assertEquals(3, event.getAddedNodes().size());
        assertEquals(0, event.getRemovedNodes().size());

        assertTrue(manager.isInitialized());
        assertEquals(view, manager.getView());
        assertEquals(1L, manager.getEpoch());
    }

    @Test
    @DisplayName("TopologyManager: updateView with higher epoch")
    public void testUpdateViewNewer() {
        ClusterView view1 = createTestView(1, 3, 2);
        ClusterView view2 = createTestView(2, 3, 2);

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNotNull(event);
        assertEquals(1L, event.getPreviousEpoch());
        assertEquals(2, event.getCurrentEpoch());
        assertEquals(view2, manager.getView());
    }

    @Test
    @DisplayName("TopologyManager: updateView rejects stale view")
    public void testUpdateViewStale() {
        ClusterView view1 = createTestView(2, 3, 2);
        ClusterView view2 = createTestView(1, 3, 2); // Lower epoch

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNull(event, "Should reject stale view");
        assertEquals(view1, manager.getView());
        assertEquals(2L, manager.getEpoch());
    }

    @Test
    @DisplayName("TopologyManager: updateView rejects duplicate epoch")
    public void testUpdateViewDuplicate() {
        ClusterView view1 = createTestView(1, 3, 2);
        ClusterView view2 = createTestView(1, 3, 2); // Same epoch

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNull(event, "Should reject duplicate epoch");
    }

    @Test
    @DisplayName("TopologyManager: updateView throws on null")
    public void testUpdateViewNull() {
        assertThrows(IllegalArgumentException.class, () -> manager.updateView(null));
    }

    @Test
    @DisplayName("TopologyManager: detects added nodes")
    public void testDetectAddedNodes() {
        ClusterView view1 = createTestView(1, 2, 1);
        ClusterView view2 = createTestView(2, 4, 1);

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNotNull(event);
        assertEquals(2, event.getAddedNodes().size());
        assertEquals(0, event.getRemovedNodes().size());
    }

    @Test
    @DisplayName("TopologyManager: detects removed nodes")
    public void testDetectRemovedNodes() {
        ClusterView view1 = createTestView(1, 4, 1);
        ClusterView view2 = createTestView(2, 2, 1);

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNotNull(event);
        assertEquals(0, event.getAddedNodes().size());
        assertEquals(2, event.getRemovedNodes().size());
    }

    @Test
    @DisplayName("TopologyManager: detects leader changes")
    public void testDetectLeaderChanges() {
        // View 1: node0 is leader for shard 0
        ClusterView view1 = createViewWithLeaders(1,
                Arrays.asList(
                        new ClusterNode("node0", "localhost:9001"),
                        new ClusterNode("node1", "localhost:9002")
                ),
                Arrays.asList(
                        new ShardInfo(0, Arrays.asList(
                                new ShardReplica("node0", true),
                                new ShardReplica("node1", false)
                        ))
                )
        );

        // View 2: node1 is leader for shard 0
        ClusterView view2 = createViewWithLeaders(2,
                Arrays.asList(
                        new ClusterNode("node0", "localhost:9001"),
                        new ClusterNode("node1", "localhost:9002")
                ),
                Arrays.asList(
                        new ShardInfo(0, Arrays.asList(
                                new ShardReplica("node0", false),
                                new ShardReplica("node1", true)
                        ))
                )
        );

        manager.updateView(view1);
        TopologyManager.TopologyChangeEvent event = manager.updateView(view2);

        assertNotNull(event);
        assertEquals(1, event.getLeaderChanges().size());
        assertTrue(event.getLeaderChanges().containsKey(0));

        TopologyManager.LeaderChange change = event.getLeaderChanges().get(0);
        assertEquals("localhost:9001", change.getOldLeader());
        assertEquals("localhost:9002", change.getNewLeader());
    }

    @Test
    @DisplayName("TopologyManager: caches shard leaders")
    public void testCachesShardLeaders() {
        ClusterView view = createViewWithLeaders(1,
                Arrays.asList(
                        new ClusterNode("node0", "localhost:9001"),
                        new ClusterNode("node1", "localhost:9002")
                ),
                Arrays.asList(
                        new ShardInfo(0, Arrays.asList(
                                new ShardReplica("node0", true),
                                new ShardReplica("node1", false)
                        )),
                        new ShardInfo(1, Arrays.asList(
                                new ShardReplica("node0", false),
                                new ShardReplica("node1", true)
                        ))
                )
        );

        manager.updateView(view);

        assertEquals("localhost:9001", manager.getShardLeader(0));
        assertEquals("localhost:9002", manager.getShardLeader(1));
    }

    @Test
    @DisplayName("TopologyManager: handles shard with no leader")
    public void testShardWithNoLeader() {
        ClusterView view = createViewWithLeaders(1,
                Arrays.asList(new ClusterNode("node0", "localhost:9001")),
                Arrays.asList(
                        new ShardInfo(0, Arrays.asList(
                                new ShardReplica("node0", false) // No leader
                        ))
                )
        );

        manager.updateView(view);

        assertNull(manager.getShardLeader(0));
    }

    @Test
    @DisplayName("TopologyManager: updateLeaderHint updates cache")
    public void testUpdateLeaderHint() {
        ClusterView view = createTestView(1, 2, 1);
        manager.updateView(view);

        manager.updateLeaderHint(5, "localhost:9003");

        assertEquals("localhost:9003", manager.getShardLeader(5));
    }

    @Test
    @DisplayName("TopologyManager: updateLeaderHint ignores null")
    public void testUpdateLeaderHintIgnoresNull() {
        manager.updateLeaderHint(5, null);
        assertNull(manager.getShardLeader(5));

        manager.updateLeaderHint(5, "");
        assertNull(manager.getShardLeader(5));
    }

    @Test
    @DisplayName("TopologyManager: getNodeAddresses returns all nodes")
    public void testGetNodeAddresses() {
        ClusterView view = createTestView(1, 3, 1);
        manager.updateView(view);

        List<String> addresses = manager.getNodeAddresses();
        assertEquals(3, addresses.size());
        assertTrue(addresses.contains("localhost:9001"));
        assertTrue(addresses.contains("localhost:9002"));
        assertTrue(addresses.contains("localhost:9003"));
    }

    @Test
    @DisplayName("TopologyManager: getNodeByAddress finds node")
    public void testGetNodeByAddress() {
        ClusterView view = createTestView(1, 3, 1);
        manager.updateView(view);

        ClusterNode node = manager.getNodeByAddress("localhost:9002");
        assertNotNull(node);
        assertEquals("localhost:9002", node.getAddr());
    }

    @Test
    @DisplayName("TopologyManager: getNodeByAddress returns null for unknown")
    public void testGetNodeByAddressUnknown() {
        ClusterView view = createTestView(1, 3, 1);
        manager.updateView(view);

        ClusterNode node = manager.getNodeByAddress("localhost:9999");
        assertNull(node);
    }

    @Test
    @DisplayName("TopologyManager: getShards returns all shards")
    public void testGetShards() {
        ClusterView view = createTestView(1, 2, 4);
        manager.updateView(view);

        List<ShardInfo> shards = manager.getShards();
        assertEquals(4, shards.size());
    }

    @Test
    @DisplayName("TopologyManager: getShard finds shard by ID")
    public void testGetShard() {
        ClusterView view = createTestView(1, 2, 4);
        manager.updateView(view);

        ShardInfo shard = manager.getShard(2);
        assertNotNull(shard);
        assertEquals(2, shard.getId());
    }

    @Test
    @DisplayName("TopologyManager: getShard returns null for unknown ID")
    public void testGetShardUnknown() {
        ClusterView view = createTestView(1, 2, 4);
        manager.updateView(view);

        ShardInfo shard = manager.getShard(999);
        assertNull(shard);
    }

    @Test
    @DisplayName("TopologyManager: onTopologyChange registers listener")
    public void testOnTopologyChangeRegistersListener() {
        AtomicInteger callCount = new AtomicInteger(0);
        AtomicReference<TopologyManager.TopologyChangeEvent> receivedEvent = new AtomicReference<>();

        manager.onTopologyChange(event -> {
            callCount.incrementAndGet();
            receivedEvent.set(event);
        });

        ClusterView view = createTestView(1, 2, 1);
        manager.updateView(view);

        assertEquals(1, callCount.get());
        assertNotNull(receivedEvent.get());
        assertEquals(1, receivedEvent.get().getCurrentEpoch());
    }

    @Test
    @DisplayName("TopologyManager: onTopologyChange allows unsubscribe")
    public void testOnTopologyChangeUnsubscribe() {
        AtomicInteger callCount = new AtomicInteger(0);

        Runnable unsubscribe = manager.onTopologyChange(event -> callCount.incrementAndGet());

        manager.updateView(createTestView(1, 2, 1));
        assertEquals(1, callCount.get());

        unsubscribe.run();

        manager.updateView(createTestView(2, 2, 1));
        assertEquals(1, callCount.get(), "Should not be called after unsubscribe");
    }

    @Test
    @DisplayName("TopologyManager: onTopologyChange supports multiple listeners")
    public void testMultipleListeners() {
        AtomicInteger count1 = new AtomicInteger(0);
        AtomicInteger count2 = new AtomicInteger(0);

        manager.onTopologyChange(event -> count1.incrementAndGet());
        manager.onTopologyChange(event -> count2.incrementAndGet());

        manager.updateView(createTestView(1, 2, 1));

        assertEquals(1, count1.get());
        assertEquals(1, count2.get());
    }

    @Test
    @DisplayName("TopologyManager: onTopologyChange handles listener exceptions")
    public void testListenerExceptions() {
        AtomicInteger count = new AtomicInteger(0);

        manager.onTopologyChange(event -> {
            throw new RuntimeException("Listener error");
        });
        manager.onTopologyChange(event -> count.incrementAndGet());

        manager.updateView(createTestView(1, 2, 1));

        // Second listener should still be called despite first throwing
        assertEquals(1, count.get());
    }

    @Test
    @DisplayName("TopologyManager: onTopologyChange throws on null listener")
    public void testOnTopologyChangeNullListener() {
        assertThrows(IllegalArgumentException.class, () -> manager.onTopologyChange(null));
    }

    @Test
    @DisplayName("TopologyManager: clear removes all state")
    public void testClear() {
        ClusterView view = createTestView(1, 3, 2);
        manager.updateView(view);

        assertTrue(manager.isInitialized());
        assertNotNull(manager.getView());

        manager.clear();

        assertFalse(manager.isInitialized());
        assertNull(manager.getView());
        assertNull(manager.getEpoch());
        assertTrue(manager.getNodeAddresses().isEmpty());
    }

    @Test
    @DisplayName("TopologyManager: getStats returns accurate statistics")
    public void testGetStats() {
        TopologyManager.TopologyStats stats = manager.getStats();

        assertNull(stats.getCurrentEpoch());
        assertEquals(0, stats.getTotalNodes());
        assertEquals(0, stats.getTotalShards());
        assertEquals(0, stats.getCachedLeaders());
        assertEquals(0, stats.getListenerCount());

        manager.onTopologyChange(event -> {});
        manager.updateView(createTestView(1, 3, 4));

        stats = manager.getStats();
        assertEquals(1L, stats.getCurrentEpoch());
        assertEquals(3, stats.getTotalNodes());
        assertEquals(4, stats.getTotalShards());
        assertTrue(stats.getCachedLeaders() > 0);
        assertEquals(1, stats.getListenerCount());
    }

    @Test
    @DisplayName("TopologyManager: concurrent updates are thread-safe")
    public void testConcurrentUpdates() throws InterruptedException {
        int numThreads = 10;
        Thread[] threads = new Thread[numThreads];

        for (int i = 0; i < numThreads; i++) {
            final int epoch = i + 1;
            threads[i] = new Thread(() -> {
                manager.updateView(createTestView(epoch, 3, 2));
            });
            threads[i].start();
        }

        for (Thread thread : threads) {
            thread.join();
        }

        // Final view should have highest epoch
        assertEquals(10L, manager.getEpoch());
    }

    @Test
    @DisplayName("TopologyManager: concurrent reads during updates")
    public void testConcurrentReadsAndUpdates() throws InterruptedException {
        manager.updateView(createTestView(1, 3, 2));

        AtomicInteger readCount = new AtomicInteger(0);
        Thread[] readers = new Thread[5];
        Thread[] writers = new Thread[5];

        for (int i = 0; i < 5; i++) {
            readers[i] = new Thread(() -> {
                for (int j = 0; j < 100; j++) {
                    manager.getView();
                    manager.getEpoch();
                    manager.getNodeAddresses();
                    manager.getShards();
                    readCount.incrementAndGet();
                }
            });
            readers[i].start();
        }

        for (int i = 0; i < 5; i++) {
            final int epoch = i + 2;
            writers[i] = new Thread(() -> {
                for (int j = 0; j < 10; j++) {
                    manager.updateView(createTestView(epoch * 10 + j, 3, 2));
                }
            });
            writers[i].start();
        }

        for (Thread reader : readers) {
            reader.join();
        }
        for (Thread writer : writers) {
            writer.join();
        }

        assertTrue(readCount.get() > 0);
        assertTrue(manager.isInitialized());
    }

    // Helper methods

    private ClusterView createTestView(long epoch, int numNodes, int numShards) {
        List<ClusterNode> nodes = new ArrayList<>();
        for (int i = 0; i < numNodes; i++) {
            nodes.add(new ClusterNode("node" + i, "localhost:900" + (i + 1)));
        }

        List<ShardInfo> shards = new ArrayList<>();
        for (int i = 0; i < numShards; i++) {
            List<ShardReplica> replicas = new ArrayList<>();
            for (int j = 0; j < Math.min(numNodes, 3); j++) {
                replicas.add(new ShardReplica("node" + j, j == 0));
            }
            shards.add(new ShardInfo(i, replicas));
        }

        return new ClusterView(epoch, nodes, shards);
    }

    private ClusterView createViewWithLeaders(long epoch, List<ClusterNode> nodes,
                                              List<ShardInfo> shards) {
        return new ClusterView(epoch, nodes, shards);
    }
}
