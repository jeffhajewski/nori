# NoriKV Go Client Troubleshooting Guide

Solutions to common issues when using the Go client SDK.

## Table of Contents

- [Connection Issues](#connection-issues)
- [Performance Problems](#performance-problems)
- [Version Conflicts](#version-conflicts)
- [Error Messages](#error-messages)
- [Configuration Issues](#configuration-issues)
- [Debugging Tips](#debugging-tips)

## Connection Issues

### Problem: "connection refused" or "dial tcp: connect: connection refused"

**Symptoms:**
```go
_, err := client.Put(ctx, key, value, nil)
// Error: rpc error: code = Unavailable desc = connection error:
// desc = "transport: Error while dialing dial tcp 127.0.0.1:9001:
// connect: connection refused"
```

**Causes:**
1. NoriKV server is not running
2. Wrong address/port in configuration
3. Firewall blocking connections

**Solutions:**

1. Verify server is running:
```bash
# Check if server is listening
netstat -an | grep 9001

# Or use lsof
lsof -i :9001
```

2. Check client configuration:
```go
config := &norikv.ClientConfig{
    Nodes: []string{"localhost:9001"}, // Verify this address
    // ...
}
```

3. Test connectivity:
```bash
telnet localhost 9001
# Or
nc -zv localhost 9001
```

### Problem: "context deadline exceeded"

**Symptoms:**
```go
ctx, cancel := context.WithTimeout(context.Background(), 100*time.Millisecond)
defer cancel()

_, err := client.Get(ctx, key, nil)
// Error: rpc error: code = DeadlineExceeded desc = context deadline exceeded
```

**Causes:**
1. Timeout too short
2. Server overloaded
3. Network latency
4. Leader election in progress

**Solutions:**

1. Increase timeout:
```go
config := &norikv.ClientConfig{
    Timeout: 10 * time.Second, // Increase from default 5s
    // ...
}

// Or use longer context timeout
ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
defer cancel()
```

2. Check server health:
```bash
# Check server logs for errors
# Check CPU/memory usage
# Verify cluster quorum
```

3. Enable retries:
```go
config := &norikv.ClientConfig{
    Retry: &norikv.RetryConfig{
        MaxAttempts:  10,
        InitialDelay: 100 * time.Millisecond,
        MaxDelay:     5 * time.Second,
    },
    // ...
}
```

### Problem: "transport is closing"

**Symptoms:**
```go
_, err := client.Put(ctx, key, value, nil)
// Error: rpc error: code = Unavailable desc = transport is closing
```

**Causes:**
1. Server restarted
2. Network interruption
3. Connection idle timeout

**Solutions:**

1. Enable automatic retries (already default behavior)
2. Configure keepalive:
```go
// Connections automatically have keepalive configured
// If you need custom settings, modify internal/conn/pool.go
```

3. Check network stability:
```bash
ping <server-host>
traceroute <server-host>
```

## Performance Problems

### Problem: Slow Put/Get operations

**Symptoms:**
```go
start := time.Now()
_, err := client.Put(ctx, key, value, nil)
duration := time.Since(start)
// duration > 100ms consistently
```

**Causes:**
1. Network latency
2. Large value sizes
3. Server overload
4. Wrong consistency level
5. Too many retries

**Diagnosis:**

1. Measure network latency:
```bash
ping <server-host>
```

2. Check value sizes:
```go
fmt.Printf("Value size: %d bytes\n", len(value))
// Optimal: 100 bytes - 10 KB
// Too large: > 1 MB
```

3. Monitor retry count:
```go
// Check logs for retry attempts
// Excessive retries indicate server issues
```

**Solutions:**

1. Use appropriate consistency level:
```go
// For reads that don't need strict consistency
options := &norikv.GetOptions{
    Consistency: norikv.ConsistencyStaleOK, // Fastest
}
result, err := client.Get(ctx, key, options)
```

2. Reduce value sizes:
```go
// Compress large values
compressed := compress(largeValue)
client.Put(ctx, key, compressed, nil)
```

3. Use concurrent requests:
```go
var wg sync.WaitGroup
for _, key := range keys {
    wg.Add(1)
    go func(k []byte) {
        defer wg.Done()
        client.Get(ctx, k, nil)
    }(key)
}
wg.Wait()
```

### Problem: High memory usage

**Symptoms:**
```go
// Client consuming excessive memory
// Memory not being released
```

**Causes:**
1. Too many goroutines
2. Large values buffered
3. Connection leaks
4. Topology listeners not cleaned up

**Solutions:**

1. Limit concurrent operations:
```go
semaphore := make(chan struct{}, 100) // Limit to 100 concurrent
for _, key := range keys {
    semaphore <- struct{}{}
    go func(k []byte) {
        defer func() { <-semaphore }()
        client.Get(ctx, k, nil)
    }(key)
}
```

2. Clean up topology listeners:
```go
unsubscribe := client.OnTopologyChange(handler)
defer unsubscribe() // Important!
```

3. Close client when done:
```go
defer client.Close()
```

4. Use pprof to diagnose:
```go
import _ "net/http/pprof"
import "net/http"

go func() {
    log.Println(http.ListenAndServe("localhost:6060", nil))
}()

// Visit http://localhost:6060/debug/pprof/
```

## Version Conflicts

### Problem: "version mismatch" on CAS operations

**Symptoms:**
```go
result, _ := client.Get(ctx, key, nil)

options := &norikv.PutOptions{
    IfMatchVersion: result.Version,
}

_, err := client.Put(ctx, key, newValue, options)
if errors.Is(err, norikv.ErrVersionMismatch) {
    // This happens frequently
}
```

**Causes:**
1. High contention on key
2. Concurrent updates from multiple clients
3. Insufficient retry logic

**Solutions:**

1. Implement retry loop:
```go
const maxRetries = 10
for i := 0; i < maxRetries; i++ {
    result, err := client.Get(ctx, key, nil)
    if err != nil {
        return err
    }

    // Compute new value
    newValue := transform(result.Value)

    options := &norikv.PutOptions{
        IfMatchVersion: result.Version,
    }

    _, err = client.Put(ctx, key, newValue, options)
    if err == nil {
        break // Success
    }

    if !errors.Is(err, norikv.ErrVersionMismatch) {
        return err
    }

    if i == maxRetries-1 {
        return fmt.Errorf("failed after %d retries", maxRetries)
    }

    // Exponential backoff
    time.Sleep(time.Duration(1<<i) * 10 * time.Millisecond)
}
```

2. Reduce contention by sharding:
```go
// Instead of single counter
key := []byte("global-counter")

// Use sharded counters
shardID := rand.Intn(10)
key := []byte(fmt.Sprintf("counter-%d", shardID))
```

3. Use idempotency keys when appropriate:
```go
options := &norikv.PutOptions{
    IdempotencyKey: requestID,
    // CAS still checked, but retry is safe
}
```

### Problem: Lost updates with concurrent writes

**Symptoms:**
```go
// Multiple goroutines updating same key
// Final value doesn't reflect all updates
```

**Cause:**
Not using CAS for concurrent updates.

**Solution:**

Always use CAS for concurrent updates:
```go
func incrementCounter(client *norikv.Client, ctx context.Context, key []byte) error {
    for i := 0; i < 10; i++ {
        result, err := client.Get(ctx, key, nil)
        if err != nil {
            return err
        }

        value, _ := strconv.Atoi(string(result.Value))
        newValue := []byte(strconv.Itoa(value + 1))

        options := &norikv.PutOptions{
            IfMatchVersion: result.Version,
        }

        _, err = client.Put(ctx, key, newValue, options)
        if err == nil {
            return nil
        }

        if !errors.Is(err, norikv.ErrVersionMismatch) {
            return err
        }
    }
    return fmt.Errorf("failed after retries")
}
```

## Error Messages

### "key not found"

**Error:**
```go
_, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrKeyNotFound) {
    // Handle
}
```

**Meaning**: Key does not exist in the database.

**Solutions:**
1. Check if key was written
2. Check for typos in key name
3. Handle gracefully:
```go
result, err := client.Get(ctx, key, nil)
if errors.Is(err, norikv.ErrKeyNotFound) {
    // Use default value
    return defaultValue
}
```

### "version mismatch"

**Error:**
```go
_, err := client.Put(ctx, key, value, &norikv.PutOptions{
    IfMatchVersion: expectedVersion,
})
if errors.Is(err, norikv.ErrVersionMismatch) {
    // Handle
}
```

**Meaning**: CAS check failed - version changed between read and write.

**Solutions**: See [Version Conflicts](#version-conflicts) above.

### "key already exists"

**Error:**
```go
_, err := client.Put(ctx, key, value, &norikv.PutOptions{
    IfNotExists: true,
})
if errors.Is(err, norikv.ErrAlreadyExists) {
    // Handle
}
```

**Meaning**: Key already exists (used with IfNotExists).

**Solutions:**
```go
if errors.Is(err, norikv.ErrAlreadyExists) {
    // Get existing value
    result, _ := client.Get(ctx, key, nil)
    // Or ignore if duplicate is okay
}
```

## Configuration Issues

### Problem: "invalid shard count"

**Symptoms:**
```go
client, err := norikv.NewClient(ctx, config)
// Error: invalid shard count
```

**Cause:**
TotalShards must match cluster configuration.

**Solution:**
```go
config := &norikv.ClientConfig{
    TotalShards: 1024, // Must match server
    // ...
}
```

### Problem: Hash mismatches causing wrong routing

**Symptoms:**
- Requests going to wrong shards
- Frequent NOT_LEADER errors

**Cause:**
Hash function incompatibility.

**Solution:**

Verify hash compatibility:
```bash
go test -v ./hash/...
```

All tests must pass to ensure compatibility with server.

### Problem: Client not finding any nodes

**Symptoms:**
```go
client, err := norikv.NewClient(ctx, config)
// Error: no nodes available
```

**Cause:**
Empty or invalid Nodes configuration.

**Solution:**
```go
config := &norikv.ClientConfig{
    Nodes: []string{
        "node1:9001",
        "node2:9001",
        "node3:9001",
    },
    // ...
}
```

## Debugging Tips

### Enable Debug Logging

```go
// Add logging to track requests
import "log"

type loggingClient struct {
    *norikv.Client
}

func (c *loggingClient) Put(ctx context.Context, key, value []byte, opts *norikv.PutOptions) (*norikv.Version, error) {
    log.Printf("PUT key=%s, size=%d", key, len(value))
    version, err := c.Client.Put(ctx, key, value, opts)
    if err != nil {
        log.Printf("PUT error: %v", err)
    } else {
        log.Printf("PUT success: version=%v", version)
    }
    return version, err
}
```

### Monitor Client Statistics

```go
// Periodically log stats
go func() {
    ticker := time.NewTicker(10 * time.Second)
    defer ticker.Stop()

    for range ticker.C {
        stats := client.Stats()
        log.Printf("Client stats: %+v", stats)
    }
}()
```

### Test with Ephemeral Server

```go
import "github.com/norikv/norikv-go/testing/ephemeral"

// Start in-memory server for testing
server := ephemeral.NewServer()
err := server.Start("127.0.0.1:0")
if err != nil {
    log.Fatal(err)
}
defer server.Stop()

// Get actual address
address := server.Address()

// Create client
config := &norikv.ClientConfig{
    Nodes:       []string{address},
    TotalShards: 1024,
}
client, err := norikv.NewClient(ctx, config)
```

### Use pprof for Performance Analysis

```go
import (
    _ "net/http/pprof"
    "net/http"
    "runtime"
)

// Enable pprof
go func() {
    log.Println(http.ListenAndServe("localhost:6060", nil))
}()

// Analyze CPU profile
// go tool pprof http://localhost:6060/debug/pprof/profile?seconds=30

// Analyze memory
// go tool pprof http://localhost:6060/debug/pprof/heap

// Check goroutines
// go tool pprof http://localhost:6060/debug/pprof/goroutine
```

### Trace Individual Requests

```go
import "context"

// Add trace ID to context
type traceKeyType string
const traceKey traceKeyType = "trace-id"

func withTraceID(ctx context.Context, id string) context.Context {
    return context.WithValue(ctx, traceKey, id)
}

func getTraceID(ctx context.Context) string {
    if id, ok := ctx.Value(traceKey).(string); ok {
        return id
    }
    return ""
}

// Use in requests
traceID := uuid.New().String()
ctx := withTraceID(context.Background(), traceID)
log.Printf("[%s] Starting request", traceID)
result, err := client.Get(ctx, key, nil)
log.Printf("[%s] Request complete: err=%v", traceID, err)
```

## Common Pitfalls

### 1. Not checking errors

```go
// ❌ Bad
result, _ := client.Get(ctx, key, nil)
process(result.Value) // May panic if result is nil

// ✅ Good
result, err := client.Get(ctx, key, nil)
if err != nil {
    log.Printf("Get failed: %v", err)
    return err
}
process(result.Value)
```

### 2. Creating client per request

```go
// ❌ Bad - closes connections!
func handleRequest() {
    client, _ := norikv.NewClient(ctx, config)
    defer client.Close()
    client.Get(ctx, key, nil)
}

// ✅ Good - reuse client
var globalClient *norikv.Client

func init() {
    globalClient, _ = norikv.NewClient(context.Background(), config)
}
```

### 3. Ignoring context cancellation

```go
// ❌ Bad
_, err := client.Get(context.Background(), key, nil)

// ✅ Good
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
defer cancel()
_, err := client.Get(ctx, key, nil)
```

### 4. Not using CAS for concurrent updates

```go
// ❌ Bad - lost updates
result, _ := client.Get(ctx, key, nil)
value, _ := strconv.Atoi(string(result.Value))
client.Put(ctx, key, []byte(strconv.Itoa(value+1)), nil)

// ✅ Good - CAS prevents lost updates
result, _ := client.Get(ctx, key, nil)
value, _ := strconv.Atoi(string(result.Value))
client.Put(ctx, key, []byte(strconv.Itoa(value+1)), &norikv.PutOptions{
    IfMatchVersion: result.Version,
})
```

## Getting Help

If you're still experiencing issues:

1. Check the [API Guide](API_GUIDE.md) for correct usage
2. Review [Architecture Guide](ARCHITECTURE.md) for understanding internals
3. See [Advanced Patterns](ADVANCED_PATTERNS.md) for complex use cases
4. Open an issue on GitHub with:
   - Go version (`go version`)
   - Client SDK version
   - Minimal reproduction code
   - Error messages and logs
   - Server configuration
