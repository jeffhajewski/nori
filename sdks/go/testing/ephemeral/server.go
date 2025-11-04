// Package ephemeral provides an in-process NoriKV server for testing.
package ephemeral

import (
	"context"
	"fmt"
	"net"
	"sync"

	"github.com/norikv/norikv-go/hash"
	"github.com/norikv/norikv-go/proto"
	"github.com/norikv/norikv-go/testing/memstore"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// Server is an ephemeral NoriKV server for testing.
// It implements the gRPC KV service with in-memory storage.
type Server struct {
	proto.UnimplementedKvServer

	mu          sync.RWMutex
	stores      map[uint32]*memstore.Store // shard ID -> store
	totalShards int
	grpcServer  *grpc.Server
	listener    net.Listener
	address     string
}

// ServerOptions configures the ephemeral server.
type ServerOptions struct {
	// TotalShards is the number of virtual shards (default: 1024)
	TotalShards int

	// Address to bind to (default: "localhost:0" for random port)
	Address string
}

// NewServer creates and starts a new ephemeral server.
func NewServer(opts ServerOptions) (*Server, error) {
	if opts.TotalShards <= 0 {
		opts.TotalShards = 1024
	}
	if opts.Address == "" {
		opts.Address = "localhost:0"
	}

	// Create listener
	listener, err := net.Listen("tcp", opts.Address)
	if err != nil {
		return nil, fmt.Errorf("failed to listen: %w", err)
	}

	s := &Server{
		stores:      make(map[uint32]*memstore.Store),
		totalShards: opts.TotalShards,
		grpcServer:  grpc.NewServer(),
		listener:    listener,
		address:     listener.Addr().String(),
	}

	// Register gRPC service
	proto.RegisterKvServer(s.grpcServer, s)

	// Start serving in background
	go func() {
		_ = s.grpcServer.Serve(listener)
	}()

	return s, nil
}

// Address returns the server's listening address.
func (s *Server) Address() string {
	return s.address
}

// Stop stops the server and closes all resources.
func (s *Server) Stop() {
	if s.grpcServer != nil {
		s.grpcServer.Stop()
	}
	if s.listener != nil {
		s.listener.Close()
	}
}

// getStore returns the store for the given shard ID, creating it if necessary.
func (s *Server) getStore(shardID uint32) *memstore.Store {
	s.mu.RLock()
	store := s.stores[shardID]
	s.mu.RUnlock()

	if store != nil {
		return store
	}

	s.mu.Lock()
	defer s.mu.Unlock()

	// Check again after acquiring write lock
	if s.stores[shardID] != nil {
		return s.stores[shardID]
	}

	// Create new store
	store = memstore.NewStore(shardID)
	s.stores[shardID] = store
	return store
}

// getShardForKey determines which shard a key belongs to.
func (s *Server) getShardForKey(key []byte) uint32 {
	return uint32(hash.GetShardForKey(key, s.totalShards))
}

// Put implements the KV Put RPC.
func (s *Server) Put(ctx context.Context, req *proto.PutRequest) (*proto.PutResponse, error) {
	if len(req.Key) == 0 {
		return nil, status.Error(codes.InvalidArgument, "key cannot be empty")
	}

	// Get shard and store
	shardID := s.getShardForKey(req.Key)
	store := s.getStore(shardID)

	// Perform put operation
	version, err := store.Put(req.Key, req.Value, req.TtlMs, req.IfMatch)
	if err != nil {
		switch err.(type) {
		case *memstore.VersionMismatchError:
			return nil, status.Error(codes.FailedPrecondition, err.Error())
		default:
			return nil, status.Error(codes.Internal, err.Error())
		}
	}

	return &proto.PutResponse{
		Version: version,
		Meta:    make(map[string]string),
	}, nil
}

// Get implements the KV Get RPC.
func (s *Server) Get(ctx context.Context, req *proto.GetRequest) (*proto.GetResponse, error) {
	if len(req.Key) == 0 {
		return nil, status.Error(codes.InvalidArgument, "key cannot be empty")
	}

	// Get shard and store
	shardID := s.getShardForKey(req.Key)
	store := s.getStore(shardID)

	// Perform get operation
	entry, err := store.Get(req.Key)
	if err != nil {
		switch err.(type) {
		case *memstore.KeyNotFoundError:
			return nil, status.Error(codes.NotFound, "key not found")
		default:
			return nil, status.Error(codes.Internal, err.Error())
		}
	}

	return &proto.GetResponse{
		Value:   entry.Value,
		Version: entry.Version,
		Meta:    entry.Metadata,
	}, nil
}

// Delete implements the KV Delete RPC.
func (s *Server) Delete(ctx context.Context, req *proto.DeleteRequest) (*proto.DeleteResponse, error) {
	if len(req.Key) == 0 {
		return nil, status.Error(codes.InvalidArgument, "key cannot be empty")
	}

	// Get shard and store
	shardID := s.getShardForKey(req.Key)
	store := s.getStore(shardID)

	// Perform delete operation
	tombstoned, err := store.Delete(req.Key, req.IfMatch)
	if err != nil {
		switch err.(type) {
		case *memstore.VersionMismatchError:
			return nil, status.Error(codes.FailedPrecondition, err.Error())
		default:
			return nil, status.Error(codes.Internal, err.Error())
		}
	}

	return &proto.DeleteResponse{
		Tombstoned: tombstoned,
	}, nil
}

// Stats returns statistics about the server.
type Stats struct {
	TotalShards  int
	ActiveShards int
	TotalEntries int
}

// GetStats returns current server statistics.
func (s *Server) GetStats() Stats {
	s.mu.RLock()
	defer s.mu.RUnlock()

	totalEntries := 0
	for _, store := range s.stores {
		totalEntries += store.Size()
	}

	return Stats{
		TotalShards:  s.totalShards,
		ActiveShards: len(s.stores),
		TotalEntries: totalEntries,
	}
}

// Cleanup runs cleanup on all stores, removing expired entries.
func (s *Server) Cleanup() int {
	s.mu.RLock()
	stores := make([]*memstore.Store, 0, len(s.stores))
	for _, store := range s.stores {
		stores = append(stores, store)
	}
	s.mu.RUnlock()

	total := 0
	for _, store := range stores {
		total += store.Cleanup()
	}
	return total
}
