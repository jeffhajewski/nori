// Package main demonstrates using the ephemeral in-memory server for testing.
package main

import (
	"context"
	"fmt"
	"log"

	norikv "github.com/norikv/norikv-go"
	"github.com/norikv/norikv-go/testing/ephemeral"
)

func main() {
	// Start an ephemeral in-memory server
	// This is perfect for testing, development, and CI/CD pipelines
	fmt.Println("Starting ephemeral NoriKV server...")

	server, err := ephemeral.NewServer(ephemeral.ServerOptions{
		TotalShards: 128,  // Number of virtual shards
		Address:     "localhost:0", // Random port
	})
	if err != nil {
		log.Fatalf("Failed to start server: %v", err)
	}
	defer server.Stop()

	fmt.Printf("✓ Server started on %s\n", server.Address())

	// Create client connected to ephemeral server
	ctx := context.Background()
	config := norikv.DefaultClientConfig([]string{server.Address()})
	config.TotalShards = 128
	config.WatchCluster = false // No cluster watching for single-node ephemeral server

	client, err := norikv.NewClient(ctx, config)
	if err != nil {
		log.Fatalf("Failed to create client: %v", err)
	}
	defer client.Close()

	fmt.Println("✓ Client connected to ephemeral server")

	// Run some operations
	if err := runOperations(ctx, client); err != nil {
		log.Fatalf("Operations failed: %v", err)
	}

	// Get server statistics
	stats := server.GetStats()
	fmt.Printf("\n--- Server Statistics ---\n")
	fmt.Printf("Total shards:  %d\n", stats.TotalShards)
	fmt.Printf("Active shards: %d\n", stats.ActiveShards)
	fmt.Printf("Total entries: %d\n", stats.TotalEntries)

	// Demonstrate cleanup of expired entries
	fmt.Println("\n--- Cleanup ---")
	removed := server.Cleanup()
	fmt.Printf("Removed %d expired entries\n", removed)
}

func runOperations(ctx context.Context, client *norikv.Client) error {
	fmt.Println("\n--- Running Operations ---")

	// Write some test data
	testData := map[string]string{
		"user:1":  `{"name":"Alice","role":"admin"}`,
		"user:2":  `{"name":"Bob","role":"user"}`,
		"user:3":  `{"name":"Charlie","role":"user"}`,
		"config:1": `{"setting":"value"}`,
	}

	for key, value := range testData {
		version, err := client.Put(ctx, []byte(key), []byte(value), nil)
		if err != nil {
			return fmt.Errorf("put %s failed: %w", key, err)
		}
		fmt.Printf("✓ Wrote %s (v%d:%d)\n", key, version.Term, version.Index)
	}

	// Read back the data
	fmt.Println("\nReading back data:")
	for key := range testData {
		result, err := client.Get(ctx, []byte(key), nil)
		if err != nil {
			return fmt.Errorf("get %s failed: %w", key, err)
		}
		fmt.Printf("✓ Read %s: %s\n", key, string(result.Value))
	}

	// Test conditional update
	fmt.Println("\nTesting conditional update:")
	result, err := client.Get(ctx, []byte("user:1"), nil)
	if err != nil {
		return fmt.Errorf("get user:1 failed: %w", err)
	}

	newVersion, err := client.Put(ctx, []byte("user:1"), []byte(`{"name":"Alice","role":"superadmin"}`), &norikv.PutOptions{
		IfMatchVersion: result.Version,
	})
	if err != nil {
		return fmt.Errorf("conditional update failed: %w", err)
	}
	fmt.Printf("✓ Updated user:1 conditionally (v%d:%d)\n", newVersion.Term, newVersion.Index)

	// Test delete
	fmt.Println("\nTesting delete:")
	err = client.Delete(ctx, []byte("config:1"), nil)
	if err != nil {
		return fmt.Errorf("delete failed: %w", err)
	}
	fmt.Printf("✓ Deleted config:1\n")

	// Verify deletion
	_, err = client.Get(ctx, []byte("config:1"), nil)
	if err != nil {
		fmt.Printf("✓ Confirmed config:1 is deleted\n")
	} else {
		return fmt.Errorf("expected config:1 to be deleted")
	}

	return nil
}
