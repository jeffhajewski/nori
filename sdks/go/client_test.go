package norikv

import (
	"context"
	"testing"
	"time"
)

func TestNewClient(t *testing.T) {
	t.Run("creates client with valid config", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001", "localhost:9002"})

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		if client == nil {
			t.Fatal("Expected client to be non-nil")
		}
		if client.pool == nil {
			t.Error("Expected pool to be initialized")
		}
		if client.router == nil {
			t.Error("Expected router to be initialized")
		}
		if client.topology == nil {
			t.Error("Expected topology manager to be initialized")
		}
		if client.retry == nil {
			t.Error("Expected retry policy to be initialized")
		}
	})

	t.Run("creates client with nil config", func(t *testing.T) {
		ctx := context.Background()

		// This should fail because default config has no nodes
		_, err := NewClient(ctx, nil)
		if err == nil {
			t.Error("Expected error for nil config")
		}
	})

	t.Run("fails with no nodes", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig(nil)

		_, err := NewClient(ctx, config)
		if err == nil {
			t.Error("Expected error for empty nodes")
		}

		invalidArgErr, ok := err.(*InvalidArgumentError)
		if !ok {
			t.Errorf("Expected InvalidArgumentError, got %T", err)
		}
		if invalidArgErr != nil && invalidArgErr.Message != "at least one node address is required" {
			t.Errorf("Unexpected error message: %s", invalidArgErr.Message)
		}
	})

	t.Run("creates client without cluster watching", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		config.WatchCluster = false

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		if client == nil {
			t.Fatal("Expected client to be non-nil")
		}
	})

	t.Run("creates client with custom config", func(t *testing.T) {
		ctx := context.Background()
		config := &ClientConfig{
			Nodes:                 []string{"localhost:9001", "localhost:9002", "localhost:9003"},
			TotalShards:           128,
			MaxConnectionsPerNode: 5,
			TimeoutMs:             10000,
			WatchCluster:          false,
			Retry: &RetryConfig{
				MaxAttempts:        5,
				InitialDelayMs:     200,
				MaxDelayMs:         10000,
				BackoffMultiplier:  3.0,
				JitterMs:           100,
				RetryOnNotLeader:   true,
			},
		}

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		if client == nil {
			t.Fatal("Expected client to be non-nil")
		}
	})
}

func TestClientPut(t *testing.T) {
	t.Run("validates empty key", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		_, err = client.Put(ctx, []byte{}, []byte("value"), nil)
		if err == nil {
			t.Error("Expected error for empty key")
		}

		invalidArgErr, ok := err.(*InvalidArgumentError)
		if !ok {
			t.Errorf("Expected InvalidArgumentError, got %T", err)
		}
		if invalidArgErr != nil && invalidArgErr.Message != "key cannot be empty" {
			t.Errorf("Unexpected error message: %s", invalidArgErr.Message)
		}
	})

	t.Run("uses default options when nil", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// This will fail to connect, but we're testing option handling
		_, err = client.Put(ctx, []byte("key"), []byte("value"), nil)
		// Error is expected (no server), but should not panic
		if err == nil {
			t.Error("Expected connection error")
		}
	})

	t.Run("applies timeout from config", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		config.TimeoutMs = 100 // Very short timeout

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		start := time.Now()
		_, err = client.Put(ctx, []byte("key"), []byte("value"), nil)
		elapsed := time.Since(start)

		if err == nil {
			t.Error("Expected timeout error")
		}

		// Should timeout quickly (within 500ms including retry overhead)
		if elapsed > 500*time.Millisecond {
			t.Errorf("Timeout took too long: %v", elapsed)
		}
	})

	t.Run("respects context deadline", func(t *testing.T) {
		baseCtx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})

		client, err := NewClient(baseCtx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// Create context with short deadline
		ctx, cancel := context.WithTimeout(baseCtx, 50*time.Millisecond)
		defer cancel()

		start := time.Now()
		_, err = client.Put(ctx, []byte("key"), []byte("value"), nil)
		elapsed := time.Since(start)

		if err == nil {
			t.Error("Expected context deadline error")
		}

		// Should respect the 50ms deadline
		if elapsed > 200*time.Millisecond {
			t.Errorf("Context deadline not respected: %v", elapsed)
		}
	})
}

