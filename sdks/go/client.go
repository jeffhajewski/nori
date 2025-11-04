package norikv

import (
	"context"
	"fmt"
	"time"

	"github.com/norikv/norikv-go/internal/conn"
	"github.com/norikv/norikv-go/internal/retry"
	"github.com/norikv/norikv-go/internal/router"
	"github.com/norikv/norikv-go/internal/topology"
	"github.com/norikv/norikv-go/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/metadata"
)

// Client is the main NoriKV client for interacting with the cluster.
type Client struct {
	config   *ClientConfig
	pool     *conn.Pool
	router   *router.Router
	topology *topology.Manager
	retry    *retry.Policy
}

// NewClient creates a new NoriKV client with the given configuration.
func NewClient(ctx context.Context, config *ClientConfig) (*Client, error) {
	if config == nil {
		config = DefaultClientConfig(nil)
	}

	if len(config.Nodes) == 0 {
		return nil, NewInvalidArgumentError("at least one node address is required")
	}

	// Create connection pool
	pool := conn.NewPool(conn.PoolOptions{
		MaxConnectionsPerNode: config.MaxConnectionsPerNode,
		DialTimeoutMs:         config.TimeoutMs,
	})

	// Create router
	rtr := router.NewRouter(pool, config.TotalShards, config.Nodes)

	// Create topology manager
	topoMgr := topology.NewManager(ctx, topology.ManagerOptions{
		InitialNodes: config.Nodes,
	})

	// Convert retry config to retry policy
	retryPolicy := retry.NewPolicy(
		retry.WithMaxAttempts(config.Retry.MaxAttempts),
		retry.WithInitialDelay(config.Retry.InitialDelayMs),
		retry.WithMaxDelay(config.Retry.MaxDelayMs),
		retry.WithBackoffMultiplier(config.Retry.BackoffMultiplier),
		retry.WithJitter(config.Retry.JitterMs),
		retry.WithRetryOnNotLeader(config.Retry.RetryOnNotLeader),
	)

	client := &Client{
		config:   config,
		pool:     pool,
		router:   rtr,
		topology: topoMgr,
		retry:    retryPolicy,
	}

	// Start topology manager if cluster watching is enabled
	if config.WatchCluster {
		if err := topoMgr.Start(); err != nil {
			return nil, fmt.Errorf("failed to start topology manager: %w", err)
		}

		// Subscribe to topology events and update router
		go client.watchTopology(ctx)
	}

	return client, nil
}

// watchTopology listens for topology changes and updates the router.
func (c *Client) watchTopology(ctx context.Context) {
	events := c.topology.Subscribe()
	defer c.topology.Unsubscribe(events)

	for {
		select {
		case <-ctx.Done():
			return
		case event, ok := <-events:
			if !ok {
				return
			}

			// Update router with new node addresses
			nodes := c.topology.GetNodes()
			c.router.UpdateNodeAddresses(nodes)

			// Clear leader cache for affected shards on leader changes
			for shardID := range event.LeaderChanges {
				c.router.ClearLeader(shardID)
			}
		}
	}
}

// Put writes a key-value pair to the cluster.
func (c *Client) Put(ctx context.Context, key, value []byte, opts *PutOptions) (*Version, error) {
	if len(key) == 0 {
		return nil, NewInvalidArgumentError("key cannot be empty")
	}

	if opts == nil {
		opts = &PutOptions{}
	}

	// Apply timeout
	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var version *Version

	err := c.retry.Execute(ctx, func() error {
		// Get connection for this key
		conn, address, shardID, err := c.router.GetConnectionForKey(ctx, key)
		if err != nil {
			return err
		}

		// Create gRPC client
		client := proto.NewKvClient(conn)

		// Build request
		req := &proto.PutRequest{
			Key:   key,
			Value: value,
		}

		if opts.TTLMs != nil {
			req.TtlMs = *opts.TTLMs
		}
		if opts.IdempotencyKey != "" {
			req.IdempotencyKey = opts.IdempotencyKey
		}
		if opts.IfMatchVersion != nil {
			req.IfMatch = opts.IfMatchVersion.ToProto()
		}
		// Note: if_not_exists is not yet in proto, will be added later
		// if opts.IfNotExists {
		//     req.IfNotExists = true
		// }

		// Execute RPC
		var md metadata.MD
		resp, err := client.Put(ctx, req, grpc.Trailer(&md))
		if err != nil {
			// Convert gRPC error
			err = FromGRPCError(err, md)

			// Handle NOT_LEADER
			if _, ok := err.(*NotLeaderError); ok {
				hint := router.ExtractLeaderHint(md)
				c.router.HandleNotLeaderError(shardID, hint)
				c.pool.Remove(address)
				return err
			}

			return err
		}

		// Convert version
		v := &Version{}
		version = v.FromProto(resp.Version)
		return nil
	})

	return version, err
}

