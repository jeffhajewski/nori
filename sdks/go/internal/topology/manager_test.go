package topology

import (
	"context"
	"testing"
	"time"
)

func TestNewManager(t *testing.T) {
	t.Run("creates manager with initial nodes", func(t *testing.T) {
		ctx := context.Background()
		nodes := []string{"localhost:9001", "localhost:9002", "localhost:9003"}
		mgr := NewManager(ctx, ManagerOptions{
			InitialNodes: nodes,
		})
		defer mgr.Close()

		if mgr.GetEpoch() != 0 {
			t.Errorf("Expected epoch=0, got %d", mgr.GetEpoch())
		}

		activeNodes := mgr.GetNodes()
		if len(activeNodes) != 3 {
			t.Errorf("Expected 3 nodes, got %d", len(activeNodes))
		}
	})

	t.Run("creates manager with default options", func(t *testing.T) {
		ctx := context.Background()
		mgr := NewManager(ctx, ManagerOptions{})
		defer mgr.Close()

		if mgr.GetEpoch() != 0 {
			t.Errorf("Expected epoch=0, got %d", mgr.GetEpoch())
		}

		activeNodes := mgr.GetNodes()
		if len(activeNodes) != 0 {
			t.Errorf("Expected 0 nodes, got %d", len(activeNodes))
		}
	})
}

func TestGetNodes(t *testing.T) {
	ctx := context.Background()
	nodes := []string{"localhost:9001", "localhost:9002"}
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: nodes,
	})
	defer mgr.Close()

	t.Run("returns all active nodes", func(t *testing.T) {
		activeNodes := mgr.GetNodes()
		if len(activeNodes) != 2 {
			t.Errorf("Expected 2 nodes, got %d", len(activeNodes))
		}
	})
}

func TestIsNodeActive(t *testing.T) {
	ctx := context.Background()
	nodes := []string{"localhost:9001"}
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: nodes,
	})
	defer mgr.Close()

	t.Run("returns true for active node", func(t *testing.T) {
		if !mgr.IsNodeActive("localhost:9001") {
			t.Error("Expected node to be active")
		}
	})

	t.Run("returns false for non-existent node", func(t *testing.T) {
		if mgr.IsNodeActive("localhost:9999") {
			t.Error("Expected node to be inactive")
		}
	})
}

func TestUpdateNodes(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("updates node list and increments epoch", func(t *testing.T) {
		initialEpoch := mgr.GetEpoch()

		newNodes := []string{"localhost:9001", "localhost:9002", "localhost:9003"}
		mgr.UpdateNodes(newNodes)

		if mgr.GetEpoch() <= initialEpoch {
			t.Errorf("Expected epoch to increment, got %d", mgr.GetEpoch())
		}

		activeNodes := mgr.GetNodes()
		if len(activeNodes) != 3 {
			t.Errorf("Expected 3 nodes, got %d", len(activeNodes))
		}
	})

	t.Run("detects added and removed nodes", func(t *testing.T) {
		// Subscribe to events
		events := mgr.Subscribe()
		defer mgr.Unsubscribe(events)

		// Update nodes: add one, remove one
		mgr.UpdateNodes([]string{"localhost:9001", "localhost:9004"})

		// Wait for event
		select {
		case event := <-events:
			if len(event.AddedNodes) == 0 {
				t.Error("Expected added nodes")
			}
			if len(event.RemovedNodes) == 0 {
				t.Error("Expected removed nodes")
			}
		case <-time.After(1 * time.Second):
			t.Error("Timeout waiting for topology event")
		}
	})
}

func TestMarkNodeInactive(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001", "localhost:9002"},
	})
	defer mgr.Close()

	t.Run("marks node as inactive", func(t *testing.T) {
		initialEpoch := mgr.GetEpoch()

		mgr.MarkNodeInactive("localhost:9001")

		if mgr.IsNodeActive("localhost:9001") {
			t.Error("Expected node to be inactive")
		}

		if mgr.GetEpoch() <= initialEpoch {
			t.Error("Expected epoch to increment")
		}
	})

	t.Run("does not increment epoch for already inactive node", func(t *testing.T) {
		mgr.MarkNodeInactive("localhost:9001")
		epoch1 := mgr.GetEpoch()

		mgr.MarkNodeInactive("localhost:9001")
		epoch2 := mgr.GetEpoch()

		if epoch2 != epoch1 {
			t.Error("Expected epoch to stay same for already inactive node")
		}
	})
}

func TestMarkNodeActive(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	mgr.MarkNodeInactive("localhost:9001")

	t.Run("marks node as active", func(t *testing.T) {
		epoch1 := mgr.GetEpoch()

		mgr.MarkNodeActive("localhost:9001")

		if !mgr.IsNodeActive("localhost:9001") {
			t.Error("Expected node to be active")
		}

		if mgr.GetEpoch() <= epoch1 {
			t.Error("Expected epoch to increment")
		}
	})
}

