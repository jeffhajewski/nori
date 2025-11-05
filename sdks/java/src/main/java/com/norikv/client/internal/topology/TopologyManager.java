package com.norikv.client.internal.topology;

import com.norikv.client.types.*;

import java.util.*;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.locks.ReadWriteLock;
import java.util.concurrent.locks.ReentrantReadWriteLock;
import java.util.function.Consumer;

/**
 * Manages cluster topology information and change notifications.
 *
 * <p>This class provides:
 * <ul>
 * <li>Tracking of cluster view (nodes and shard assignments)</li>
 * <li>Leader caching for efficient routing</li>
 * <li>Change detection and notification</li>
 * <li>Thread-safe concurrent access</li>
 * </ul>
 *
 * <p>Thread-safe for concurrent use.
 */
public final class TopologyManager {
    private volatile ClusterView view;
    private final ConcurrentHashMap<Integer, String> shardLeaderCache;
    private final ConcurrentHashMap<String, ClusterNode> nodeByIdCache;
    private final CopyOnWriteArrayList<Consumer<TopologyChangeEvent>> listeners;
    private final ReadWriteLock viewLock;

    /**
     * Creates a new TopologyManager.
     */
    public TopologyManager() {
        this.view = null;
        this.shardLeaderCache = new ConcurrentHashMap<>();
        this.nodeByIdCache = new ConcurrentHashMap<>();
        this.listeners = new CopyOnWriteArrayList<>();
        this.viewLock = new ReentrantReadWriteLock();
    }

    /**
     * Updates the cluster view.
     *
     * <p>Checks if the new view is actually newer (higher epoch) before updating.
     * Computes changes and notifies listeners.
     *
     * @param newView the new cluster view
     * @return change event describing the update, or null if view was stale
     */
    public TopologyChangeEvent updateView(ClusterView newView) {
        if (newView == null) {
            throw new IllegalArgumentException("newView cannot be null");
        }

        viewLock.writeLock().lock();
        try {
            ClusterView oldView = this.view;

            // Check if this is actually a newer view
            if (oldView != null && newView.getEpoch() <= oldView.getEpoch()) {
                return null; // Stale or duplicate view
            }

            this.view = newView;

            // Rebuild caches
            rebuildCaches(newView);

            // Compute changes
            TopologyChangeEvent event = computeChanges(oldView, newView);

            // Notify listeners (outside the lock)
            notifyListeners(event);

            return event;
        } finally {
            viewLock.writeLock().unlock();
        }
    }

    /**
     * Rebuilds internal caches from the cluster view.
     *
     * @param view the cluster view
     */
    private void rebuildCaches(ClusterView view) {
        // Build node cache
        nodeByIdCache.clear();
        for (ClusterNode node : view.getNodes()) {
            nodeByIdCache.put(node.getId(), node);
        }

        // Build shard leader cache
        shardLeaderCache.clear();
        for (ShardInfo shard : view.getShards()) {
            ShardReplica leader = findLeader(shard.getReplicas());
            if (leader != null) {
                ClusterNode leaderNode = nodeByIdCache.get(leader.getNodeId());
                if (leaderNode != null) {
                    shardLeaderCache.put(shard.getId(), leaderNode.getAddr());
                }
            }
            // Note: If no leader, we simply don't cache anything for this shard.
            // getShardLeader() will return null for uncached entries.
        }
    }

    /**
     * Finds the leader replica in a list of replicas.
     *
     * @param replicas the list of replicas
     * @return the leader replica, or null if no leader
     */
    private ShardReplica findLeader(List<ShardReplica> replicas) {
        for (ShardReplica replica : replicas) {
            if (replica.isLeader()) {
                return replica;
            }
        }
        return null;
    }

