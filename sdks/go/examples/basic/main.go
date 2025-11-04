// Package main demonstrates basic usage of the NoriKV Go client.
package main

import (
	"context"
	"fmt"
	"log"
	"time"

	norikv "github.com/norikv/norikv-go"
)

func main() {
	ctx := context.Background()

	// Configure client with cluster nodes
	config := norikv.DefaultClientConfig([]string{
		"localhost:9001",
		"localhost:9002",
		"localhost:9003",
	})

	// Optional: customize configuration
	config.TimeoutMs = 5000       // 5 second timeout
	config.TotalShards = 1024     // Match server configuration
	config.WatchCluster = true    // Enable cluster topology watching

	// Create client
	client, err := norikv.NewClient(ctx, config)
	if err != nil {
		log.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	fmt.Println("✓ Connected to NoriKV cluster")

	// Example 1: Simple Put and Get
	fmt.Println("\n--- Example 1: Basic Put/Get ---")
	if err := basicPutGet(ctx, client); err != nil {
		log.Printf("Error in basic put/get: %v", err)
	}

	// Example 2: Conditional updates (CAS)
	fmt.Println("\n--- Example 2: Conditional Update (CAS) ---")
	if err := conditionalUpdate(ctx, client); err != nil {
		log.Printf("Error in conditional update: %v", err)
	}

	// Example 3: TTL expiration
	fmt.Println("\n--- Example 3: TTL Expiration ---")
	if err := ttlExample(ctx, client); err != nil {
		log.Printf("Error in TTL example: %v", err)
	}

	// Example 4: Consistency levels
	fmt.Println("\n--- Example 4: Consistency Levels ---")
	if err := consistencyLevels(ctx, client); err != nil {
		log.Printf("Error in consistency levels: %v", err)
	}

	// Example 5: Idempotency
	fmt.Println("\n--- Example 5: Idempotent Writes ---")
	if err := idempotencyExample(ctx, client); err != nil {
		log.Printf("Error in idempotency example: %v", err)
	}

	fmt.Println("\n✓ All examples completed successfully")
}

// basicPutGet demonstrates simple put and get operations.
func basicPutGet(ctx context.Context, client *norikv.Client) error {
	key := []byte("user:alice")
	value := []byte(`{"name":"Alice","email":"alice@example.com"}`)

	// Put a value
	version, err := client.Put(ctx, key, value, nil)
	if err != nil {
		return fmt.Errorf("put failed: %w", err)
	}
	fmt.Printf("✓ Wrote key with version: term=%d index=%d\n", version.Term, version.Index)

	// Get the value back
	result, err := client.Get(ctx, key, nil)
	if err != nil {
		return fmt.Errorf("get failed: %w", err)
	}
	fmt.Printf("✓ Read value: %s\n", string(result.Value))
	fmt.Printf("  Version: term=%d index=%d\n", result.Version.Term, result.Version.Index)

	return nil
}

// conditionalUpdate demonstrates compare-and-swap (CAS) operations.
func conditionalUpdate(ctx context.Context, client *norikv.Client) error {
	key := []byte("counter")
	initialValue := []byte("0")

	// Initial write
	v1, err := client.Put(ctx, key, initialValue, nil)
	if err != nil {
		return fmt.Errorf("initial put failed: %w", err)
	}
	fmt.Printf("✓ Initial write: version=%d:%d\n", v1.Term, v1.Index)

	// Conditional update with correct version (should succeed)
	v2, err := client.Put(ctx, key, []byte("1"), &norikv.PutOptions{
		IfMatchVersion: v1,
	})
	if err != nil {
		return fmt.Errorf("conditional update failed: %w", err)
	}
	fmt.Printf("✓ Conditional update succeeded: version=%d:%d\n", v2.Term, v2.Index)

	// Conditional update with old version (should fail)
	_, err = client.Put(ctx, key, []byte("2"), &norikv.PutOptions{
		IfMatchVersion: v1, // Using old version
	})
	if err != nil {
		fmt.Printf("✓ Conditional update with old version correctly failed: %v\n", err)
	} else {
		return fmt.Errorf("expected conditional update to fail, but it succeeded")
	}

	return nil
}

// ttlExample demonstrates time-to-live expiration.
func ttlExample(ctx context.Context, client *norikv.Client) error {
	key := []byte("session:temp")
	value := []byte("temporary-session-data")
	ttlMs := uint64(2000) // 2 seconds

	// Put with TTL
	_, err := client.Put(ctx, key, value, &norikv.PutOptions{
		TTLMs: &ttlMs,
	})
	if err != nil {
		return fmt.Errorf("put with TTL failed: %w", err)
	}
	fmt.Printf("✓ Wrote key with 2s TTL\n")

	// Verify it exists immediately
	_, err = client.Get(ctx, key, nil)
	if err != nil {
		return fmt.Errorf("expected key to exist: %w", err)
	}
	fmt.Printf("✓ Key exists immediately after write\n")

	// Wait for expiration
	fmt.Printf("  Waiting for expiration...")
	time.Sleep(2500 * time.Millisecond)
	fmt.Printf(" done\n")

	// Verify it's gone
	_, err = client.Get(ctx, key, nil)
	if err != nil {
		fmt.Printf("✓ Key correctly expired: %v\n", err)
	} else {
		return fmt.Errorf("expected key to be expired")
	}

	return nil
}

// consistencyLevels demonstrates different read consistency levels.
func consistencyLevels(ctx context.Context, client *norikv.Client) error {
	key := []byte("config:setting")
	value := []byte("production")

	// Write a value
	_, err := client.Put(ctx, key, value, nil)
	if err != nil {
		return fmt.Errorf("put failed: %w", err)
	}

	// Lease-based read (default, fastest)
	result, err := client.Get(ctx, key, &norikv.GetOptions{
		Consistency: norikv.ConsistencyLease,
	})
	if err != nil {
		return fmt.Errorf("lease read failed: %w", err)
	}
	fmt.Printf("✓ Lease-based read: %s\n", string(result.Value))

	// Linearizable read (strongest consistency)
	result, err = client.Get(ctx, key, &norikv.GetOptions{
		Consistency: norikv.ConsistencyLinearizable,
	})
	if err != nil {
		return fmt.Errorf("linearizable read failed: %w", err)
	}
	fmt.Printf("✓ Linearizable read: %s\n", string(result.Value))

	// Stale-OK read (fastest, may be stale)
	result, err = client.Get(ctx, key, &norikv.GetOptions{
		Consistency: norikv.ConsistencyStaleOK,
	})
	if err != nil {
		return fmt.Errorf("stale read failed: %w", err)
	}
	fmt.Printf("✓ Stale-OK read: %s\n", string(result.Value))

	return nil
}

// idempotencyExample demonstrates idempotent writes.
func idempotencyExample(ctx context.Context, client *norikv.Client) error {
	key := []byte("transaction:12345")
	value := []byte("processed")
	idempotencyKey := "request-id-abc-123"

	// First write with idempotency key
	v1, err := client.Put(ctx, key, value, &norikv.PutOptions{
		IdempotencyKey: idempotencyKey,
	})
	if err != nil {
		return fmt.Errorf("first put failed: %w", err)
	}
	fmt.Printf("✓ First write: version=%d:%d\n", v1.Term, v1.Index)

	// Note: The ephemeral test server doesn't implement full idempotency
	// deduplication. In a production NoriKV cluster, retrying with the
	// same idempotency key would return the same version without
	// performing another write.
	fmt.Printf("  (Idempotency key: %s)\n", idempotencyKey)

	return nil
}