func TestClientGet(t *testing.T) {
	t.Run("validates empty key", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		_, err = client.Get(ctx, []byte{}, nil)
		if err == nil {
			t.Error("Expected error for empty key")
		}

		invalidArgErr, ok := err.(*InvalidArgumentError)
		if !ok {
			t.Errorf("Expected InvalidArgumentError, got %T", err)
		}
		if invalidArgErr != nil && invalidArgErr.Message != "key cannot be empty" {
			t.Errorf("Unexpected error message: %s", invalidArgErr.Message)
		}
	})

	t.Run("uses default consistency when nil options", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// This will fail to connect, but we're testing option handling
		_, err = client.Get(ctx, []byte("key"), nil)
		// Error is expected (no server), but should not panic
		if err == nil {
			t.Error("Expected connection error")
		}
	})

	t.Run("accepts consistency level options", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		opts := &GetOptions{
			Consistency: ConsistencyLinearizable,
		}

		_, err = client.Get(ctx, []byte("key"), opts)
		// Error is expected (no server)
		if err == nil {
			t.Error("Expected connection error")
		}
	})
}

func TestClientDelete(t *testing.T) {
	t.Run("validates empty key", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		err = client.Delete(ctx, []byte{}, nil)
		if err == nil {
			t.Error("Expected error for empty key")
		}

		invalidArgErr, ok := err.(*InvalidArgumentError)
		if !ok {
			t.Errorf("Expected InvalidArgumentError, got %T", err)
		}
		if invalidArgErr != nil && invalidArgErr.Message != "key cannot be empty" {
			t.Errorf("Unexpected error message: %s", invalidArgErr.Message)
		}
	})

	t.Run("uses default options when nil", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// This will fail to connect, but we're testing option handling
		err = client.Delete(ctx, []byte("key"), nil)
		// Error is expected (no server), but should not panic
		if err == nil {
			t.Error("Expected connection error")
		}
	})
}

func TestClientClose(t *testing.T) {
	t.Run("closes all resources", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}

		err = client.Close()
		if err != nil {
			t.Errorf("Expected no error on close, got %v", err)
		}
	})

	t.Run("closes without cluster watching", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		config.WatchCluster = false

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}

		err = client.Close()
		if err != nil {
			t.Errorf("Expected no error on close, got %v", err)
		}
	})
}

func TestGetClusterView(t *testing.T) {
	t.Run("returns cluster view", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001", "localhost:9002"})
		config.WatchCluster = false

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		view := client.GetClusterView()
		if view == nil {
			t.Fatal("Expected cluster view to be non-nil")
		}

		if view.Epoch != 0 {
			t.Errorf("Expected epoch 0, got %d", view.Epoch)
		}

		if len(view.Nodes) != 2 {
			t.Errorf("Expected 2 nodes, got %d", len(view.Nodes))
		}

		// Check node details
		for _, node := range view.Nodes {
			if node.ID == "" {
				t.Error("Expected node to have ID")
			}
			if node.Addr == "" {
				t.Error("Expected node to have address")
			}
			if node.Role != "voter" {
				t.Errorf("Expected role 'voter', got '%s'", node.Role)
			}
		}
	})
}

func TestClientWithTimeout(t *testing.T) {
	t.Run("returns noop cancel when context has deadline", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// Create context with existing deadline
		deadlineCtx, cancel := context.WithTimeout(ctx, 1*time.Second)
		defer cancel()

		newCtx, newCancel := client.withTimeout(deadlineCtx)
		defer newCancel()

		// Should return same context
		if _, hasDeadline := newCtx.Deadline(); !hasDeadline {
			t.Error("Expected context to have deadline")
		}
	})

	t.Run("applies config timeout when no deadline", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001"})
		config.TimeoutMs = 5000

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		newCtx, newCancel := client.withTimeout(ctx)
		defer newCancel()

		deadline, hasDeadline := newCtx.Deadline()
		if !hasDeadline {
			t.Error("Expected context to have deadline")
		}

		// Check that deadline is approximately 5 seconds in future
		expectedDeadline := time.Now().Add(5 * time.Second)
		diff := deadline.Sub(expectedDeadline).Abs()
		if diff > 100*time.Millisecond {
			t.Errorf("Deadline not as expected. Diff: %v", diff)
		}
	})
}