func TestSubscribe(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("receives topology events", func(t *testing.T) {
		events := mgr.Subscribe()
		defer mgr.Unsubscribe(events)

		// Trigger an event
		mgr.UpdateNodes([]string{"localhost:9001", "localhost:9002"})

		// Wait for event
		select {
		case event := <-events:
			if event.Epoch == 0 {
				t.Error("Expected non-zero epoch")
			}
			if len(event.AddedNodes) == 0 {
				t.Error("Expected added nodes")
			}
		case <-time.After(1 * time.Second):
			t.Error("Timeout waiting for event")
		}
	})

	t.Run("multiple subscribers receive events", func(t *testing.T) {
		events1 := mgr.Subscribe()
		events2 := mgr.Subscribe()
		defer mgr.Unsubscribe(events1)
		defer mgr.Unsubscribe(events2)

		// Trigger an event
		mgr.UpdateNodes([]string{"localhost:9003"})

		// Both should receive
		received := 0
		timeout := time.After(1 * time.Second)

		for received < 2 {
			select {
			case <-events1:
				received++
			case <-events2:
				received++
			case <-timeout:
				t.Errorf("Expected 2 events, got %d", received)
				return
			}
		}
	})
}

func TestUnsubscribe(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("stops receiving events after unsubscribe", func(t *testing.T) {
		events := mgr.Subscribe()

		// Receive one event
		mgr.UpdateNodes([]string{"localhost:9002"})
		<-events

		// Unsubscribe
		mgr.Unsubscribe(events)

		// Trigger another event
		mgr.UpdateNodes([]string{"localhost:9003"})

		// Should not receive
		select {
		case <-events:
			t.Error("Should not receive event after unsubscribe")
		case <-time.After(200 * time.Millisecond):
			// Expected
		}
	})
}

func TestStart(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("starts without error", func(t *testing.T) {
		err := mgr.Start()
		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
	})
}

func TestClose(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})

	mgr.Start()

	t.Run("closes successfully", func(t *testing.T) {
		err := mgr.Close()
		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
	})
}

func TestWaitForEpoch(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("returns immediately if epoch already reached", func(t *testing.T) {
		err := mgr.WaitForEpoch(ctx, 0)
		if err != nil {
			t.Errorf("Expected no error, got %v", err)
		}
	})

	t.Run("waits for epoch to be reached", func(t *testing.T) {
		done := make(chan error)

		// Start waiting in background
		go func() {
			done <- mgr.WaitForEpoch(ctx, 5)
		}()

		// Give it time to start waiting
		time.Sleep(100 * time.Millisecond)

		// Update nodes several times to increment epoch
		for i := 0; i < 6; i++ {
			mgr.UpdateNodes([]string{"localhost:9001", "localhost:9002"})
			time.Sleep(50 * time.Millisecond)
		}

		// Should complete
		select {
		case err := <-done:
			if err != nil {
				t.Errorf("Expected no error, got %v", err)
			}
		case <-time.After(2 * time.Second):
			t.Error("Timeout waiting for epoch")
		}
	})

	t.Run("returns error on context cancellation", func(t *testing.T) {
		cancelCtx, cancel := context.WithCancel(ctx)
		cancel() // Cancel immediately

		err := mgr.WaitForEpoch(cancelCtx, 999)
		if err == nil {
			t.Error("Expected context error, got nil")
		}
	})
}

func TestConcurrency(t *testing.T) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	t.Run("handles concurrent updates safely", func(t *testing.T) {
		done := make(chan bool, 10)

		for i := 0; i < 10; i++ {
			go func(id int) {
				nodes := []string{"localhost:9001", "localhost:9002"}
				mgr.UpdateNodes(nodes)
				_ = mgr.GetNodes()
				_ = mgr.GetEpoch()

				done <- true
			}(i)
		}

		for i := 0; i < 10; i++ {
			<-done
		}

		// No panics = success
	})

	t.Run("handles concurrent subscriptions safely", func(t *testing.T) {
		done := make(chan bool, 5)

		for i := 0; i < 5; i++ {
			go func() {
				events := mgr.Subscribe()
				time.Sleep(10 * time.Millisecond)
				mgr.Unsubscribe(events)

				done <- true
			}()
		}

		for i := 0; i < 5; i++ {
			<-done
		}

		// No panics = success
	})
}

func BenchmarkGetNodes(b *testing.B) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001", "localhost:9002"},
	})
	defer mgr.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = mgr.GetNodes()
	}
}

func BenchmarkGetEpoch(b *testing.B) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		_ = mgr.GetEpoch()
	}
}

func BenchmarkUpdateNodes(b *testing.B) {
	ctx := context.Background()
	mgr := NewManager(ctx, ManagerOptions{
		InitialNodes: []string{"localhost:9001"},
	})
	defer mgr.Close()

	nodes := []string{"localhost:9001", "localhost:9002"}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		mgr.UpdateNodes(nodes)
	}
}
