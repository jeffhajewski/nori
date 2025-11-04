# Ephemeral Server Example

This example demonstrates using the **ephemeral in-memory server** for testing, development, and CI/CD pipelines.

## What is the Ephemeral Server?

The ephemeral server is a lightweight, in-memory NoriKV server that:
- Starts instantly with no external dependencies
- Uses in-memory storage (no disk I/O)
- Implements the full gRPC protocol
- Perfect for testing and development
- Automatically cleans up on shutdown

## Running

```bash
go run main.go
```

Output:
```
Starting ephemeral NoriKV server...
✓ Server started on 127.0.0.1:xxxxx
✓ Client connected to ephemeral server

--- Running Operations ---
✓ Wrote config:1 (v1:1)
✓ Wrote user:1 (v1:1)
...

--- Server Statistics ---
Total shards:  128
Active shards: 4
Total entries: 3
```

## Use Cases

### 1. Unit Tests
Test your application code without a real cluster:

```go
func TestMyApp(t *testing.T) {
    server, _ := ephemeral.NewServer(ephemeral.ServerOptions{})
    defer server.Stop()

    client, _ := norikv.NewClient(ctx, norikv.DefaultClientConfig([]string{server.Address()}))
    defer client.Close()

    // Test your application logic
}
```

### 2. Integration Tests
Validate NoriKV client behavior:

```go
func TestPutGet(t *testing.T) {
    server, _ := ephemeral.NewServer(ephemeral.ServerOptions{TotalShards: 128})
    defer server.Stop()

    // Run tests against server
}
```

### 3. CI/CD Pipelines
No need to spin up infrastructure in CI:

```yaml
# .github/workflows/test.yml
- name: Run tests
  run: go test ./...  # Ephemeral servers start automatically
```

### 4. Local Development
Quickly test changes without cluster setup.

## Server Options

Configure the ephemeral server:

```go
server, err := ephemeral.NewServer(ephemeral.ServerOptions{
    TotalShards: 128,          // Number of virtual shards (default: 1024)
    Address:     "localhost:0", // Bind address (default: random port)
})
```

## Server Statistics

Get runtime statistics:

```go
stats := server.GetStats()
fmt.Printf("Total shards:  %d\n", stats.TotalShards)
fmt.Printf("Active shards: %d\n", stats.ActiveShards)  // Shards with data
fmt.Printf("Total entries: %d\n", stats.TotalEntries)  // Key-value pairs
```

## Cleanup

Manually trigger expired entry cleanup:

```go
removed := server.Cleanup()
fmt.Printf("Removed %d expired entries\n", removed)
```

This is automatically done in the real server, but can be manually triggered in tests.

## Limitations

The ephemeral server is **not** a full NoriKV cluster:

- Single node only (no replication, no Raft consensus)
- In-memory storage (no persistence)
- Simplified version tracking (single term, monotonic index)
- No idempotency deduplication (accepts duplicate operations)
- No SWIM membership protocol

For production use, deploy a real NoriKV cluster with:
- Multiple nodes for high availability
- Raft replication for durability
- WAL + LSM storage engine
- Full idempotency support

## Real Integration Tests

See `../../integration_test.go` for complete integration tests using the ephemeral server.

## Learn More

- [Go SDK README](../../README.md)
- [Basic Usage Example](../basic/)
- [Integration Tests](../../integration_test.go)