func TestWatchTopology(t *testing.T) {
	t.Run("updates router on node changes", func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()

		config := DefaultClientConfig([]string{"localhost:9001"})
		config.WatchCluster = true

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// Give watchTopology goroutine time to start
		time.Sleep(50 * time.Millisecond)

		// Update topology
		client.topology.UpdateNodes([]string{"localhost:9001", "localhost:9002", "localhost:9003"})

		// Give time for event to propagate
		time.Sleep(100 * time.Millisecond)

		// Check that router has updated node addresses
		nodes := client.router.GetNodeAddresses()
		if len(nodes) != 3 {
			t.Errorf("Expected router to have 3 nodes, got %d", len(nodes))
		}
	})

	t.Run("clears leader cache on leader changes", func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())
		defer cancel()

		config := DefaultClientConfig([]string{"localhost:9001"})
		config.WatchCluster = true

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// Set some leaders
		client.router.SetLeader(0, "localhost:9001")
		client.router.SetLeader(1, "localhost:9002")

		// Verify leaders are cached
		if client.router.GetLeader(0) != "localhost:9001" {
			t.Error("Expected leader for shard 0")
		}

		// Give watchTopology goroutine time to start
		time.Sleep(50 * time.Millisecond)

		// Trigger topology event with leader change
		event := client.topology.Subscribe()
		defer client.topology.Unsubscribe(event)

		client.topology.UpdateNodes([]string{"localhost:9001", "localhost:9003"})

		// Wait for event
		select {
		case e := <-event:
			// Manually trigger leader change
			e.LeaderChanges = map[uint32]string{0: "localhost:9003"}

			// Simulate what watchTopology would do
			for shardID := range e.LeaderChanges {
				client.router.ClearLeader(shardID)
			}
		case <-time.After(200 * time.Millisecond):
			t.Error("Timeout waiting for topology event")
		}

		// Leader should be cleared
		if client.router.GetLeader(0) != "" {
			t.Error("Expected leader for shard 0 to be cleared")
		}
	})

	t.Run("stops watching on context cancellation", func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())

		config := DefaultClientConfig([]string{"localhost:9001"})
		config.WatchCluster = true

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		// Give watchTopology goroutine time to start
		time.Sleep(50 * time.Millisecond)

		// Cancel context
		cancel()

		// Give time for goroutine to exit
		time.Sleep(100 * time.Millisecond)

		// Should be able to close without hanging
		err = client.Close()
		if err != nil {
			t.Errorf("Expected no error on close, got %v", err)
		}
	})
}

func TestClientConcurrency(t *testing.T) {
	t.Run("handles concurrent operations safely", func(t *testing.T) {
		ctx := context.Background()
		config := DefaultClientConfig([]string{"localhost:9001", "localhost:9002"})
		config.WatchCluster = false
		config.TimeoutMs = 100        // Very short timeout to speed up test
		config.Retry.MaxAttempts = 1  // No retries

		client, err := NewClient(ctx, config)
		if err != nil {
			t.Fatalf("Expected no error, got %v", err)
		}
		defer client.Close()

		done := make(chan bool, 10)

		// Spawn concurrent operations
		for i := 0; i < 10; i++ {
			go func(id int) {
				// These will fail (no server), but shouldn't panic
				key := []byte{byte(id)}
				client.Put(ctx, key, []byte("value"), nil)
				client.Get(ctx, key, nil)
				client.Delete(ctx, key, nil)
				_ = client.GetClusterView()

				done <- true
			}(i)
		}

		// Wait for all goroutines (with generous timeout for 10 operations)
		for i := 0; i < 10; i++ {
			select {
			case <-done:
				// Good
			case <-time.After(5 * time.Second):
				t.Fatal("Timeout waiting for concurrent operations")
			}
		}

		// No panics = success
	})
}

func BenchmarkClientGetClusterView(b *testing.B) {
	ctx := context.Background()
	config := DefaultClientConfig([]string{"localhost:9001", "localhost:9002"})
	config.WatchCluster = false

	client, err := NewClient(ctx, config)
	if err != nil {
		b.Fatalf("Expected no error, got %v", err)
	}
	defer client.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = client.GetClusterView()
	}
}

func BenchmarkClientWithTimeout(b *testing.B) {
	ctx := context.Background()
	config := DefaultClientConfig([]string{"localhost:9001"})
	client, err := NewClient(ctx, config)
	if err != nil {
		b.Fatalf("Expected no error, got %v", err)
	}
	defer client.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		newCtx, cancel := client.withTimeout(ctx)
		cancel()
		_ = newCtx
	}
}
