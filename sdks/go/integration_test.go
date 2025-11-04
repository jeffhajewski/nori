package norikv

import (
	"context"
	"testing"
	"time"

	"github.com/norikv/norikv-go/testing/ephemeral"
)

// TestIntegrationPutGet tests basic Put and Get operations against a live server.
func TestIntegrationPutGet(t *testing.T) {
	// Start ephemeral server
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{
		TotalShards: 128,
	})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	// Create client
	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.TotalShards = 128
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	// Test Put
	key := []byte("test-key")
	value := []byte("test-value")

	version, err := client.Put(ctx, key, value, nil)
	if err != nil {
		t.Fatalf("Put failed: %v", err)
	}
	if version == nil {
		t.Fatal("Expected version to be non-nil")
	}
	if version.Term == 0 || version.Index == 0 {
		t.Errorf("Expected non-zero version, got term=%d index=%d", version.Term, version.Index)
	}

	// Test Get
	result, err := client.Get(ctx, key, nil)
	if err != nil {
		t.Fatalf("Get failed: %v", err)
	}
	if string(result.Value) != string(value) {
		t.Errorf("Expected value %s, got %s", value, result.Value)
	}
	if result.Version.Term != version.Term || result.Version.Index != version.Index {
		t.Errorf("Version mismatch: put=%v get=%v", version, result.Version)
	}
}

// TestIntegrationDelete tests delete operations.
func TestIntegrationDelete(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	key := []byte("delete-test")
	value := []byte("to-be-deleted")

	// Put a value
	_, err = client.Put(ctx, key, value, nil)
	if err != nil {
		t.Fatalf("Put failed: %v", err)
	}

	// Verify it exists
	_, err = client.Get(ctx, key, nil)
	if err != nil {
		t.Fatalf("Get failed: %v", err)
	}

	// Delete it
	err = client.Delete(ctx, key, nil)
	if err != nil {
		t.Fatalf("Delete failed: %v", err)
	}

	// Verify it's gone
	_, err = client.Get(ctx, key, nil)
	if err == nil {
		t.Error("Expected error for deleted key, got nil")
	}
}

// TestIntegrationConditionalPut tests conditional put with version matching.
func TestIntegrationConditionalPut(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	key := []byte("cas-test")
	value1 := []byte("value-1")
	value2 := []byte("value-2")

	// Initial put
	v1, err := client.Put(ctx, key, value1, nil)
	if err != nil {
		t.Fatalf("Initial put failed: %v", err)
	}

	// Conditional put with correct version should succeed
	v2, err := client.Put(ctx, key, value2, &PutOptions{
		IfMatchVersion: v1,
	})
	if err != nil {
		t.Fatalf("Conditional put failed: %v", err)
	}
	if v2.Index <= v1.Index {
		t.Error("Expected version to increment")
	}

	// Conditional put with wrong version should fail
	_, err = client.Put(ctx, key, []byte("value-3"), &PutOptions{
		IfMatchVersion: v1, // Old version
	})
	if err == nil {
		t.Error("Expected error for mismatched version")
	}
}

// TestIntegrationTTL tests TTL expiration.
func TestIntegrationTTL(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	key := []byte("ttl-test")
	value := []byte("expires-soon")
	ttlMs := uint64(200) // 200ms

	// Put with TTL
	_, err = client.Put(ctx, key, value, &PutOptions{
		TTLMs: &ttlMs,
	})
	if err != nil {
		t.Fatalf("Put with TTL failed: %v", err)
	}

	// Should exist immediately
	_, err = client.Get(ctx, key, nil)
	if err != nil {
		t.Fatalf("Get failed: %v", err)
	}

	// Wait for expiration
	time.Sleep(250 * time.Millisecond)

	// Should be gone
	_, err = client.Get(ctx, key, nil)
	if err == nil {
		t.Error("Expected error for expired key")
	}
}

// TestIntegrationMultipleKeys tests operations on multiple keys.
func TestIntegrationMultipleKeys(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{
		TotalShards: 128,
	})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.TotalShards = 128
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	// Write 100 keys
	numKeys := 100
	for i := 0; i < numKeys; i++ {
		key := []byte{byte(i)}
		value := []byte{byte(i * 2)}

		_, err := client.Put(ctx, key, value, nil)
		if err != nil {
			t.Fatalf("Put failed for key %d: %v", i, err)
		}
	}

	// Read them all back
	for i := 0; i < numKeys; i++ {
		key := []byte{byte(i)}
		expectedValue := []byte{byte(i * 2)}

		result, err := client.Get(ctx, key, nil)
		if err != nil {
			t.Fatalf("Get failed for key %d: %v", i, err)
		}
		if string(result.Value) != string(expectedValue) {
			t.Errorf("Value mismatch for key %d: expected %v, got %v", i, expectedValue, result.Value)
		}
	}

	// Check server stats
	stats := server.GetStats()
	if stats.TotalEntries != numKeys {
		t.Errorf("Expected %d entries, got %d", numKeys, stats.TotalEntries)
	}
	if stats.ActiveShards == 0 {
		t.Error("Expected at least one active shard")
	}
}

// TestIntegrationConsistencyLevels tests different consistency levels.
func TestIntegrationConsistencyLevels(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	key := []byte("consistency-test")
	value := []byte("test-value")

	// Put a value
	_, err = client.Put(ctx, key, value, nil)
	if err != nil {
		t.Fatalf("Put failed: %v", err)
	}

	// Test different consistency levels
	consistencyLevels := []ConsistencyLevel{
		ConsistencyLease,
		ConsistencyLinearizable,
		ConsistencyStaleOK,
	}

	for _, level := range consistencyLevels {
		t.Run(string(level), func(t *testing.T) {
			result, err := client.Get(ctx, key, &GetOptions{
				Consistency: level,
			})
			if err != nil {
				t.Fatalf("Get with %s failed: %v", level, err)
			}
			if string(result.Value) != string(value) {
				t.Errorf("Value mismatch with %s", level)
			}
		})
	}
}

// TestIntegrationIdempotency tests idempotency key handling.
func TestIntegrationIdempotency(t *testing.T) {
	server, err := ephemeral.NewServer(ephemeral.ServerOptions{})
	if err != nil {
		t.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	ctx := context.Background()
	config := DefaultClientConfig([]string{server.Address()})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		t.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	key := []byte("idem-test")
	value := []byte("idempotent-value")
	idempotencyKey := "unique-request-id-123"

	// First put with idempotency key
	v1, err := client.Put(ctx, key, value, &PutOptions{
		IdempotencyKey: idempotencyKey,
	})
	if err != nil {
		t.Fatalf("First put failed: %v", err)
	}

	// Note: The current simple in-memory server doesn't implement
	// idempotency deduplication. This test verifies the key is accepted
	// and passed through the protocol.
	_ = v1

	// Second put with same key should work (no dedup in simple server)
	_, err = client.Put(ctx, key, value, &PutOptions{
		IdempotencyKey: idempotencyKey,
	})
	if err != nil {
		t.Fatalf("Second put failed: %v", err)
	}
}
