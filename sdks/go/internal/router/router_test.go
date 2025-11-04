package router

import (
	"context"
	"testing"

	"github.com/norikv/norikv-go/internal/conn"
	"google.golang.org/grpc/metadata"
)

func TestNewRouter(t *testing.T) {
	t.Run("creates router with configuration", func(t *testing.T) {
		pool := conn.NewPool(conn.PoolOptions{})
		defer pool.Close()

		nodes := []string{"localhost:9001", "localhost:9002", "localhost:9003"}
		router := NewRouter(pool, 1024, nodes)

		if router.TotalShards() != 1024 {
			t.Errorf("Expected totalShards=1024, got %d", router.TotalShards())
		}

		addresses := router.GetNodeAddresses()
		if len(addresses) != 3 {
			t.Errorf("Expected 3 node addresses, got %d", len(addresses))
		}
	})
}

func TestGetShardForKey(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	t.Run("returns consistent shard for same key", func(t *testing.T) {
		key := []byte("test-key")
		shard1 := router.GetShardForKey(key)
		shard2 := router.GetShardForKey(key)

		if shard1 != shard2 {
			t.Errorf("Expected consistent shard assignment, got %d and %d", shard1, shard2)
		}

		if shard1 >= 1024 {
			t.Errorf("Expected shard < 1024, got %d", shard1)
		}
	})

	t.Run("returns different shards for different keys", func(t *testing.T) {
		// This test might fail occasionally if keys hash to same shard,
		// but with 1024 shards, probability is low
		key1 := []byte("key1")
		key2 := []byte("key2")
		key3 := []byte("key3")

		shard1 := router.GetShardForKey(key1)
		shard2 := router.GetShardForKey(key2)
		shard3 := router.GetShardForKey(key3)

		// At least one should be different
		if shard1 == shard2 && shard2 == shard3 {
			t.Error("Expected different shards for different keys")
		}
	})
}

func TestLeaderCache(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	t.Run("caches and retrieves leader", func(t *testing.T) {
		shardID := uint32(42)
		address := "localhost:9001"

		// Initially no leader
		leader := router.GetLeader(shardID)
		if leader != "" {
			t.Errorf("Expected no leader, got %s", leader)
		}

		// Set leader
		router.SetLeader(shardID, address)

		// Retrieve leader
		leader = router.GetLeader(shardID)
		if leader != address {
			t.Errorf("Expected leader=%s, got %s", address, leader)
		}
	})

	t.Run("clears leader", func(t *testing.T) {
		shardID := uint32(43)
		address := "localhost:9001"

		router.SetLeader(shardID, address)
		router.ClearLeader(shardID)

		leader := router.GetLeader(shardID)
		if leader != "" {
			t.Errorf("Expected no leader after clear, got %s", leader)
		}
	})

	t.Run("gets all leaders", func(t *testing.T) {
		router.SetLeader(1, "localhost:9001")
		router.SetLeader(2, "localhost:9002")
		router.SetLeader(3, "localhost:9003")

		leaders := router.GetAllLeaders()
		if len(leaders) < 3 {
			t.Errorf("Expected at least 3 leaders, got %d", len(leaders))
		}

		if leaders[1] != "localhost:9001" {
			t.Errorf("Expected leader for shard 1 = localhost:9001, got %s", leaders[1])
		}
	})

	t.Run("clears all leaders", func(t *testing.T) {
		router.SetLeader(1, "localhost:9001")
		router.SetLeader(2, "localhost:9002")

		router.ClearAllLeaders()

		leaders := router.GetAllLeaders()
		if len(leaders) != 0 {
			t.Errorf("Expected no leaders after clear all, got %d", len(leaders))
		}
	})
}

func TestUpdateNodeAddresses(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	t.Run("updates node addresses", func(t *testing.T) {
		newNodes := []string{"localhost:9001", "localhost:9002", "localhost:9003"}
		router.UpdateNodeAddresses(newNodes)

		addresses := router.GetNodeAddresses()
		if len(addresses) != 3 {
			t.Errorf("Expected 3 addresses after update, got %d", len(addresses))
		}
	})
}