    /**
     * Computes changes between two cluster views.
     *
     * @param oldView the old view (may be null)
     * @param newView the new view
     * @return change event describing the differences
     */
    private TopologyChangeEvent computeChanges(ClusterView oldView, ClusterView newView) {
        List<String> addedNodes = new ArrayList<>();
        List<String> removedNodes = new ArrayList<>();
        Map<Integer, LeaderChange> leaderChanges = new HashMap<>();

        if (oldView == null) {
            // First view - all nodes are "added"
            for (ClusterNode node : newView.getNodes()) {
                addedNodes.add(node.getAddr());
            }

            return new TopologyChangeEvent(
                    null,
                    newView.getEpoch(),
                    addedNodes,
                    removedNodes,
                    leaderChanges
            );
        }

        // Track node changes
        Set<String> oldNodeAddrs = new HashSet<>();
        for (ClusterNode node : oldView.getNodes()) {
            oldNodeAddrs.add(node.getAddr());
        }

        Set<String> newNodeAddrs = new HashSet<>();
        for (ClusterNode node : newView.getNodes()) {
            newNodeAddrs.add(node.getAddr());
        }

        for (String addr : newNodeAddrs) {
            if (!oldNodeAddrs.contains(addr)) {
                addedNodes.add(addr);
            }
        }

        for (String addr : oldNodeAddrs) {
            if (!newNodeAddrs.contains(addr)) {
                removedNodes.add(addr);
            }
        }

        // Track leader changes
        Map<Integer, String> oldShardLeaders = new HashMap<>();
        for (ShardInfo shard : oldView.getShards()) {
            ShardReplica leader = findLeader(shard.getReplicas());
            if (leader != null) {
                for (ClusterNode node : oldView.getNodes()) {
                    if (node.getId().equals(leader.getNodeId())) {
                        oldShardLeaders.put(shard.getId(), node.getAddr());
                        break;
                    }
                }
            } else {
                oldShardLeaders.put(shard.getId(), null);
            }
        }

        for (ShardInfo shard : newView.getShards()) {
            String oldLeader = oldShardLeaders.get(shard.getId());
            ShardReplica leader = findLeader(shard.getReplicas());
            String newLeader = null;
            if (leader != null) {
                for (ClusterNode node : newView.getNodes()) {
                    if (node.getId().equals(leader.getNodeId())) {
                        newLeader = node.getAddr();
                        break;
                    }
                }
            }

            if (!Objects.equals(oldLeader, newLeader)) {
                leaderChanges.put(shard.getId(), new LeaderChange(oldLeader, newLeader));
            }
        }

        return new TopologyChangeEvent(
                oldView.getEpoch(),
                newView.getEpoch(),
                addedNodes,
                removedNodes,
                leaderChanges
        );
    }

    /**
     * Notifies all registered listeners of a topology change.
     *
     * @param event the change event
     */
    private void notifyListeners(TopologyChangeEvent event) {
        for (Consumer<TopologyChangeEvent> listener : listeners) {
            try {
                listener.accept(event);
            } catch (Exception e) {
                // Log error but continue notifying other listeners
                System.err.println("Topology listener error: " + e.getMessage());
            }
        }
    }

    /**
     * Gets the current cluster view.
     *
     * @return the cluster view, or null if not initialized
     */
    public ClusterView getView() {
        return view;
    }

    /**
     * Gets the current epoch.
     *
     * @return the epoch, or null if not initialized
     */
    public Long getEpoch() {
        ClusterView currentView = view;
        return currentView != null ? currentView.getEpoch() : null;
    }

    /**
     * Gets the leader address for a shard.
     *
     * @param shardId the shard ID
     * @return the leader address, or null if no leader
     */
    public String getShardLeader(int shardId) {
        return shardLeaderCache.get(shardId);
    }

    /**
     * Updates the leader hint for a shard.
     *
     * <p>This is an optimization to avoid waiting for the next cluster view update.
     * Called when receiving NotLeaderException with a leader hint.
     *
     * @param shardId the shard ID
     * @param leaderAddr the leader address
     */
    public void updateLeaderHint(int shardId, String leaderAddr) {
        if (leaderAddr != null && !leaderAddr.isEmpty()) {
            shardLeaderCache.put(shardId, leaderAddr);
        }
    }

    /**
     * Gets all node addresses from the current view.
     *
     * @return list of node addresses, empty if not initialized
     */
    public List<String> getNodeAddresses() {
        ClusterView currentView = view;
        if (currentView == null) {
            return Collections.emptyList();
        }

        List<String> addresses = new ArrayList<>();
        for (ClusterNode node : currentView.getNodes()) {
            addresses.add(node.getAddr());
        }
        return addresses;
    }

    /**
     * Gets a node by address.
     *
     * @param address the node address
     * @return the cluster node, or null if not found
     */
    public ClusterNode getNodeByAddress(String address) {
        ClusterView currentView = view;
        if (currentView == null) {
            return null;
        }

        for (ClusterNode node : currentView.getNodes()) {
            if (node.getAddr().equals(address)) {
                return node;
            }
        }
        return null;
    }

    /**
     * Gets all shards from the current view.
     *
     * @return list of shard info, empty if not initialized
     */
    public List<ShardInfo> getShards() {
        ClusterView currentView = view;
        return currentView != null ? currentView.getShards() : Collections.emptyList();
    }

