// Package norikv provides a Go client for NoriKV.
package norikv

import (
	"github.com/norikv/norikv-go/proto"
)

// ConsistencyLevel specifies the consistency guarantees for read operations.
type ConsistencyLevel string

const (
	// ConsistencyLease uses lease-based linearizable reads (fast, default).
	// The leader serves reads from its lease without quorum confirmation.
	ConsistencyLease ConsistencyLevel = "lease"

	// ConsistencyLinearizable uses strict linearizable reads with read-index protocol.
	// Requires quorum confirmation for guaranteed up-to-date reads.
	ConsistencyLinearizable ConsistencyLevel = "linearizable"

	// ConsistencyStaleOK allows stale reads from any replica (fastest).
	// May return stale data but provides highest throughput.
	ConsistencyStaleOK ConsistencyLevel = "stale_ok"
)

// Version identifies a specific version of a key-value pair.
// Corresponds to the Raft log entry where it was committed.
type Version struct {
	Term  uint64 // Raft term when this version was written
	Index uint64 // Raft log index
}

// Equal returns true if two versions are equal.
func (v *Version) Equal(other *Version) bool {
	if v == nil || other == nil {
		return v == other
	}
	return v.Term == other.Term && v.Index == other.Index
}

// FromProto converts a protobuf Version to a Go Version.
func (v *Version) FromProto(pv *proto.Version) *Version {
	if pv == nil {
		return nil
	}
	return &Version{
		Term:  pv.Term,
		Index: pv.Index,
	}
}

// ToProto converts a Go Version to a protobuf Version.
func (v *Version) ToProto() *proto.Version {
	if v == nil {
		return nil
	}
	return &proto.Version{
		Term:  v.Term,
		Index: v.Index,
	}
}

// PutOptions contains options for Put operations.
type PutOptions struct {
	// TTL in milliseconds. 0 or nil means no expiration.
	TTLMs *uint64

	// IfNotExists only writes if the key doesn't exist.
	// Returns AlreadyExistsError if key exists.
	IfNotExists bool

	// IfMatchVersion only writes if the current version matches.
	// Used for optimistic concurrency control (CAS).
	IfMatchVersion *Version

	// IdempotencyKey for safe retries.
	// Same key + idempotency_key always produces the same result.
	IdempotencyKey string
}

// GetOptions contains options for Get operations.
type GetOptions struct {
	// Consistency level for this read (default: ConsistencyLease).
	Consistency ConsistencyLevel
}

// DeleteOptions contains options for Delete operations.
type DeleteOptions struct {
	// IfMatchVersion only deletes if the current version matches.
	// Used for optimistic concurrency control (CAS).
	IfMatchVersion *Version

	// IdempotencyKey for safe retries.
	IdempotencyKey string
}

// GetResult contains the result of a Get operation.
type GetResult struct {
	// Value is the retrieved value, or nil if key doesn't exist.
	Value []byte

	// Version of the value, or nil if key doesn't exist.
	Version *Version

	// Metadata from the server (optional).
	Metadata map[string]string
}

// ClusterNode represents a node in the cluster.
type ClusterNode struct {
	ID   string // Node identifier
	Addr string // Node address (host:port)
	Role string // Node role (e.g., "voter", "learner")
}

// ShardReplica represents a replica of a shard.
type ShardReplica struct {
	NodeID   string // Node hosting this replica
	IsLeader bool   // True if this replica is the leader
}

// ShardInfo contains information about a shard.
type ShardInfo struct {
	ID       uint32         // Shard identifier
	Replicas []ShardReplica // Replicas for this shard
}

// ClusterView represents the current state of the cluster.
type ClusterView struct {
	Epoch  uint64        // Cluster epoch (increments on topology changes)
	Nodes  []ClusterNode // All nodes in the cluster
	Shards []ShardInfo   // Shard assignments
}

// TopologyEvent represents a cluster topology change event.
type TopologyEvent struct {
	PreviousEpoch uint64                 // Previous epoch
	CurrentEpoch  uint64                 // New epoch
	AddedNodes    []string               // Node IDs that were added
	RemovedNodes  []string               // Node IDs that were removed
	LeaderChanges map[uint32]string      // Shard ID -> new leader node ID
}

// ClientConfig contains configuration for the NoriKV client.
type ClientConfig struct {
	// Nodes is the list of node addresses to connect to (at least one required).
	Nodes []string

	// TotalShards is the total number of virtual shards (default: 1024).
	TotalShards int

	// Timeout is the default request timeout in milliseconds (default: 5000).
	TimeoutMs int

	// Retry policy configuration.
	Retry *RetryConfig

	// WatchCluster enables cluster view watching (default: true).
	WatchCluster bool

	// MaxConnectionsPerNode is the maximum gRPC connections per node (default: 10).
	MaxConnectionsPerNode int
}

// RetryConfig contains retry policy configuration.
type RetryConfig struct {
	// MaxAttempts is the maximum number of retry attempts (default: 3).
	MaxAttempts int

	// InitialDelayMs is the initial delay in milliseconds (default: 10).
	InitialDelayMs int

	// MaxDelayMs is the maximum delay in milliseconds (default: 1000).
	MaxDelayMs int

	// BackoffMultiplier is the backoff multiplier (default: 2).
	BackoffMultiplier float64

	// JitterMs is the jitter in milliseconds (default: 100).
	JitterMs int

	// RetryOnNotLeader enables retry on NOT_LEADER errors (default: true).
	RetryOnNotLeader bool
}

// DefaultRetryConfig returns the default retry configuration.
func DefaultRetryConfig() *RetryConfig {
	return &RetryConfig{
		MaxAttempts:       3,
		InitialDelayMs:    10,
		MaxDelayMs:        1000,
		BackoffMultiplier: 2.0,
		JitterMs:          100,
		RetryOnNotLeader:  true,
	}
}

// DefaultClientConfig returns the default client configuration.
func DefaultClientConfig(nodes []string) *ClientConfig {
	return &ClientConfig{
		Nodes:                 nodes,
		TotalShards:           1024,
		TimeoutMs:             5000,
		Retry:                 DefaultRetryConfig(),
		WatchCluster:          true,
		MaxConnectionsPerNode: 10,
	}
}