// Get retrieves a value from the cluster.
func (c *Client) Get(ctx context.Context, key []byte, opts *GetOptions) (*GetResult, error) {
	if len(key) == 0 {
		return nil, NewInvalidArgumentError("key cannot be empty")
	}

	if opts == nil {
		opts = &GetOptions{
			Consistency: ConsistencyLease,
		}
	}

	// Apply timeout
	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	var result *GetResult

	err := c.retry.Execute(ctx, func() error {
		// Get connection for this key
		conn, address, shardID, err := c.router.GetConnectionForKey(ctx, key)
		if err != nil {
			return err
		}

		// Create gRPC client
		client := proto.NewKvClient(conn)

		// Build request
		req := &proto.GetRequest{
			Key: key,
		}

		// Set consistency level (proto uses string, not enum)
		switch opts.Consistency {
		case ConsistencyLease:
			req.Consistency = "lease"
		case ConsistencyLinearizable:
			req.Consistency = "linearizable"
		case ConsistencyStaleOK:
			req.Consistency = "stale_ok"
		default:
			req.Consistency = "lease"
		}

		// Execute RPC
		var md metadata.MD
		resp, err := client.Get(ctx, req, grpc.Trailer(&md))
		if err != nil {
			// Convert gRPC error
			err = FromGRPCError(err, md)

			// Handle NOT_LEADER
			if _, ok := err.(*NotLeaderError); ok {
				hint := router.ExtractLeaderHint(md)
				c.router.HandleNotLeaderError(shardID, hint)
				c.pool.Remove(address)
				return err
			}

			return err
		}

		// Convert response
		v := &Version{}
		result = &GetResult{
			Value:    resp.Value,
			Version:  v.FromProto(resp.Version),
			Metadata: resp.Meta,
		}
		return nil
	})

	return result, err
}

// Delete removes a key from the cluster.
func (c *Client) Delete(ctx context.Context, key []byte, opts *DeleteOptions) error {
	if len(key) == 0 {
		return NewInvalidArgumentError("key cannot be empty")
	}

	if opts == nil {
		opts = &DeleteOptions{}
	}

	// Apply timeout
	ctx, cancel := c.withTimeout(ctx)
	defer cancel()

	return c.retry.Execute(ctx, func() error {
		// Get connection for this key
		conn, address, shardID, err := c.router.GetConnectionForKey(ctx, key)
		if err != nil {
			return err
		}

		// Create gRPC client
		client := proto.NewKvClient(conn)

		// Build request
		req := &proto.DeleteRequest{
			Key: key,
		}

		if opts.IdempotencyKey != "" {
			req.IdempotencyKey = opts.IdempotencyKey
		}
		if opts.IfMatchVersion != nil {
			req.IfMatch = opts.IfMatchVersion.ToProto()
		}

		// Execute RPC
		var md metadata.MD
		_, err = client.Delete(ctx, req, grpc.Trailer(&md))
		if err != nil {
			// Convert gRPC error
			err = FromGRPCError(err, md)

			// Handle NOT_LEADER
			if _, ok := err.(*NotLeaderError); ok {
				hint := router.ExtractLeaderHint(md)
				c.router.HandleNotLeaderError(shardID, hint)
				c.pool.Remove(address)
				return err
			}

			return err
		}

		return nil
	})
}

// Close closes the client and cleans up resources.
func (c *Client) Close() error {
	// Stop topology manager
	if err := c.topology.Close(); err != nil {
		return err
	}

	// Close router
	if err := c.router.Close(); err != nil {
		return err
	}

	// Close connection pool
	if err := c.pool.Close(); err != nil {
		return err
	}

	return nil
}

// GetClusterView returns the current cluster topology view.
func (c *Client) GetClusterView() *ClusterView {
	nodes := c.topology.GetNodes()
	epoch := c.topology.GetEpoch()

	clusterNodes := make([]ClusterNode, len(nodes))
	for i, addr := range nodes {
		active := c.topology.IsNodeActive(addr)
		role := "voter"
		if !active {
			role = "unavailable"
		}
		clusterNodes[i] = ClusterNode{
			ID:   addr,
			Addr: addr,
			Role: role,
		}
	}

	leaders := c.router.GetAllLeaders()
	shards := make([]ShardInfo, 0)
	for shardID, leaderAddr := range leaders {
		shards = append(shards, ShardInfo{
			ID: shardID,
			Replicas: []ShardReplica{
				{
					NodeID:   leaderAddr,
					IsLeader: true,
				},
			},
		})
	}

	return &ClusterView{
		Epoch:  epoch,
		Nodes:  clusterNodes,
		Shards: shards,
	}
}

// withTimeout applies the configured timeout to a context if no deadline is set.
func (c *Client) withTimeout(ctx context.Context) (context.Context, context.CancelFunc) {
	if _, ok := ctx.Deadline(); ok {
		// Context already has a deadline
		return ctx, func() {}
	}

	timeout := time.Duration(c.config.TimeoutMs) * time.Millisecond
	return context.WithTimeout(ctx, timeout)
}
