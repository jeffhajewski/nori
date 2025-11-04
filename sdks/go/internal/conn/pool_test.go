package conn

import (
	"context"
	"testing"
	"time"

	"google.golang.org/grpc/connectivity"
)

func TestNewPool(t *testing.T) {
	t.Run("creates pool with defaults", func(t *testing.T) {
		pool := NewPool(PoolOptions{})

		if pool.opts.MaxConnectionsPerNode != 1 {
			t.Errorf("Expected MaxConnectionsPerNode=1, got %d", pool.opts.MaxConnectionsPerNode)
		}
		if pool.opts.DialTimeoutMs != 5000 {
			t.Errorf("Expected DialTimeoutMs=5000, got %d", pool.opts.DialTimeoutMs)
		}
		if pool.opts.IdleTimeoutMs != 300000 {
			t.Errorf("Expected IdleTimeoutMs=300000, got %d", pool.opts.IdleTimeoutMs)
		}
		if pool.opts.HealthCheckIntervalMs != 30000 {
			t.Errorf("Expected HealthCheckIntervalMs=30000, got %d", pool.opts.HealthCheckIntervalMs)
		}
		if pool.Size() != 0 {
			t.Errorf("Expected empty pool, got size %d", pool.Size())
		}
	})

	t.Run("creates pool with custom options", func(t *testing.T) {
		pool := NewPool(PoolOptions{
			MaxConnectionsPerNode: 5,
			DialTimeoutMs:         10000,
			IdleTimeoutMs:         600000,
			HealthCheckIntervalMs: 60000,
		})

		if pool.opts.MaxConnectionsPerNode != 5 {
			t.Errorf("Expected MaxConnectionsPerNode=5, got %d", pool.opts.MaxConnectionsPerNode)
		}
		if pool.opts.DialTimeoutMs != 10000 {
			t.Errorf("Expected DialTimeoutMs=10000, got %d", pool.opts.DialTimeoutMs)
		}
		if pool.opts.IdleTimeoutMs != 600000 {
			t.Errorf("Expected IdleTimeoutMs=600000, got %d", pool.opts.IdleTimeoutMs)
		}
		if pool.opts.HealthCheckIntervalMs != 60000 {
			t.Errorf("Expected HealthCheckIntervalMs=60000, got %d", pool.opts.HealthCheckIntervalMs)
		}
	})
}

func TestPoolGet(t *testing.T) {
	t.Run("returns error for invalid address", func(t *testing.T) {
		pool := NewPool(PoolOptions{
			DialTimeoutMs: 1000,
		})
		defer pool.Close()

		ctx := context.Background()
		_, err := pool.Get(ctx, "invalid-address:9999")

		if err == nil {
			t.Error("Expected error for invalid address, got nil")
		}
	})

	t.Run("respects context cancellation", func(t *testing.T) {
		pool := NewPool(PoolOptions{
			DialTimeoutMs: 10000, // Long timeout
		})
		defer pool.Close()

		ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
		defer cancel()

		_, err := pool.Get(ctx, "10.255.255.1:9999") // Non-routable address

		if err == nil {
			t.Error("Expected error due to context timeout, got nil")
		}
	})
}

func TestPoolSize(t *testing.T) {
	t.Run("returns correct size", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		if pool.Size() != 0 {
			t.Errorf("Expected size=0, got %d", pool.Size())
		}
	})
}

func TestPoolAddresses(t *testing.T) {
	t.Run("returns empty list for empty pool", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		addresses := pool.Addresses()
		if len(addresses) != 0 {
			t.Errorf("Expected 0 addresses, got %d", len(addresses))
		}
	})
}

func TestPoolGetState(t *testing.T) {
	t.Run("returns Shutdown for non-existent connection", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		state := pool.GetState("localhost:9999")
		if state != connectivity.Shutdown {
			t.Errorf("Expected Shutdown state, got %v", state)
		}
	})
}

func TestPoolRemove(t *testing.T) {
	t.Run("removes non-existent connection without error", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		// Should not panic or error
		pool.Remove("localhost:9999")

		if pool.Size() != 0 {
			t.Errorf("Expected size=0, got %d", pool.Size())
		}
	})
}

func TestPoolClose(t *testing.T) {
	t.Run("closes empty pool", func(t *testing.T) {
		pool := NewPool(PoolOptions{})

		err := pool.Close()
		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}

		if pool.Size() != 0 {
			t.Errorf("Expected size=0 after close, got %d", pool.Size())
		}
	})
}

func TestPoolConcurrency(t *testing.T) {
	t.Run("handles concurrent Get calls safely", func(t *testing.T) {
		pool := NewPool(PoolOptions{
			DialTimeoutMs: 1000,
		})
		defer pool.Close()

		ctx := context.Background()
		address := "invalid-address:9999"

		// Launch multiple goroutines trying to get the same connection
		done := make(chan bool, 5)
		for i := 0; i < 5; i++ {
			go func() {
				_, _ = pool.Get(ctx, address)
				done <- true
			}()
		}

		// Wait for all goroutines to complete
		for i := 0; i < 5; i++ {
			<-done
		}

		// Pool should remain consistent (no panics, no race conditions)
		// Size might be 0 or 1 depending on timing, but should not be > 1
		if pool.Size() > 1 {
			t.Errorf("Expected size <= 1, got %d", pool.Size())
		}
	})

	t.Run("handles concurrent Remove calls safely", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		address := "localhost:9999"

		// Launch multiple goroutines trying to remove the same connection
		done := make(chan bool, 5)
		for i := 0; i < 5; i++ {
			go func() {
				pool.Remove(address)
				done <- true
			}()
		}

		// Wait for all goroutines to complete
		for i := 0; i < 5; i++ {
			<-done
		}

		// Pool should remain consistent
		if pool.Size() != 0 {
			t.Errorf("Expected size=0, got %d", pool.Size())
		}
	})

	t.Run("handles concurrent Size calls safely", func(t *testing.T) {
		pool := NewPool(PoolOptions{})
		defer pool.Close()

		// Launch multiple goroutines reading the size
		done := make(chan bool, 10)
		for i := 0; i < 10; i++ {
			go func() {
				_ = pool.Size()
				done <- true
			}()
		}

		// Wait for all goroutines to complete
		for i := 0; i < 10; i++ {
			<-done
		}

		// No panics = success
	})
}

func BenchmarkPoolGet(b *testing.B) {
	pool := NewPool(PoolOptions{
		DialTimeoutMs: 1000,
	})
	defer pool.Close()

	ctx := context.Background()
	address := "invalid-address:9999"

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_, _ = pool.Get(ctx, address)
	}
}

func BenchmarkPoolSize(b *testing.B) {
	pool := NewPool(PoolOptions{})
	defer pool.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = pool.Size()
	}
}

func BenchmarkPoolGetState(b *testing.B) {
	pool := NewPool(PoolOptions{})
	defer pool.Close()

	address := "localhost:9999"

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = pool.GetState(address)
	}
}
