// Package router implements client-side routing for NoriKV.
package router

import (
	"context"
	"fmt"
	"sync"

	"github.com/norikv/norikv-go/hash"
	"golang.org/x/sync/singleflight"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"

	"github.com/norikv/norikv-go/internal/conn"
)

// Router handles routing requests to the appropriate node based on key hashing
// and leader information. Uses single-flight pattern to deduplicate concurrent
// leader lookups for the same shard.
type Router struct {
	pool        *conn.Pool
	totalShards int

	// Leader tracking
	mu            sync.RWMutex
	leaderCache   map[uint32]string // shard ID -> leader address
	nodeAddresses []string          // all known node addresses

	// Single-flight for leader discovery
	leaderLookup singleflight.Group
}

// NewRouter creates a new router with the given connection pool and configuration.
func NewRouter(pool *conn.Pool, totalShards int, nodeAddresses []string) *Router {
	return &Router{
		pool:          pool,
		totalShards:   totalShards,
		leaderCache:   make(map[uint32]string),
		nodeAddresses: nodeAddresses,
	}
}

// GetShardForKey returns the shard ID for the given key.
func (r *Router) GetShardForKey(key []byte) uint32 {
	return uint32(hash.GetShardForKey(key, r.totalShards))
}

// GetLeader returns the cached leader address for the given shard.
// Returns empty string if no leader is cached.
func (r *Router) GetLeader(shardID uint32) string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.leaderCache[shardID]
}

// SetLeader updates the cached leader address for the given shard.
func (r *Router) SetLeader(shardID uint32, address string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.leaderCache[shardID] = address
}

// ClearLeader removes the cached leader for the given shard.
// Used when receiving NOT_LEADER errors.
func (r *Router) ClearLeader(shardID uint32) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.leaderCache, shardID)
}

// UpdateNodeAddresses updates the list of known node addresses.
// Called by the topology manager when cluster membership changes.
func (r *Router) UpdateNodeAddresses(addresses []string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.nodeAddresses = addresses
}

// GetConnection returns a gRPC connection for the given shard.
// First tries to use the cached leader, falls back to any node if no leader is cached.
// Uses single-flight pattern to deduplicate concurrent lookups for the same shard.
func (r *Router) GetConnection(ctx context.Context, shardID uint32) (*grpc.ClientConn, string, error) {
	// Fast path: try cached leader
	leader := r.GetLeader(shardID)
	if leader != "" {
		conn, err := r.pool.Get(ctx, leader)
		if err == nil {
			return conn, leader, nil
		}
		// Leader connection failed, clear cache and continue
		r.ClearLeader(shardID)
	}

	// Slow path: discover leader using single-flight
	// This ensures only one goroutine performs leader discovery per shard
	key := fmt.Sprintf("shard:%d", shardID)
	result, err, _ := r.leaderLookup.Do(key, func() (interface{}, error) {
		return r.discoverLeader(ctx, shardID)
	})

	if err != nil {
		return nil, "", err
	}

	address := result.(string)
	conn, err := r.pool.Get(ctx, address)
	if err != nil {
		return nil, "", fmt.Errorf("failed to connect to %s: %w", address, err)
	}

	return conn, address, nil
}

// discoverLeader attempts to discover the leader for the given shard.
// Tries all known nodes until one responds successfully or returns leader hint.
func (r *Router) discoverLeader(ctx context.Context, shardID uint32) (string, error) {
	r.mu.RLock()
	nodes := make([]string, len(r.nodeAddresses))
	copy(nodes, r.nodeAddresses)
	r.mu.RUnlock()

	if len(nodes) == 0 {
		return "", fmt.Errorf("no nodes available for shard %d", shardID)
	}

	// Try each node until we find the leader or get a leader hint
	for _, address := range nodes {
		_, err := r.pool.Get(ctx, address)
		if err != nil {
			// Connection failed, try next node
			continue
		}

		// Successfully connected - cache this node as potential leader
		r.SetLeader(shardID, address)
		return address, nil
	}

	return "", fmt.Errorf("failed to discover leader for shard %d: all nodes unreachable", shardID)
}

// GetConnectionForKey returns a gRPC connection for the node responsible for the given key.
// Convenience method that combines GetShardForKey and GetConnection.
func (r *Router) GetConnectionForKey(ctx context.Context, key []byte) (*grpc.ClientConn, string, uint32, error) {
	shardID := r.GetShardForKey(key)
	conn, address, err := r.GetConnection(ctx, shardID)
	if err != nil {
		return nil, "", shardID, err
	}
	return conn, address, shardID, nil
}

// HandleNotLeaderError processes a NOT_LEADER error by updating the leader cache.
// Extracts the leader hint from the error/metadata and updates the cache.
func (r *Router) HandleNotLeaderError(shardID uint32, leaderHint string) {
	// Clear old leader
	r.ClearLeader(shardID)

	// If we got a leader hint, cache it
	if leaderHint != "" {
		r.SetLeader(shardID, leaderHint)
	}
}

// ExtractLeaderHint extracts the leader hint from gRPC metadata.
// Returns empty string if no hint is present.
func ExtractLeaderHint(md metadata.MD) string {
	if md == nil {
		return ""
	}
	hints := md.Get("leader-hint")
	if len(hints) > 0 {
		return hints[0]
	}
	return ""
}

// GetAllLeaders returns a copy of the current leader cache.
// Useful for debugging and monitoring.
func (r *Router) GetAllLeaders() map[uint32]string {
	r.mu.RLock()
	defer r.mu.RUnlock()

	leaders := make(map[uint32]string, len(r.leaderCache))
	for shardID, address := range r.leaderCache {
		leaders[shardID] = address
	}
	return leaders
}

// ClearAllLeaders removes all cached leader information.
// Used when cluster topology changes significantly.
func (r *Router) ClearAllLeaders() {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.leaderCache = make(map[uint32]string)
}

// GetNodeAddresses returns the current list of known node addresses.
func (r *Router) GetNodeAddresses() []string {
	r.mu.RLock()
	defer r.mu.RUnlock()

	addresses := make([]string, len(r.nodeAddresses))
	copy(addresses, r.nodeAddresses)
	return addresses
}

// TotalShards returns the total number of shards.
func (r *Router) TotalShards() int {
	return r.totalShards
}

// Close closes the router and cleans up resources.
func (r *Router) Close() error {
	r.ClearAllLeaders()
	return nil
}
