package com.norikv.client.examples;

import com.norikv.client.NoriKVClient;
import com.norikv.client.types.*;

import java.nio.charset.StandardCharsets;

/**
 * Example demonstrating retry behavior and error handling.
 *
 * <p>This example shows:
 * <ul>
 * <li>Handling different exception types</li>
 * <li>Configuring custom retry policies</li>
 * <li>Using idempotency keys for safe retries</li>
 * <li>Handling transient vs permanent errors</li>
 * <li>Best practices for error recovery</li>
 * </ul>
 *
 * <p>Usage:
 * <pre>
 * mvn compile exec:java -Dexec.mainClass="com.norikv.client.examples.RetryAndErrorHandlingExample"
 * </pre>
 */
public class RetryAndErrorHandlingExample {

    public static void main(String[] args) {
        // Example 1: Default Retry Configuration
        System.out.println("=== Example 1: Default Retry Configuration ===");
        ClientConfig defaultConfig = ClientConfig.builder()
                .nodes(java.util.Arrays.asList("localhost:9001", "localhost:9002"))
                .totalShards(1024)
                .build();

        System.out.println("Default retry config:");
        System.out.println("  Max attempts: " + defaultConfig.getRetry().getMaxAttempts());
        System.out.println("  Initial delay: " + defaultConfig.getRetry().getInitialDelayMs() + "ms");
        System.out.println("  Max delay: " + defaultConfig.getRetry().getMaxDelayMs() + "ms");
        System.out.println("  Jitter: " + defaultConfig.getRetry().getJitterMs() + "ms");
        System.out.println();

        // Example 2: Custom Retry Configuration
        System.out.println("=== Example 2: Custom Retry Configuration ===");
        RetryConfig customRetry = RetryConfig.builder()
                .maxAttempts(5)           // Fewer retries
                .initialDelayMs(50)        // Start with 50ms
                .maxDelayMs(2000)          // Cap at 2 seconds
                .jitterMs(200)             // 200ms jitter
                .build();

        ClientConfig customConfig = ClientConfig.builder()
                .nodes(java.util.Arrays.asList("localhost:9001", "localhost:9002"))
                .totalShards(1024)
                .retry(customRetry)
                .build();

        System.out.println("Custom retry config:");
        System.out.println("  Max attempts: " + customConfig.getRetry().getMaxAttempts());
        System.out.println("  Initial delay: " + customConfig.getRetry().getInitialDelayMs() + "ms");
        System.out.println("  Max delay: " + customConfig.getRetry().getMaxDelayMs() + "ms");
        System.out.println("  Jitter: " + customConfig.getRetry().getJitterMs() + "ms");
        System.out.println();

        // Example 3: Handling KeyNotFoundException
        System.out.println("=== Example 3: Handling KeyNotFoundException ===");
        try (NoriKVClient client = new NoriKVClient(defaultConfig)) {
            byte[] missingKey = "nonexistent:key".getBytes(StandardCharsets.UTF_8);

            try {
                GetResult result = client.get(missingKey, null);
                System.out.println("ERROR: Should have thrown KeyNotFoundException");
            } catch (KeyNotFoundException e) {
                System.out.println("Caught KeyNotFoundException: " + e.getMessage());
                System.out.println("This is expected - key does not exist");
                System.out.println("Error code: " + e.getCode());
            }
        } catch (NoriKVException e) {
            // Connection errors expected when no server is running
            System.out.println("Note: This example requires a running NoriKV cluster");
            System.out.println("Error: " + e.getCode() + " - " + e.getMessage());
        }
        System.out.println();

        // Example 4: Handling VersionMismatchException
        System.out.println("=== Example 4: Handling VersionMismatchException ===");
        try (NoriKVClient client = new NoriKVClient(defaultConfig)) {
            byte[] key = "counter:test".getBytes(StandardCharsets.UTF_8);
            byte[] value1 = "1".getBytes(StandardCharsets.UTF_8);
            byte[] value2 = "2".getBytes(StandardCharsets.UTF_8);

            // Initial write
            Version v1 = client.put(key, value1, null);
            System.out.println("Wrote value: 1, version=" + v1);

            // Update to new value
            Version v2 = client.put(key, value2, null);
            System.out.println("Wrote value: 2, version=" + v2);

            // Try conditional update with stale version
            byte[] value3 = "3".getBytes(StandardCharsets.UTF_8);
            PutOptions staleOptions = PutOptions.builder()
                    .ifMatchVersion(v1) // v1 is now stale
                    .build();

            try {
                client.put(key, value3, staleOptions);
                System.out.println("ERROR: Should have thrown VersionMismatchException");
            } catch (VersionMismatchException e) {
                System.out.println("Caught VersionMismatchException: " + e.getMessage());
                System.out.println("Expected version: " + v1);
                System.out.println("Actual version: " + v2);
                System.out.println("Action: Re-read and retry with current version");
            }
        } catch (NoriKVException e) {
            System.out.println("Note: This example requires a running NoriKV cluster");
            System.out.println("Error: " + e.getCode() + " - " + e.getMessage());
        }
        System.out.println();

        // Example 5: Idempotency Keys for Safe Retries
        System.out.println("=== Example 5: Idempotency Keys for Safe Retries ===");
        try (NoriKVClient client = new NoriKVClient(defaultConfig)) {
            byte[] orderKey = "order:12345".getBytes(StandardCharsets.UTF_8);
            byte[] orderData = "{\"product\":\"laptop\",\"amount\":999.99}".getBytes(StandardCharsets.UTF_8);

            String idempotencyKey = "order-creation-12345-" + System.currentTimeMillis();

            PutOptions idempotentPut = PutOptions.builder()
                    .idempotencyKey(idempotencyKey)
                    .build();

            // First attempt
            System.out.println("Attempt 1 with idempotency key: " + idempotencyKey);
            Version v1 = client.put(orderKey, orderData, idempotentPut);
            System.out.println("Order created, version=" + v1);

            // Retry with same idempotency key (safe - won't create duplicate)
            System.out.println("Attempt 2 with same idempotency key (simulating retry)");
            Version v2 = client.put(orderKey, orderData, idempotentPut);
            System.out.println("Retry succeeded, version=" + v2);
            System.out.println("Same version indicates idempotent retry: " + v1.equals(v2));
        } catch (NoriKVException e) {
            System.out.println("Note: This example requires a running NoriKV cluster");
            System.out.println("Error: " + e.getCode() + " - " + e.getMessage());
        }
        System.out.println();

        // Example 6: Connection Exception Handling
        System.out.println("=== Example 6: Connection Exception Handling ===");
        ClientConfig unreachableConfig = ClientConfig.builder()
                .nodes(java.util.Arrays.asList("unreachable:9999"))
                .totalShards(1024)
                .timeoutMs(1000) // Short timeout
                .retry(RetryConfig.builder()
                        .maxAttempts(2) // Few retries
                        .initialDelayMs(100)
                        .build())
                .build();

        try (NoriKVClient client = new NoriKVClient(unreachableConfig)) {
            byte[] key = "test".getBytes(StandardCharsets.UTF_8);
            byte[] value = "value".getBytes(StandardCharsets.UTF_8);

            try {
                client.put(key, value, null);
                System.out.println("ERROR: Should have thrown ConnectionException");
            } catch (ConnectionException e) {
                System.out.println("Caught ConnectionException: " + e.getMessage());
                System.out.println("Error code: " + e.getCode());
                System.out.println("This indicates network or cluster issues");
                System.out.println("Recommended action: Check cluster health, retry with backoff");
            }
        } catch (NoriKVException e) {
            System.out.println("Caught retry exhausted: " + e.getCode());
            System.out.println("Message: " + e.getMessage());
        }
        System.out.println();

        // Example 7: Retry Logic for Transient Errors
        System.out.println("=== Example 7: Application-Level Retry Logic ===");
        System.out.println("The client automatically retries transient errors:");
        System.out.println("  - UNAVAILABLE: Retried");
        System.out.println("  - ABORTED: Retried");
        System.out.println("  - DEADLINE_EXCEEDED: Retried");
        System.out.println("  - RESOURCE_EXHAUSTED: Retried");
        System.out.println();
        System.out.println("Non-retryable errors:");
        System.out.println("  - INVALID_ARGUMENT: Not retried (client error)");
        System.out.println("  - NOT_FOUND: Not retried (key doesn't exist)");
        System.out.println("  - FAILED_PRECONDITION: Not retried (version mismatch)");
        System.out.println("  - PERMISSION_DENIED: Not retried (auth error)");
        System.out.println();

        // Example 8: Error Recovery Pattern
        System.out.println("=== Example 8: Error Recovery Pattern ===");
        try (NoriKVClient client = new NoriKVClient(defaultConfig)) {
            byte[] key = "important:data".getBytes(StandardCharsets.UTF_8);
            byte[] value = "{\"critical\":true}".getBytes(StandardCharsets.UTF_8);

            boolean success = writeWithRetry(client, key, value, 3);
            if (success) {
                System.out.println("Write succeeded");
            } else {
                System.out.println("Write failed after retries");
            }
        } catch (Exception e) {
            System.out.println("Note: This example requires a running NoriKV cluster");
            System.out.println("Error: " + e.getMessage());
        }
        System.out.println();

        // Example 9: Graceful Degradation
        System.out.println("=== Example 9: Graceful Degradation ===");
        try (NoriKVClient client = new NoriKVClient(defaultConfig)) {
            byte[] key = "cache:user_profile".getBytes(StandardCharsets.UTF_8);

            String userProfile = getWithFallback(client, key, "{\"name\":\"default\"}");
            System.out.println("User profile: " + userProfile);
        } catch (Exception e) {
            System.out.println("Note: This example requires a running NoriKV cluster");
            System.out.println("Error: " + e.getMessage());
        }
        System.out.println();

        // Example 10: Circuit Breaker Pattern (Conceptual)
        System.out.println("=== Example 10: Circuit Breaker Pattern ===");
        System.out.println("For production systems, consider implementing:");
        System.out.println("  1. Circuit breaker to prevent cascading failures");
        System.out.println("  2. Bulkhead pattern for resource isolation");
        System.out.println("  3. Timeout management at application level");
        System.out.println("  4. Metrics and alerting on error rates");
        System.out.println("  5. Fallback strategies for degraded mode");
        System.out.println();

        System.out.println("All error handling examples completed!");
    }

