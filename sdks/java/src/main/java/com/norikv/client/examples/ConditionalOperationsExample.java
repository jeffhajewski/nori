package com.norikv.client.examples;

import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;

import java.nio.charset.StandardCharsets;

/**
 * Example demonstrating conditional operations (Compare-And-Swap).
 *
 * <p>This example shows:
 * <ul>
 * <li>Optimistic concurrency control with version matching</li>
 * <li>Conditional PUT operations (Compare-And-Swap pattern)</li>
 * <li>Conditional DELETE operations</li>
 * <li>Handling version mismatch exceptions</li>
 * <li>Practical use cases: counters, inventory management</li>
 * </ul>
 *
 * <p>Usage:
 * <pre>
 * mvn compile exec:java -Dexec.mainClass="com.norikv.client.examples.ConditionalOperationsExample"
 * </pre>
 */
public class ConditionalOperationsExample {

    public static void main(String[] args) {
        ClientConfig config = ClientConfig.builder()
                .nodes(java.util.Arrays.asList(
                    "localhost:9001",
                    "localhost:9002",
                    "localhost:9003"
                ))
                .totalShards(1024)
                .timeoutMs(5000)
                .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            System.out.println("Connected to NoriKV cluster");
            System.out.println();

            // Example 1: Basic Compare-And-Swap
            System.out.println("=== Example 1: Basic Compare-And-Swap ===");
            byte[] userKey = "user:bob".getBytes(StandardCharsets.UTF_8);
            byte[] initialValue = "{\"credits\":100}".getBytes(StandardCharsets.UTF_8);

            // Initial write
            Version v1 = client.put(userKey, initialValue, null);
            System.out.println("Initial write: credits=100, version=" + v1);

            // Read current value
            GetResult result = client.get(userKey, null);
            System.out.println("Read: " + new String(result.getValue(), StandardCharsets.UTF_8));
            System.out.println("Current version: " + result.getVersion());

            // Conditional update: only succeed if version matches
            byte[] updatedValue = "{\"credits\":90}".getBytes(StandardCharsets.UTF_8);
            PutOptions casOptions = PutOptions.builder()
                    .ifMatchVersion(result.getVersion())
                    .build();

            Version v2 = client.put(userKey, updatedValue, casOptions);
            System.out.println("CAS update succeeded: credits=90, version=" + v2);
            System.out.println();

            // Example 2: Handling Version Mismatch
            System.out.println("=== Example 2: Handling Version Mismatch ===");

            // Try to update with stale version (v1 is now stale, current is v2)
            byte[] staleUpdate = "{\"credits\":80}".getBytes(StandardCharsets.UTF_8);
            PutOptions staleOptions = PutOptions.builder()
                    .ifMatchVersion(v1) // This is stale!
                    .build();

            try {
                client.put(userKey, staleUpdate, staleOptions);
                System.out.println("ERROR: Should have thrown VersionMismatchException");
            } catch (VersionMismatchException e) {
                System.out.println("Correctly rejected stale version");
                System.out.println("Expected: " + v1);
                System.out.println("Actual: " + v2);
                System.out.println("Message: " + e.getMessage());
            }
            System.out.println();

            // Example 3: Retry Loop with CAS
            System.out.println("=== Example 3: Retry Loop with CAS ===");
            byte[] counterKey = "counter:page_views".getBytes(StandardCharsets.UTF_8);

            // Initialize counter
            client.put(counterKey, "0".getBytes(StandardCharsets.UTF_8), null);
            System.out.println("Initialized counter to 0");

            // Increment counter with retry loop
            int maxRetries = 5;
            for (int attempt = 1; attempt <= maxRetries; attempt++) {
                try {
                    // Read current value and version
                    GetResult current = client.get(counterKey, null);
                    int currentCount = Integer.parseInt(new String(current.getValue(), StandardCharsets.UTF_8));

                    // Increment
                    int newCount = currentCount + 1;
                    byte[] newValue = String.valueOf(newCount).getBytes(StandardCharsets.UTF_8);

                    // Conditional update
                    PutOptions incrementOptions = PutOptions.builder()
                            .ifMatchVersion(current.getVersion())
                            .build();

                    Version newVersion = client.put(counterKey, newValue, incrementOptions);
                    System.out.println("Increment succeeded on attempt " + attempt +
                                     ": " + currentCount + " -> " + newCount +
                                     ", version=" + newVersion);
                    break;

                } catch (VersionMismatchException e) {
                    System.out.println("Conflict on attempt " + attempt + ", retrying...");
                    if (attempt == maxRetries) {
                        System.out.println("Max retries exceeded");
                        throw e;
                    }
                    // Small backoff
                    Thread.sleep(10);
                }
            }
            System.out.println();

            // Example 4: Inventory Management with CAS
            System.out.println("=== Example 4: Inventory Management with CAS ===");
            byte[] inventoryKey = "inventory:laptop_model_x".getBytes(StandardCharsets.UTF_8);
            byte[] initialStock = "{\"quantity\":10,\"reserved\":0}".getBytes(StandardCharsets.UTF_8);

            // Initialize inventory
            Version invV1 = client.put(inventoryKey, initialStock, null);
            System.out.println("Initial inventory: quantity=10, reserved=0");

            // Reserve 3 items (conditional update)
            GetResult invResult = client.get(inventoryKey, null);
            String invData = new String(invResult.getValue(), StandardCharsets.UTF_8);
            System.out.println("Current inventory: " + invData);

            // Parse and update (in production, use proper JSON library)
            byte[] reservedStock = "{\"quantity\":7,\"reserved\":3}".getBytes(StandardCharsets.UTF_8);
            PutOptions reserveOptions = PutOptions.builder()
                    .ifMatchVersion(invResult.getVersion())
                    .build();

            Version invV2 = client.put(inventoryKey, reservedStock, reserveOptions);
            System.out.println("Reserved 3 items: quantity=7, reserved=3");
            System.out.println("Version: " + invV2);
            System.out.println();

            // Example 5: Conditional Delete
            System.out.println("=== Example 5: Conditional Delete ===");
            byte[] tempKey = "temp:session_abc".getBytes(StandardCharsets.UTF_8);
            byte[] tempValue = "{\"active\":false}".getBytes(StandardCharsets.UTF_8);

            // Create temporary data
            Version tempV1 = client.put(tempKey, tempValue, null);
            System.out.println("Created temp session, version=" + tempV1);

            // Conditional delete - only delete if version matches
            DeleteOptions deleteOptions = DeleteOptions.builder()
                    .ifMatchVersion(tempV1)
                    .build();

            boolean deleted = client.delete(tempKey, deleteOptions);
            System.out.println("Conditional delete succeeded: " + deleted);
            System.out.println();

            // Example 6: Preventing Lost Updates
            System.out.println("=== Example 6: Preventing Lost Updates ===");
            byte[] configKey = "config:rate_limit".getBytes(StandardCharsets.UTF_8);

            // Two "clients" trying to update concurrently
            // Client A reads config
            byte[] configV1Value = "{\"max_requests\":1000}".getBytes(StandardCharsets.UTF_8);
            Version configV1 = client.put(configKey, configV1Value, null);
            GetResult clientA = client.get(configKey, null);
            System.out.println("Client A reads: max_requests=1000, version=" + clientA.getVersion());

            // Client B reads config
            GetResult clientB = client.get(configKey, null);
            System.out.println("Client B reads: max_requests=1000, version=" + clientB.getVersion());

            // Client A updates first
            byte[] clientAUpdate = "{\"max_requests\":1500}".getBytes(StandardCharsets.UTF_8);
            PutOptions clientAOptions = PutOptions.builder()
                    .ifMatchVersion(clientA.getVersion())
                    .build();
            Version configV2 = client.put(configKey, clientAUpdate, clientAOptions);
            System.out.println("Client A updates to 1500, version=" + configV2);

            // Client B tries to update with stale version
            byte[] clientBUpdate = "{\"max_requests\":2000}".getBytes(StandardCharsets.UTF_8);
            PutOptions clientBOptions = PutOptions.builder()
                    .ifMatchVersion(clientB.getVersion()) // Stale!
                    .build();

            try {
                client.put(configKey, clientBUpdate, clientBOptions);
                System.out.println("ERROR: Should have prevented lost update");
            } catch (VersionMismatchException e) {
                System.out.println("Correctly prevented lost update from Client B");
                System.out.println("Client B must re-read and retry");

                // Client B re-reads and retries
                GetResult clientBRetry = client.get(configKey, null);
                System.out.println("Client B re-reads: max_requests=1500");

                PutOptions clientBRetryOptions = PutOptions.builder()
                        .ifMatchVersion(clientBRetry.getVersion())
                        .build();
                Version configV3 = client.put(configKey, clientBUpdate, clientBRetryOptions);
                System.out.println("Client B successfully updates to 2000, version=" + configV3);
            }
            System.out.println();

            // Example 7: Helper for Safe Increment
            System.out.println("=== Example 7: Safe Increment Helper ===");
            byte[] scoreKey = "score:player_123".getBytes(StandardCharsets.UTF_8);
            client.put(scoreKey, "100".getBytes(StandardCharsets.UTF_8), null);
            System.out.println("Initial score: 100");

            // Increment by 50
            int increment = 50;
            safeIncrement(client, scoreKey, increment);

            GetResult finalScore = client.get(scoreKey, null);
            System.out.println("Final score: " + new String(finalScore.getValue(), StandardCharsets.UTF_8));
            System.out.println();

            System.out.println("All conditional operations completed successfully!");

        } catch (NoriKVException e) {
            System.err.println("NoriKV error: " + e.getCode() + " - " + e.getMessage());
            e.printStackTrace();
        } catch (Exception e) {
            System.err.println("Unexpected error: " + e.getMessage());
            e.printStackTrace();
        }
    }

    /**
     * Helper method to safely increment a counter with CAS retry loop.
     *
     * @param client the NoriKV client
     * @param key the counter key
     * @param delta the amount to increment
     * @throws NoriKVException if operation fails
     * @throws InterruptedException if interrupted during retry
     */
    private static void safeIncrement(NoriKVClient client, byte[] key, int delta)
            throws NoriKVException, InterruptedException {
        int maxRetries = 10;
        for (int attempt = 1; attempt <= maxRetries; attempt++) {
            try {
                // Read current value
                GetResult current = client.get(key, null);
                int currentValue = Integer.parseInt(new String(current.getValue(), StandardCharsets.UTF_8));

                // Compute new value
                int newValue = currentValue + delta;
                byte[] newValueBytes = String.valueOf(newValue).getBytes(StandardCharsets.UTF_8);

                // Conditional update
                PutOptions options = PutOptions.builder()
                        .ifMatchVersion(current.getVersion())
                        .build();

                client.put(key, newValueBytes, options);
                System.out.println("Incremented: " + currentValue + " + " + delta + " = " + newValue);
                return;

            } catch (VersionMismatchException e) {
                if (attempt == maxRetries) {
                    throw e;
                }
                // Exponential backoff
                Thread.sleep((long) Math.pow(2, attempt));
            }
        }
    }
}