    /**
     * Gets shard info by ID.
     *
     * @param shardId the shard ID
     * @return the shard info, or null if not found
     */
    public ShardInfo getShard(int shardId) {
        ClusterView currentView = view;
        if (currentView == null) {
            return null;
        }

        for (ShardInfo shard : currentView.getShards()) {
            if (shard.getId() == shardId) {
                return shard;
            }
        }
        return null;
    }

    /**
     * Registers a listener for topology changes.
     *
     * @param listener the listener callback
     * @return runnable to unregister the listener
     */
    public Runnable onTopologyChange(Consumer<TopologyChangeEvent> listener) {
        if (listener == null) {
            throw new IllegalArgumentException("listener cannot be null");
        }

        listeners.add(listener);
        return () -> listeners.remove(listener);
    }

    /**
     * Checks if the topology is initialized.
     *
     * @return true if a cluster view has been set
     */
    public boolean isInitialized() {
        return view != null;
    }

    /**
     * Clears all cached topology information.
     */
    public void clear() {
        viewLock.writeLock().lock();
        try {
            view = null;
            shardLeaderCache.clear();
            nodeByIdCache.clear();
        } finally {
            viewLock.writeLock().unlock();
        }
    }

    /**
     * Gets statistics about the topology manager.
     *
     * @return topology statistics
     */
    public TopologyStats getStats() {
        ClusterView currentView = view;
        return new TopologyStats(
                currentView != null ? currentView.getEpoch() : null,
                currentView != null ? currentView.getNodes().size() : 0,
                currentView != null ? currentView.getShards().size() : 0,
                shardLeaderCache.size(),
                listeners.size()
        );
    }

    /**
     * Event describing changes to cluster topology.
     */
    public static final class TopologyChangeEvent {
        private final Long previousEpoch;
        private final long currentEpoch;
        private final List<String> addedNodes;
        private final List<String> removedNodes;
        private final Map<Integer, LeaderChange> leaderChanges;

        TopologyChangeEvent(Long previousEpoch, long currentEpoch,
                           List<String> addedNodes, List<String> removedNodes,
                           Map<Integer, LeaderChange> leaderChanges) {
            this.previousEpoch = previousEpoch;
            this.currentEpoch = currentEpoch;
            this.addedNodes = addedNodes;
            this.removedNodes = removedNodes;
            this.leaderChanges = leaderChanges;
        }

        public Long getPreviousEpoch() {
            return previousEpoch;
        }

        public long getCurrentEpoch() {
            return currentEpoch;
        }

        public List<String> getAddedNodes() {
            return addedNodes;
        }

        public List<String> getRemovedNodes() {
            return removedNodes;
        }

        public Map<Integer, LeaderChange> getLeaderChanges() {
            return leaderChanges;
        }

        @Override
        public String toString() {
            return "TopologyChangeEvent{" +
                    "previousEpoch=" + previousEpoch +
                    ", currentEpoch=" + currentEpoch +
                    ", addedNodes=" + addedNodes.size() +
                    ", removedNodes=" + removedNodes.size() +
                    ", leaderChanges=" + leaderChanges.size() +
                    '}';
        }
    }

    /**
     * Describes a leader change for a shard.
     */
    public static final class LeaderChange {
        private final String oldLeader;
        private final String newLeader;

        LeaderChange(String oldLeader, String newLeader) {
            this.oldLeader = oldLeader;
            this.newLeader = newLeader;
        }

        public String getOldLeader() {
            return oldLeader;
        }

        public String getNewLeader() {
            return newLeader;
        }

        @Override
        public String toString() {
            return "LeaderChange{" +
                    "old=" + oldLeader +
                    ", new=" + newLeader +
                    '}';
        }
    }

    /**
     * Statistics about the topology manager.
     */
    public static final class TopologyStats {
        private final Long currentEpoch;
        private final int totalNodes;
        private final int totalShards;
        private final int cachedLeaders;
        private final int listenerCount;

        TopologyStats(Long currentEpoch, int totalNodes, int totalShards,
                     int cachedLeaders, int listenerCount) {
            this.currentEpoch = currentEpoch;
            this.totalNodes = totalNodes;
            this.totalShards = totalShards;
            this.cachedLeaders = cachedLeaders;
            this.listenerCount = listenerCount;
        }

        public Long getCurrentEpoch() {
            return currentEpoch;
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

        public int getListenerCount() {
            return listenerCount;
        }

        @Override
        public String toString() {
            return "TopologyStats{" +
                    "currentEpoch=" + currentEpoch +
                    ", totalNodes=" + totalNodes +
                    ", totalShards=" + totalShards +
                    ", cachedLeaders=" + cachedLeaders +
                    ", listenerCount=" + listenerCount +
                    '}';
        }
    }
}
