# NoriKV Java Client Troubleshooting Guide

Solutions to common issues and debugging tips.

## Table of Contents

- [Connection Issues](#connection-issues)
- [Performance Problems](#performance-problems)
- [Error Messages](#error-messages)
- [Configuration Issues](#configuration-issues)
- [Debugging Tips](#debugging-tips)
- [Common Pitfalls](#common-pitfalls)

## Connection Issues

### Problem: `ConnectionException: UNAVAILABLE`

**Symptoms**:
```
ConnectionException: UNAVAILABLE: io exception
```

**Possible Causes**:
1. Server is not running
2. Incorrect node addresses
3. Network connectivity issues
4. Firewall blocking connections

**Solutions**:

```bash
# 1. Verify server is running
curl http://localhost:9001/health

# 2. Check network connectivity
telnet localhost 9001

# 3. Verify addresses in config
ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList("localhost:9001"))  // Check port!
    .build();
```

###Problem: `ConnectionException: DEADLINE_EXCEEDED`

**Symptoms**:
```
ConnectionException: DEADLINE_EXCEEDED: deadline exceeded after 5s
```

**Cause**: Request timeout exceeded

**Solutions**:

```java
// 1. Increase timeout
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(1024)
    .timeoutMs(10000) // Increase from 5s to 10s
    .build();

// 2. Check server performance
// - Is server overloaded?
// - Are operations taking too long?
// - Network latency issues?

// 3. Optimize operations
// - Reduce value sizes
// - Use batch operations
// - Check consistency level (STALE_OK is fastest)
```

### Problem: Cannot Connect to Cluster

**Symptoms**:
```
NoriKVException: RETRY_EXHAUSTED: Failed after 10 attempts
```

**Debug Steps**:

```java
// 1. Enable verbose logging
System.setProperty("java.util.logging.SimpleFormatter.format",
    "[%1$tF %1$tT] [%4$-7s] %5$s %n");

// 2. Test connectivity manually
try (Socket socket = new Socket("localhost", 9001)) {
    System.out.println("Connection successful");
} catch (IOException e) {
    System.err.println("Cannot connect: " + e.getMessage());
}

// 3. Verify gRPC works
ManagedChannel channel = ManagedChannelBuilder
    .forAddress("localhost", 9001)
    .usePlaintext()
    .build();

try {
    // Try health check if available
    channel.getState(true);
    System.out.println("Channel state: " + channel.getState(false));
} finally {
    channel.shutdown();
}
```

## Performance Problems

### Problem: Slow Operations

**Symptoms**:
- Operations taking > 100ms
- High p95/p99 latencies
- Timeouts under load

**Diagnosis**:

```java
// Measure operation latency
long start = System.nanoTime();
try {
    client.get(key, null);
} finally {
    long latency = System.nanoTime() - start;
    System.out.println("Latency: " + (latency / 1_000_000.0) + " ms");
}

// Check client stats
ClientStats stats = client.getStats();
System.out.println("Active channels: " + stats.getPoolStats().getActiveChannels());
System.out.println("Cached leaders: " + stats.getTopologyStats().getCachedLeaders());
```

**Solutions**:

1. **Use appropriate consistency level**:
```java
// STALE_OK is fastest (may return stale data)
GetOptions options = GetOptions.builder()
    .consistency(ConsistencyLevel.STALE_OK)
    .build();
```

2. **Check value sizes**:
```java
// Large values slow down operations
GetResult result = client.get(key, null);
System.out.println("Value size: " + result.getValue().length + " bytes");

// Consider compressing large values
byte[] compressed = compress(largeValue);
client.put(key, compressed, null);
```

3. **Verify network latency**:
```bash
# Measure network round-trip time
ping -c 10 server-host
```

4. **Check server load**:
- Monitor server CPU/memory
- Check server logs for errors
- Verify server is not overloaded

### Problem: High Memory Usage

**Symptoms**:
- OutOfMemoryError
- Frequent garbage collection
- Growing heap size

**Causes & Solutions**:

1. **Large values**:
```java
//  Bad: Storing large values
byte[] huge = new byte[100 * 1024 * 1024]; // 100 MB
client.put(key, huge, null);

//  Good: Keep values small
byte[] reasonable = new byte[10 * 1024]; // 10 KB
client.put(key, reasonable, null);
```

2. **Not closing clients**:
```java
//  Bad: Leaking clients
public void handleRequest() {
    NoriKVClient client = new NoriKVClient(config);
    client.get(key, null);
    // Never closed!
}

//  Good: Use try-with-resources
public void handleRequest() {
    try (NoriKVClient client = new NoriKVClient(config)) {
        client.get(key, null);
    } // Automatically closed
}
```

3. **Creating too many clients**:
```java
//  Bad: Client per request
for (int i = 0; i < 1000; i++) {
    try (NoriKVClient client = new NoriKVClient(config)) {
        client.get(key, null);
    }
}

//  Good: Reuse single client
NoriKVClient client = new NoriKVClient(config);
for (int i = 0; i < 1000; i++) {
    client.get(key, null);
}
client.close();
```

### Problem: Version Conflicts / CAS Failures

**Symptoms**:
```
VersionMismatchException: Version mismatch
```

**Cause**: High contention on hot keys

**Solutions**:

1. **Implement retry with backoff**:
```java
int maxRetries = 20;
for (int attempt = 0; attempt < maxRetries; attempt++) {
    try {
        GetResult current = client.get(key, null);
        int value = Integer.parseInt(new String(current.getValue()));

        PutOptions options = PutOptions.builder()
            .ifMatchVersion(current.getVersion())
            .build();

        client.put(key, String.valueOf(value + 1).getBytes(), options);
        break; // Success
    } catch (VersionMismatchException e) {
        if (attempt == maxRetries - 1) throw e;

        // Exponential backoff with jitter
        long backoff = Math.min(1L << attempt, 1000);
        Thread.sleep(backoff + random.nextInt(100));
    }
}
```

2. **Reduce contention**:
```java
//  Bad: Single hot key
client.put("global_counter".getBytes(), value, null);

//  Good: Shard across multiple keys
int shardId = threadId % 10;
client.put(("counter_shard_" + shardId).getBytes(), value, null);
```

3. **Use idempotency for writes that don't need CAS**:
```java
// If you don't need version checking, use idempotency instead
PutOptions options = PutOptions.builder()
    .idempotencyKey("order-" + orderId)
    .build();

client.put(key, value, options); // Safe to retry
```

## Error Messages

### `KeyNotFoundException: Key not found`

**Meaning**: Key does not exist in the store

**Solutions**:
```java
// 1. Handle gracefully
try {
    GetResult result = client.get(key, null);
} catch (KeyNotFoundException e) {
    // Use default value or create key
    client.put(key, defaultValue, null);
}

// 2. Check if key was deleted
// Keys with TTL expire automatically

// 3. Verify key encoding
byte[] key = "user:123".getBytes(StandardCharsets.UTF_8);
//  Not: "user:123".getBytes() // Uses platform default!
```

### `VersionMismatchException: Version mismatch`

**Meaning**: CAS operation failed - version changed

**Normal behavior**: Expected under concurrent access

**Handling**:
```java
// This is not an error - it's how CAS works!
// Just retry the operation

for (int retry = 0; retry < 10; retry++) {
    try {
        GetResult current = client.get(key, null);
        // ... compute new value ...

        PutOptions options = PutOptions.builder()
            .ifMatchVersion(current.getVersion())
            .build();

        client.put(key, newValue, options);
        break; // Success
    } catch (VersionMismatchException e) {
        // Expected - someone else modified the key
        // Loop will retry
    }
}
```

### `IllegalArgumentException: key cannot be null`

**Meaning**: Validation failed - key/value is null or empty

**Fix**:
```java
//  Bad
byte[] key = null;
client.put(key, value, null); // IllegalArgumentException

//  Good
byte[] key = "user:123".getBytes(StandardCharsets.UTF_8);
if (key.length > 0) {
    client.put(key, value, null);
}
```

### `NoriKVException: RETRY_EXHAUSTED`

**Meaning**: All retry attempts failed

**Causes**:
- Server is down
- Network issues
- Persistent errors

**Diagnosis**:
```java
try {
    client.put(key, value, null);
} catch (NoriKVException e) {
    System.err.println("Error code: " + e.getCode());
    System.err.println("Message: " + e.getMessage());

    if (e.getCause() != null) {
        System.err.println("Underlying cause:");
        e.getCause().printStackTrace();
    }
}
```

## Configuration Issues

### Problem: Wrong Total Shards

**Symptoms**:
- Keys not found even though they exist
- Inconsistent behavior

**Cause**: `totalShards` doesn't match cluster configuration

**Fix**:
```java
//  Wrong
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(512) // Cluster has 1024!
    .build();

//  Correct
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(1024) // Match cluster config
    .build();

// Query server for actual shard count if unsure
```

### Problem: Missing Nodes

**Symptoms**:
- Some operations fail
- Uneven load distribution

**Fix**:
```java
//  Bad: Missing nodes
ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList("node1:9001")) // Only 1 of 3 nodes!
    .build();

//  Good: All nodes
ClientConfig config = ClientConfig.builder()
    .nodes(Arrays.asList(
        "node1:9001",
        "node2:9001",
        "node3:9001"
    ))
    .build();
```

### Problem: Retry Configuration Too Aggressive

**Symptoms**:
- Operations take very long to fail
- High latency on errors

**Fix**:
```java
//  Too many retries
RetryConfig config = RetryConfig.builder()
    .maxAttempts(100) // Way too many!
    .maxDelayMs(60000) // 60 second backoff!
    .build();

//  Reasonable
RetryConfig config = RetryConfig.builder()
    .maxAttempts(5)
    .initialDelayMs(100)
    .maxDelayMs(2000)
    .build();
```

## Debugging Tips

### Enable Detailed Logging

```java
// Enable gRPC logging
import java.util.logging.*;

Logger grpcLogger = Logger.getLogger("io.grpc");
grpcLogger.setLevel(Level.FINE);

ConsoleHandler handler = new ConsoleHandler();
handler.setLevel(Level.FINE);
grpcLogger.addHandler(handler);
```

### Capture Network Traffic

```bash
# Use tcpdump to capture gRPC traffic
sudo tcpdump -i any -w norikv.pcap port 9001

# Analyze with Wireshark
wireshark norikv.pcap
```

### Inspect Protobuf Messages

```java
// Log protobuf messages
Norikv.PutRequest request = ProtoConverters.buildPutRequest(key, value, options);
System.out.println("Request: " + request);
```

### Monitor Client Stats

```java
// Periodic stats monitoring
ScheduledExecutorService scheduler = Executors.newScheduledThreadPool(1);
scheduler.scheduleAtFixedRate(() -> {
    ClientStats stats = client.getStats();
    System.out.println("=== Client Stats ===");
    System.out.println("Closed: " + stats.isClosed());
    System.out.println("Channels: " + stats.getPoolStats().getActiveChannels());
    System.out.println("Epoch: " + stats.getTopologyStats().getCurrentEpoch());
}, 0, 10, TimeUnit.SECONDS);
```

### Test with Ephemeral Server

```java
// Use ephemeral server for testing
import com.norikv.client.testing.EphemeralServer;

EphemeralServer server = EphemeralServer.start(9001);
try {
    ClientConfig config = ClientConfig.builder()
        .nodes(Arrays.asList(server.getAddress()))
        .totalShards(1024)
        .build();

    try (NoriKVClient client = new NoriKVClient(config)) {
        // Test operations
        client.put("test".getBytes(), "value".getBytes(), null);
    }
} finally {
    server.stop();
}
```

### Measure Operation Latency

```java
public class LatencyMonitor {
    public static void measureOperation(String name, Runnable operation) {
        long start = System.nanoTime();
        try {
            operation.run();
        } finally {
            long latency = System.nanoTime() - start;
            double ms = latency / 1_000_000.0;
            System.out.printf("%s: %.3f ms%n", name, ms);
        }
    }
}

// Usage
LatencyMonitor.measureOperation("PUT", () -> {
    try {
        client.put(key, value, null);
    } catch (NoriKVException e) {
        // Handle error
    }
});
```

## Common Pitfalls

### 1. Not Using UTF-8 Encoding

```java
//  Bad: Platform default encoding
byte[] key = "user:123".getBytes();

//  Good: Explicit UTF-8
byte[] key = "user:123".getBytes(StandardCharsets.UTF_8);
```

### 2. Forgetting to Close Client

```java
//  Bad: Resource leak
NoriKVClient client = new NoriKVClient(config);
client.get(key, null);
// Never closed!

//  Good: Always close
try (NoriKVClient client = new NoriKVClient(config)) {
    client.get(key, null);
} // Automatically closed
```

### 3. Creating Client Per Request

```java
//  Bad: Expensive
public void handleRequest() {
    try (NoriKVClient client = new NoriKVClient(config)) {
        client.get(key, null);
    }
}

//  Good: Reuse client
private final NoriKVClient client = new NoriKVClient(config);

public void handleRequest() {
    client.get(key, null);
}
```

### 4. Not Handling Version Conflicts

```java
//  Bad: No retry
try {
    client.put(key, newValue, casOptions);
} catch (VersionMismatchException e) {
    // Just fail? Should retry!
}

//  Good: Retry with backoff
for (int i = 0; i < maxRetries; i++) {
    try {
        GetResult current = client.get(key, null);
        PutOptions opts = PutOptions.builder()
            .ifMatchVersion(current.getVersion())
            .build();
        client.put(key, newValue, opts);
        break;
    } catch (VersionMismatchException e) {
        if (i == maxRetries - 1) throw e;
        Thread.sleep(backoff);
    }
}
```

### 5. Ignoring Client Statistics

```java
//  Monitor health
ClientStats stats = client.getStats();
if (stats.isClosed()) {
    throw new IllegalStateException("Client is closed!");
}

if (stats.getPoolStats().getActiveChannels() == 0) {
    logger.warn("No active connections!");
}
```

### 6. Using Wrong Consistency Level

```java
//  Bad: Always linearizable (slow)
GetOptions options = GetOptions.builder()
    .consistency(ConsistencyLevel.LINEARIZABLE)
    .build();

//  Good: Use appropriate level
// - LEASE (default): Most operations
// - LINEARIZABLE: Critical reads only
// - STALE_OK: Caching, read-heavy
```

### 7. Not Setting Timeouts

```java
//  Bad: Default might be too short/long
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(1024)
    .build(); // Uses default 5s

//  Good: Set appropriate timeout
ClientConfig config = ClientConfig.builder()
    .nodes(nodes)
    .totalShards(1024)
    .timeoutMs(10000) // 10s for slow operations
    .build();
```

### 8. Large Keys

```java
//  Bad: Huge keys
String hugeKey = "x".repeat(1000000); // 1MB key!
client.put(hugeKey.getBytes(), value, null);

//  Good: Reasonable key sizes
String key = "user:123"; // < 1KB
client.put(key.getBytes(StandardCharsets.UTF_8), value, null);
```

## Getting Help

If you're still stuck:

1. **Check Examples**: [src/main/java/com/norikv/client/examples/](../src/main/java/com/norikv/client/examples/)

2. **Read API Guide**: [API_GUIDE.md](API_GUIDE.md)

3. **Check Architecture**: [ARCHITECTURE.md](ARCHITECTURE.md)

4. **Run Tests**: Verify SDK works in your environment
   ```bash
   mvn test
   ```

5. **Enable Debug Logging**: See detailed error messages

6. **File an Issue**: [GitHub Issues](https://github.com/norikv/norikv/issues)
   - Include: Java version, SDK version, error messages, configuration
   - Provide: Minimal reproducible example

## See Also

- [API Guide](API_GUIDE.md) - Complete API reference
- [Architecture Guide](ARCHITECTURE.md) - Internal design
- [Advanced Patterns](ADVANCED_PATTERNS.md) - Complex use cases
