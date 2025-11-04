// Package memstore provides an in-memory storage backend for testing.
package memstore

import (
	"sync"
	"time"

	"github.com/norikv/norikv-go/proto"
)

// Entry represents a stored key-value pair with metadata.
type Entry struct {
	Key       []byte
	Value     []byte
	Version   *proto.Version
	ExpiresAt *time.Time
	Metadata  map[string]string
}

// Store is a thread-safe in-memory key-value store.
// Implements basic NoriKV semantics for testing purposes.
type Store struct {
	mu      sync.RWMutex
	data    map[string]*Entry // key (as string) -> entry
	term    uint64            // Current term (incremented on each write)
	index   uint64            // Current index (incremented on each write)
	shardID uint32            // Shard ID this store represents
}

// NewStore creates a new in-memory store for the given shard.
func NewStore(shardID uint32) *Store {
	return &Store{
		data:    make(map[string]*Entry),
		term:    1,
		index:   0,
		shardID: shardID,
	}
}

// Put writes or updates a key-value pair.
// Returns the new version and an error if conditions are not met.
func (s *Store) Put(key, value []byte, ttlMs uint64, ifMatch *proto.Version) (*proto.Version, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	keyStr := string(key)
	existing := s.data[keyStr]

	// Check if_match condition
	if ifMatch != nil {
		if existing == nil {
			return nil, &VersionMismatchError{
				Message: "key does not exist",
			}
		}
		if existing.Version.Term != ifMatch.Term || existing.Version.Index != ifMatch.Index {
			return nil, &VersionMismatchError{
				Message: "version mismatch",
			}
		}
	}

	// Increment index for new write
	s.index++

	// Create version
	version := &proto.Version{
		Term:  s.term,
		Index: s.index,
	}

	// Calculate expiration
	var expiresAt *time.Time
	if ttlMs > 0 {
		expiry := time.Now().Add(time.Duration(ttlMs) * time.Millisecond)
		expiresAt = &expiry
	}

	// Store entry
	s.data[keyStr] = &Entry{
		Key:       key,
		Value:     value,
		Version:   version,
		ExpiresAt: expiresAt,
		Metadata:  make(map[string]string),
	}

	return version, nil
}

// Get retrieves a value by key.
// Returns nil if the key doesn't exist or has expired.
func (s *Store) Get(key []byte) (*Entry, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()

	keyStr := string(key)
	entry := s.data[keyStr]

	if entry == nil {
		return nil, &KeyNotFoundError{
			Key: key,
		}
	}

	// Check if expired
	if entry.ExpiresAt != nil && time.Now().After(*entry.ExpiresAt) {
		return nil, &KeyNotFoundError{
			Key: key,
		}
	}

	return entry, nil
}

// Delete removes a key from the store.
// Returns true if the key was deleted, false if it didn't exist.
func (s *Store) Delete(key []byte, ifMatch *proto.Version) (bool, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	keyStr := string(key)
	existing := s.data[keyStr]

	if existing == nil {
		return false, nil
	}

	// Check if_match condition
	if ifMatch != nil {
		if existing.Version.Term != ifMatch.Term || existing.Version.Index != ifMatch.Index {
			return false, &VersionMismatchError{
				Message: "version mismatch",
			}
		}
	}

	delete(s.data, keyStr)
	return true, nil
}

// Cleanup removes expired entries from the store.
// Returns the number of entries removed.
func (s *Store) Cleanup() int {
	s.mu.Lock()
	defer s.mu.Unlock()

	now := time.Now()
	count := 0

	for keyStr, entry := range s.data {
		if entry.ExpiresAt != nil && now.After(*entry.ExpiresAt) {
			delete(s.data, keyStr)
			count++
		}
	}

	return count
}

// Size returns the number of entries in the store.
func (s *Store) Size() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.data)
}

// Clear removes all entries from the store.
func (s *Store) Clear() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.data = make(map[string]*Entry)
	s.index = 0
}

// GetShardID returns the shard ID this store represents.
func (s *Store) GetShardID() uint32 {
	return s.shardID
}

// VersionMismatchError indicates a version mismatch on conditional operation.
type VersionMismatchError struct {
	Message string
}

func (e *VersionMismatchError) Error() string {
	return e.Message
}

// KeyNotFoundError indicates a key was not found.
type KeyNotFoundError struct {
	Key []byte
}

func (e *KeyNotFoundError) Error() string {
	return "key not found"
}
