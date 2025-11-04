// Package topology implements cluster topology management for NoriKV client.
package topology

import (
	"context"
	"sync"
	"time"
)

// TopologyEvent represents a change in cluster topology.
type TopologyEvent struct {
	Epoch         uint64            // Current epoch
	AddedNodes    []string          // Nodes that were added
	RemovedNodes  []string          // Nodes that were removed
	LeaderChanges map[uint32]string // Shard ID -> new leader address
}

// Manager tracks cluster topology and notifies subscribers of changes.
// Thread-safe for concurrent access.
type Manager struct {
	mu    sync.RWMutex
	epoch uint64
	nodes map[string]bool // node address -> active

	// Event broadcasting
	eventMu     sync.RWMutex
	subscribers []chan TopologyEvent

	// Lifecycle management
	ctx     context.Context
	cancel  context.CancelFunc
	done    chan struct{}
	started bool
	closeMu sync.Mutex
}

// ManagerOptions contains configuration for the topology manager.
type ManagerOptions struct {
	// PollIntervalMs is the interval for polling cluster state (default: 30000)
	// In a full implementation, this would use gRPC streaming instead.
	PollIntervalMs int

	// InitialNodes is the list of initial node addresses
	InitialNodes []string
}

// NewManager creates a new topology manager.
func NewManager(ctx context.Context, opts ManagerOptions) *Manager {
	if opts.PollIntervalMs <= 0 {
		opts.PollIntervalMs = 30000 // 30 seconds
	}

	mgrCtx, cancel := context.WithCancel(ctx)

	m := &Manager{
		epoch:       0,
		nodes:       make(map[string]bool),
		subscribers: make([]chan TopologyEvent, 0),
		ctx:         mgrCtx,
		cancel:      cancel,
		done:        make(chan struct{}),
	}

	// Initialize with initial nodes
	for _, node := range opts.InitialNodes {
		m.nodes[node] = true
	}

	return m
}

// Start starts the topology manager's background tasks.
// In a full implementation, this would establish a gRPC streaming connection
// to watch cluster changes. For now, it's a no-op placeholder.
func (m *Manager) Start() error {
	m.closeMu.Lock()
	defer m.closeMu.Unlock()

	if !m.started {
		m.started = true
		go m.run()
	}
	return nil
}

// run is the main event loop (placeholder for future streaming implementation).
func (m *Manager) run() {
	defer close(m.done)

	// In a full implementation, this would:
	// 1. Establish gRPC stream to server for cluster view updates
	// 2. Parse updates and emit TopologyEvents
	// 3. Handle reconnection on stream errors
	//
	// For now, this is a placeholder that just waits for cancellation.

	<-m.ctx.Done()
}

// GetEpoch returns the current cluster epoch.
func (m *Manager) GetEpoch() uint64 {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.epoch
}

// GetNodes returns the current list of active node addresses.
func (m *Manager) GetNodes() []string {
	m.mu.RLock()
	defer m.mu.RUnlock()

	nodes := make([]string, 0, len(m.nodes))
	for addr, active := range m.nodes {
		if active {
			nodes = append(nodes, addr)
		}
	}
	return nodes
}

// IsNodeActive returns whether the given node is currently active.
func (m *Manager) IsNodeActive(address string) bool {
	m.mu.RLock()
	defer m.mu.RUnlock()
	return m.nodes[address]
}

// UpdateNodes updates the cluster membership.
// This is used for manual updates when not using streaming.
func (m *Manager) UpdateNodes(nodes []string) {
	m.mu.Lock()

	oldNodes := make(map[string]bool, len(m.nodes))
	for addr, active := range m.nodes {
		oldNodes[addr] = active
	}

	// Update node map
	newNodeMap := make(map[string]bool)
	for _, addr := range nodes {
		newNodeMap[addr] = true
	}
	m.nodes = newNodeMap
	m.epoch++

	epoch := m.epoch
	m.mu.Unlock()

	// Detect changes
	var added, removed []string
	for addr := range newNodeMap {
		if !oldNodes[addr] {
			added = append(added, addr)
		}
	}
	for addr := range oldNodes {
		if !newNodeMap[addr] {
			removed = append(removed, addr)
		}
	}

	// Emit event if there were changes
	if len(added) > 0 || len(removed) > 0 {
		event := TopologyEvent{
			Epoch:         epoch,
			AddedNodes:    added,
			RemovedNodes:  removed,
			LeaderChanges: make(map[uint32]string),
		}
		m.emitEvent(event)
	}
}

// MarkNodeInactive marks a node as inactive (e.g., after connection failures).
func (m *Manager) MarkNodeInactive(address string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	if active, exists := m.nodes[address]; exists && active {
		m.nodes[address] = false
		m.epoch++
	}
}

// MarkNodeActive marks a node as active (e.g., after successful reconnection).
func (m *Manager) MarkNodeActive(address string) {
	m.mu.Lock()
	defer m.mu.Unlock()

	if active, exists := m.nodes[address]; exists && !active {
		m.nodes[address] = true
		m.epoch++
	}
}

// Subscribe subscribes to topology change events.
// The returned channel will receive events when cluster topology changes.
// Caller should not close the channel.
func (m *Manager) Subscribe() <-chan TopologyEvent {
	ch := make(chan TopologyEvent, 10) // Buffer to avoid blocking

	m.eventMu.Lock()
	m.subscribers = append(m.subscribers, ch)
	m.eventMu.Unlock()

	return ch
}

// Unsubscribe removes a subscription.
func (m *Manager) Unsubscribe(ch <-chan TopologyEvent) {
	m.eventMu.Lock()
	defer m.eventMu.Unlock()

	// Find and remove the channel
	for i, sub := range m.subscribers {
		if sub == ch {
			// Remove by swapping with last element
			m.subscribers[i] = m.subscribers[len(m.subscribers)-1]
			m.subscribers = m.subscribers[:len(m.subscribers)-1]
			break
		}
	}
}

// emitEvent sends an event to all subscribers.
func (m *Manager) emitEvent(event TopologyEvent) {
	m.eventMu.RLock()
	subscribers := make([]chan TopologyEvent, len(m.subscribers))
	copy(subscribers, m.subscribers)
	m.eventMu.RUnlock()

	// Send to all subscribers (non-blocking)
	for _, ch := range subscribers {
		select {
		case ch <- event:
		case <-time.After(100 * time.Millisecond):
			// Skip slow subscriber
		}
	}
}

// Close stops the topology manager and cleans up resources.
func (m *Manager) Close() error {
	m.closeMu.Lock()
	alreadyStarted := m.started
	m.closeMu.Unlock()

	m.cancel()

	// Wait for background goroutine to finish (if started)
	if alreadyStarted {
		<-m.done
	}

	// Close all subscriber channels
	m.eventMu.Lock()
	for _, ch := range m.subscribers {
		close(ch)
	}
	m.subscribers = nil
	m.eventMu.Unlock()

	return nil
}

// WaitForEpoch waits until the cluster reaches the specified epoch.
// Returns error if context is canceled before epoch is reached.
func (m *Manager) WaitForEpoch(ctx context.Context, targetEpoch uint64) error {
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()

	for {
		m.mu.RLock()
		currentEpoch := m.epoch
		m.mu.RUnlock()

		if currentEpoch >= targetEpoch {
			return nil
		}

		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
			// Continue polling
		}
	}
}
