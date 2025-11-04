// Package conn implements connection pooling for NoriKV client.
package conn

import (
	"context"
	"fmt"
	"sync"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/connectivity"
	"google.golang.org/grpc/credentials/insecure"
)

// Pool manages gRPC connections to cluster nodes.
// Connections are cached per node address and reused across requests.
// Thread-safe for concurrent access.
type Pool struct {
	mu          sync.RWMutex
	connections map[string]*pooledConn // address -> connection
	opts        PoolOptions
}

// PoolOptions contains configuration for the connection pool.
type PoolOptions struct {
	// MaxConnectionsPerNode is the maximum number of connections per node (default: 1)
	// Currently only supports 1 connection per node for simplicity.
	MaxConnectionsPerNode int

	// DialTimeout is the timeout for establishing connections (default: 5s)
	DialTimeoutMs int

	// IdleTimeout is the duration after which idle connections are closed (default: 5m)
	// 0 means connections are never closed due to idleness.
	IdleTimeoutMs int

	// HealthCheckInterval is the interval for health checking connections (default: 30s)
	// 0 disables health checking.
	HealthCheckIntervalMs int
}

// pooledConn wraps a gRPC connection with metadata.
type pooledConn struct {
	conn         *grpc.ClientConn
	address      string
	lastUsed     time.Time
	mu           sync.Mutex
	healthTicker *time.Ticker
	stopHealth   chan struct{}
}

// NewPool creates a new connection pool with the given options.
func NewPool(opts PoolOptions) *Pool {
	if opts.MaxConnectionsPerNode <= 0 {
		opts.MaxConnectionsPerNode = 1
	}
	if opts.DialTimeoutMs <= 0 {
		opts.DialTimeoutMs = 5000
	}
	if opts.IdleTimeoutMs <= 0 {
		opts.IdleTimeoutMs = 300000 // 5 minutes
	}
	if opts.HealthCheckIntervalMs <= 0 {
		opts.HealthCheckIntervalMs = 30000 // 30 seconds
	}

	return &Pool{
		connections: make(map[string]*pooledConn),
		opts:        opts,
	}
}

// Get returns a connection to the specified address.
// Creates a new connection if one doesn't exist or if the existing one is not ready.
// Returns an error if the connection cannot be established.
func (p *Pool) Get(ctx context.Context, address string) (*grpc.ClientConn, error) {
	// Fast path: try to get existing connection
	p.mu.RLock()
	pc, exists := p.connections[address]
	p.mu.RUnlock()

	if exists {
		pc.mu.Lock()
		pc.lastUsed = time.Now()
		pc.mu.Unlock()

		// Check if connection is in a good state
		state := pc.conn.GetState()
		if state == connectivity.Ready || state == connectivity.Idle {
			return pc.conn, nil
		}

		// Connection is in a bad state (e.g., TransientFailure, Shutdown)
		// Try to reconnect
		p.removeConnection(address)
	}

	// Slow path: create new connection
	return p.createConnection(ctx, address)
}

// createConnection establishes a new gRPC connection to the specified address.
func (p *Pool) createConnection(ctx context.Context, address string) (*grpc.ClientConn, error) {
	p.mu.Lock()
	defer p.mu.Unlock()

	// Double-check if another goroutine created the connection
	if pc, exists := p.connections[address]; exists {
		state := pc.conn.GetState()
		if state == connectivity.Ready || state == connectivity.Idle {
			return pc.conn, nil
		}
		// Connection is bad, remove it
		p.removeConnectionLocked(address)
	}

	// Create dial context with timeout
	dialCtx, cancel := context.WithTimeout(ctx, time.Duration(p.opts.DialTimeoutMs)*time.Millisecond)
	defer cancel()

	// Dial options
	dialOpts := []grpc.DialOption{
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithBlock(), // Wait for connection to be ready
	}

	// Establish connection
	conn, err := grpc.DialContext(dialCtx, address, dialOpts...)
	if err != nil {
		return nil, fmt.Errorf("failed to dial %s: %w", address, err)
	}

	// Create pooled connection
	pc := &pooledConn{
		conn:       conn,
		address:    address,
		lastUsed:   time.Now(),
		stopHealth: make(chan struct{}),
	}

	// Start health checking if enabled
	if p.opts.HealthCheckIntervalMs > 0 {
		pc.healthTicker = time.NewTicker(time.Duration(p.opts.HealthCheckIntervalMs) * time.Millisecond)
		go p.healthCheckLoop(pc)
	}

	// Store in pool
	p.connections[address] = pc

	return conn, nil
}

// healthCheckLoop periodically checks the health of a connection.
// Removes the connection from the pool if it becomes unhealthy.
func (p *Pool) healthCheckLoop(pc *pooledConn) {
	for {
		select {
		case <-pc.healthTicker.C:
			state := pc.conn.GetState()

			// Check if connection is in a bad state
			if state == connectivity.TransientFailure || state == connectivity.Shutdown {
				// Remove from pool
				p.removeConnection(pc.address)
				return
			}

			// Check idle timeout
			pc.mu.Lock()
			idleDuration := time.Since(pc.lastUsed)
			pc.mu.Unlock()

			if p.opts.IdleTimeoutMs > 0 && idleDuration > time.Duration(p.opts.IdleTimeoutMs)*time.Millisecond {
				// Connection has been idle too long, remove it
				p.removeConnection(pc.address)
				return
			}

		case <-pc.stopHealth:
			return
		}
	}
}

// removeConnection removes a connection from the pool and closes it.
func (p *Pool) removeConnection(address string) {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.removeConnectionLocked(address)
}

// removeConnectionLocked removes a connection from the pool (caller must hold lock).
func (p *Pool) removeConnectionLocked(address string) {
	pc, exists := p.connections[address]
	if !exists {
		return
	}

	// Stop health checking
	if pc.healthTicker != nil {
		pc.healthTicker.Stop()
		close(pc.stopHealth)
	}

	// Close connection
	_ = pc.conn.Close()

	// Remove from pool
	delete(p.connections, address)
}

// Remove removes a connection from the pool by address.
// Useful for handling NOT_LEADER errors where we want to force reconnection.
func (p *Pool) Remove(address string) {
	p.removeConnection(address)
}

// Close closes all connections in the pool.
// Should be called when the client is shutting down.
func (p *Pool) Close() error {
	p.mu.Lock()
	defer p.mu.Unlock()

	for address := range p.connections {
		p.removeConnectionLocked(address)
	}

	return nil
}

// Size returns the number of connections in the pool.
func (p *Pool) Size() int {
	p.mu.RLock()
	defer p.mu.RUnlock()
	return len(p.connections)
}

// Addresses returns the addresses of all connections in the pool.
func (p *Pool) Addresses() []string {
	p.mu.RLock()
	defer p.mu.RUnlock()

	addresses := make([]string, 0, len(p.connections))
	for addr := range p.connections {
		addresses = append(addresses, addr)
	}
	return addresses
}

// GetState returns the connectivity state of the connection to the specified address.
// Returns connectivity.Shutdown if no connection exists.
func (p *Pool) GetState(address string) connectivity.State {
	p.mu.RLock()
	pc, exists := p.connections[address]
	p.mu.RUnlock()

	if !exists {
		return connectivity.Shutdown
	}

	return pc.conn.GetState()
}
