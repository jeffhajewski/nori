package com.norikv.client.examples;

import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;

import java.nio.charset.StandardCharsets;

/**
 * Basic example demonstrating simple put, get, and delete operations.
 *
 * <p>This example shows:
 * <ul>
 * <li>Connecting to a NoriKV cluster</li>
 * <li>Storing key-value pairs</li>
 * <li>Retrieving values by key</li>
 * <li>Deleting keys</li>
 * <li>Proper resource cleanup with try-with-resources</li>
 * </ul>
 *
 * <p>Usage:
 * <pre>
 * mvn compile exec:java -Dexec.mainClass="com.norikv.client.examples.BasicExample"
 * </pre>
 */
public class BasicExample {

    public static void main(String[] args) {
        // Configure client to connect to a 3-node cluster
        ClientConfig config = ClientConfig.builder()
                .nodes(java.util.Arrays.asList(
                    "localhost:9001",
                    "localhost:9002",
                    "localhost:9003"
                ))
                .totalShards(1024)
                .timeoutMs(5000)
                .build();

        // Use try-with-resources for automatic cleanup
        try (NoriKVClient client = new NoriKVClient(config)) {
            System.out.println("Connected to NoriKV cluster");
            System.out.println();

            // Example 1: Simple Put
            System.out.println("=== Example 1: Simple Put ===");
            byte[] key = "user:alice".getBytes(StandardCharsets.UTF_8);
            byte[] value = "{\"name\":\"Alice\",\"age\":30}".getBytes(StandardCharsets.UTF_8);

            Version version = client.put(key, value, null);
            System.out.println("Stored key: user:alice");
            System.out.println("Version: term=" + version.getTerm() + ", index=" + version.getIndex());
            System.out.println();

            // Example 2: Simple Get
            System.out.println("=== Example 2: Simple Get ===");
            GetResult result = client.get(key, null);
            String retrievedValue = new String(result.getValue(), StandardCharsets.UTF_8);
            System.out.println("Retrieved value: " + retrievedValue);
            System.out.println("Version: term=" + result.getVersion().getTerm() +
                             ", index=" + result.getVersion().getIndex());
            System.out.println();

            // Example 3: Put with TTL (time-to-live)
            System.out.println("=== Example 3: Put with TTL ===");
            byte[] sessionKey = "session:abc123".getBytes(StandardCharsets.UTF_8);
            byte[] sessionData = "{\"user_id\":42,\"expires\":1234567890}".getBytes(StandardCharsets.UTF_8);

            PutOptions ttlOptions = PutOptions.builder()
                    .ttlMs(60000L) // 60 seconds
                    .build();

            Version sessionVersion = client.put(sessionKey, sessionData, ttlOptions);
            System.out.println("Stored session with 60s TTL");
            System.out.println("Version: " + sessionVersion);
            System.out.println();

            // Example 4: Put with Idempotency Key
            System.out.println("=== Example 4: Put with Idempotency Key ===");
            byte[] orderKey = "order:12345".getBytes(StandardCharsets.UTF_8);
            byte[] orderData = "{\"product\":\"laptop\",\"quantity\":1}".getBytes(StandardCharsets.UTF_8);

            PutOptions idempotentOptions = PutOptions.builder()
                    .idempotencyKey("order-creation-12345")
                    .build();

            Version orderVersion = client.put(orderKey, orderData, idempotentOptions);
            System.out.println("Stored order with idempotency key");
            System.out.println("Version: " + orderVersion);
            System.out.println();

            // Retry with same idempotency key (should be safe)
            System.out.println("Retrying with same idempotency key...");
            Version retryVersion = client.put(orderKey, orderData, idempotentOptions);
            System.out.println("Retry succeeded, version: " + retryVersion);
            System.out.println();

            // Example 5: Get with Different Consistency Levels
            System.out.println("=== Example 5: Get with Consistency Levels ===");

            // Lease read (default, fastest)
            GetOptions leaseRead = GetOptions.builder()
                    .consistency(ConsistencyLevel.LEASE)
                    .build();
            GetResult leaseResult = client.get(key, leaseRead);
            System.out.println("Lease read completed");

            // Linearizable read (strictest)
            GetOptions linearizableRead = GetOptions.builder()
                    .consistency(ConsistencyLevel.LINEARIZABLE)
                    .build();
            GetResult linearResult = client.get(key, linearizableRead);
            System.out.println("Linearizable read completed");

            // Stale read (fastest, may be stale)
            GetOptions staleRead = GetOptions.builder()
                    .consistency(ConsistencyLevel.STALE_OK)
                    .build();
            GetResult staleResult = client.get(key, staleRead);
            System.out.println("Stale read completed");
            System.out.println();

            // Example 6: Delete
            System.out.println("=== Example 6: Delete ===");
            boolean deleted = client.delete(key, null);
            System.out.println("Deleted key: user:alice, success: " + deleted);
            System.out.println();

            // Try to get deleted key
            System.out.println("Attempting to get deleted key...");
            try {
                client.get(key, null);
                System.out.println("ERROR: Should have thrown KeyNotFoundException");
            } catch (KeyNotFoundException e) {
                System.out.println("Correctly got KeyNotFoundException: " + e.getMessage());
            }
            System.out.println();

            // Example 7: Delete with Idempotency
            System.out.println("=== Example 7: Delete with Idempotency ===");
            DeleteOptions deleteOptions = DeleteOptions.builder()
                    .idempotencyKey("delete-order-12345")
                    .build();

            boolean orderDeleted = client.delete(orderKey, deleteOptions);
            System.out.println("Deleted order: " + orderDeleted);
            System.out.println();

            // Example 8: Client Statistics
            System.out.println("=== Example 8: Client Statistics ===");
            NoriKVClient.ClientStats stats = client.getStats();
            System.out.println("Router stats: " + stats.getRouterStats());
            System.out.println("Pool stats: " + stats.getPoolStats());
            System.out.println("Topology stats: " + stats.getTopologyStats());
            System.out.println();

            System.out.println("All examples completed successfully!");

        } catch (NoriKVException e) {
            System.err.println("NoriKV error: " + e.getCode() + " - " + e.getMessage());
            e.printStackTrace();
        } catch (Exception e) {
            System.err.println("Unexpected error: " + e.getMessage());
            e.printStackTrace();
        }
    }
}