    /**
     * Writes a value with application-level retry logic.
     *
     * @param client the NoriKV client
     * @param key the key to write
     * @param value the value to write
     * @param maxAttempts maximum number of attempts
     * @return true if write succeeded, false otherwise
     */
    private static boolean writeWithRetry(NoriKVClient client, byte[] key, byte[] value, int maxAttempts) {
        for (int attempt = 1; attempt <= maxAttempts; attempt++) {
            try {
                client.put(key, value, null);
                System.out.println("Write succeeded on attempt " + attempt);
                return true;
            } catch (ConnectionException e) {
                System.out.println("Attempt " + attempt + " failed: " + e.getMessage());
                if (attempt == maxAttempts) {
                    System.out.println("Max attempts reached");
                    return false;
                }
                // Exponential backoff
                try {
                    long delay = (long) (Math.pow(2, attempt) * 100);
                    System.out.println("Backing off for " + delay + "ms");
                    Thread.sleep(delay);
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    return false;
                }
            } catch (NoriKVException e) {
                // Non-retryable error
                System.out.println("Non-retryable error: " + e.getCode() + " - " + e.getMessage());
                return false;
            }
        }
        return false;
    }

    /**
     * Gets a value with fallback to default.
     *
     * @param client the NoriKV client
     * @param key the key to read
     * @param defaultValue the default value if key not found
     * @return the value or default
     */
    private static String getWithFallback(NoriKVClient client, byte[] key, String defaultValue) {
        try {
            GetResult result = client.get(key, null);
            return new String(result.getValue(), StandardCharsets.UTF_8);
        } catch (KeyNotFoundException e) {
            System.out.println("Key not found, using default value");
            return defaultValue;
        } catch (NoriKVException e) {
            System.out.println("Error reading from NoriKV: " + e.getMessage());
            System.out.println("Falling back to default value");
            return defaultValue;
        }
    }
}