func TestHandleNotLeaderError(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	t.Run("clears old leader and sets new hint", func(t *testing.T) {
		shardID := uint32(10)

		// Set initial leader
		router.SetLeader(shardID, "localhost:9001")

		// Simulate NOT_LEADER with hint
		router.HandleNotLeaderError(shardID, "localhost:9002")

		// Should have new leader
		leader := router.GetLeader(shardID)
		if leader != "localhost:9002" {
			t.Errorf("Expected leader=localhost:9002, got %s", leader)
		}
	})

	t.Run("clears leader when no hint", func(t *testing.T) {
		shardID := uint32(11)

		// Set initial leader
		router.SetLeader(shardID, "localhost:9001")

		// Simulate NOT_LEADER with no hint
		router.HandleNotLeaderError(shardID, "")

		// Should have no leader
		leader := router.GetLeader(shardID)
		if leader != "" {
			t.Errorf("Expected no leader, got %s", leader)
		}
	})
}

func TestExtractLeaderHint(t *testing.T) {
	t.Run("extracts hint from metadata", func(t *testing.T) {
		md := metadata.New(map[string]string{
			"leader-hint": "localhost:9002",
		})

		hint := ExtractLeaderHint(md)
		if hint != "localhost:9002" {
			t.Errorf("Expected hint=localhost:9002, got %s", hint)
		}
	})

	t.Run("returns empty string when no hint", func(t *testing.T) {
		md := metadata.New(map[string]string{})

		hint := ExtractLeaderHint(md)
		if hint != "" {
			t.Errorf("Expected empty hint, got %s", hint)
		}
	})

	t.Run("returns empty string for nil metadata", func(t *testing.T) {
		hint := ExtractLeaderHint(nil)
		if hint != "" {
			t.Errorf("Expected empty hint, got %s", hint)
		}
	})
}

func TestGetConnection(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{
		DialTimeoutMs: 1000,
	})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"invalid-address:9999"})

	t.Run("returns error for unreachable nodes", func(t *testing.T) {
		ctx := context.Background()
		shardID := uint32(1)

		_, _, err := router.GetConnection(ctx, shardID)
		if err == nil {
			t.Error("Expected error for unreachable node, got nil")
		}
	})
}

func TestGetConnectionForKey(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{
		DialTimeoutMs: 1000,
	})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"invalid-address:9999"})

	t.Run("returns error for unreachable nodes", func(t *testing.T) {
		ctx := context.Background()
		key := []byte("test-key")

		_, _, shardID, err := router.GetConnectionForKey(ctx, key)
		if err == nil {
			t.Error("Expected error for unreachable node, got nil")
		}
		if shardID >= 1024 {
			t.Errorf("Expected shard < 1024, got %d", shardID)
		}
	})
}

func TestRouterConcurrency(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001", "localhost:9002"})

	t.Run("handles concurrent leader updates safely", func(t *testing.T) {
		done := make(chan bool, 10)

		// Launch multiple goroutines updating leaders
		for i := 0; i < 10; i++ {
			go func(id int) {
				shardID := uint32(id % 100)
				address := "localhost:9001"

				router.SetLeader(shardID, address)
				_ = router.GetLeader(shardID)
				router.ClearLeader(shardID)

				done <- true
			}(i)
		}

		// Wait for all goroutines
		for i := 0; i < 10; i++ {
			<-done
		}

		// No panics = success
	})

	t.Run("handles concurrent node address updates safely", func(t *testing.T) {
		done := make(chan bool, 5)

		for i := 0; i < 5; i++ {
			go func() {
				newNodes := []string{"localhost:9001", "localhost:9002", "localhost:9003"}
				router.UpdateNodeAddresses(newNodes)
				_ = router.GetNodeAddresses()

				done <- true
			}()
		}

		for i := 0; i < 5; i++ {
			<-done
		}

		// No panics = success
	})
}

func TestClose(t *testing.T) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	t.Run("closes router successfully", func(t *testing.T) {
		router.SetLeader(1, "localhost:9001")

		err := router.Close()
		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}

		// Leaders should be cleared
		leaders := router.GetAllLeaders()
		if len(leaders) != 0 {
			t.Errorf("Expected no leaders after close, got %d", len(leaders))
		}
	})
}

func BenchmarkGetShardForKey(b *testing.B) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})
	key := []byte("test-key")

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = router.GetShardForKey(key)
	}
}

func BenchmarkGetLeader(b *testing.B) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})
	router.SetLeader(42, "localhost:9001")

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = router.GetLeader(42)
	}
}

func BenchmarkSetLeader(b *testing.B) {
	pool := conn.NewPool(conn.PoolOptions{})
	defer pool.Close()

	router := NewRouter(pool, 1024, []string{"localhost:9001"})

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		router.SetLeader(uint32(i%100), "localhost:9001")
	}
}
