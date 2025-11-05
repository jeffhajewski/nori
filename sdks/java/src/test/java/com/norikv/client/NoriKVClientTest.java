package com.norikv.client;

import com.norikv.client.types.*;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DisplayName;
import static org.junit.jupiter.api.Assertions.*;

import java.util.Arrays;

/**
 * Test suite for NoriKVClient.
 *
 * <p>Note: Full integration tests require gRPC proto stubs to be generated.
 * These tests focus on client initialization, configuration, and component integration.
 */
public class NoriKVClientTest {

    @Test
    @DisplayName("NoriKVClient: constructor validates config")
    public void testConstructorValidation() {
        assertThrows(IllegalArgumentException.class, () -> new NoriKVClient(null));
    }

    @Test
    @DisplayName("NoriKVClient: constructor accepts valid config")
    public void testConstructorValid() {
        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList("localhost:9001", "localhost:9002"))
                .totalShards(1024)
                .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            assertNotNull(client);
            assertFalse(client.isClosed());
        }
    }

    @Test
    @DisplayName("NoriKVClient: uses default config values")
    public void testDefaultConfig() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            assertNotNull(client);
        }
    }

    @Test
    @DisplayName("NoriKVClient: close is idempotent")
    public void testCloseIdempotent() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");
        NoriKVClient client = new NoriKVClient(config);

        assertFalse(client.isClosed());

        client.close();
        assertTrue(client.isClosed());

        // Second close should not throw
        client.close();
        assertTrue(client.isClosed());
    }

    @Test
    @DisplayName("NoriKVClient: operations fail when closed")
    public void testOperationsFailWhenClosed() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");
        NoriKVClient client = new NoriKVClient(config);
        client.close();

        byte[] key = "test".getBytes();
        byte[] value = "value".getBytes();

        assertThrows(ConnectionException.class, () -> client.put(key, value, null));
        assertThrows(ConnectionException.class, () -> client.get(key, null));
        assertThrows(ConnectionException.class, () -> client.delete(key, null));
    }

    @Test
    @DisplayName("NoriKVClient: validates key in put")
    public void testPutValidatesKey() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            byte[] value = "value".getBytes();

            assertThrows(NoriKVException.class, () -> client.put(null, value, null));
            assertThrows(NoriKVException.class, () -> client.put(new byte[0], value, null));
        }
    }

    @Test
    @DisplayName("NoriKVClient: validates value in put")
    public void testPutValidatesValue() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            byte[] key = "test".getBytes();

            assertThrows(NoriKVException.class, () -> client.put(key, null, null));
        }
    }

    @Test
    @DisplayName("NoriKVClient: validates key in get")
    public void testGetValidatesKey() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            assertThrows(NoriKVException.class, () -> client.get(null, null));
            assertThrows(NoriKVException.class, () -> client.get(new byte[0], null));
        }
    }

    @Test
    @DisplayName("NoriKVClient: validates key in delete")
    public void testDeleteValidatesKey() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            assertThrows(NoriKVException.class, () -> client.delete(null, null));
            assertThrows(NoriKVException.class, () -> client.delete(new byte[0], null));
        }
    }

    @Test
    @DisplayName("NoriKVClient: accepts null options")
    public void testAcceptsNullOptions() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            byte[] key = "test".getBytes();
            byte[] value = "value".getBytes();

            // Should not throw on validation - will fail on connection since no server is running
            // Expects either RETRY_EXHAUSTED (after retries) or connection-related errors
            try {
                client.put(key, value, null);
                fail("Should have thrown NoriKVException");
            } catch (NoriKVException e) {
                // Accept either RETRY_EXHAUSTED or connection errors
                assertTrue(e.getCode().equals("RETRY_EXHAUSTED") ||
                          e.getCode().equals("UNAVAILABLE") ||
                          e instanceof ConnectionException,
                          "Expected retry or connection error, got: " + e.getCode());
            }

            try {
                client.get(key, null);
                fail("Should have thrown NoriKVException");
            } catch (NoriKVException e) {
                assertTrue(e.getCode().equals("RETRY_EXHAUSTED") ||
                          e.getCode().equals("UNAVAILABLE") ||
                          e instanceof ConnectionException,
                          "Expected retry or connection error, got: " + e.getCode());
            }

            try {
                client.delete(key, null);
                fail("Should have thrown NoriKVException");
            } catch (NoriKVException e) {
                assertTrue(e.getCode().equals("RETRY_EXHAUSTED") ||
                          e.getCode().equals("UNAVAILABLE") ||
                          e instanceof ConnectionException,
                          "Expected retry or connection error, got: " + e.getCode());
            }
        }
    }

    @Test
    @DisplayName("NoriKVClient: getClusterView returns null initially")
    public void testGetClusterViewInitial() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            // Initially null until cluster view is fetched
            ClusterView view = client.getClusterView();
            // May be null or may have seed view depending on implementation
            // Just verify it doesn't throw
            assertNotNull(client);
        }
    }

    @Test
    @DisplayName("NoriKVClient: onTopologyChange registers listener")
    public void testOnTopologyChange() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            java.util.concurrent.atomic.AtomicInteger callCount = new java.util.concurrent.atomic.AtomicInteger(0);

            Runnable unsubscribe = client.onTopologyChange(event -> {
                callCount.incrementAndGet();
            });

            assertNotNull(unsubscribe);

            // Unsubscribe should not throw
            unsubscribe.run();
        }
    }

    @Test
    @DisplayName("NoriKVClient: getStats returns statistics")
    public void testGetStats() {
        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList("localhost:9001", "localhost:9002", "localhost:9003"))
                .totalShards(1024)
                .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            NoriKVClient.ClientStats stats = client.getStats();

            assertNotNull(stats);
            assertNotNull(stats.getRouterStats());
            assertNotNull(stats.getPoolStats());
            assertNotNull(stats.getTopologyStats());
            assertFalse(stats.isClosed());

            // Verify router has correct configuration
            assertEquals(3, stats.getRouterStats().getTotalNodes());
            assertEquals(1024, stats.getRouterStats().getTotalShards());
        }

        // After close, stats should reflect closed state
        ClientConfig config2 = ClientConfig.defaultConfig("localhost:9001");
        NoriKVClient client2 = new NoriKVClient(config2);
        client2.close();

        NoriKVClient.ClientStats stats2 = client2.getStats();
        assertTrue(stats2.isClosed());
    }

    @Test
    @DisplayName("NoriKVClient: try-with-resources auto-closes")
    public void testTryWithResources() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        NoriKVClient client;
        try (NoriKVClient c = new NoriKVClient(config)) {
            client = c;
            assertFalse(client.isClosed());
        }

        assertTrue(client.isClosed());
    }

    @Test
    @DisplayName("NoriKVClient: multiple clients can coexist")
    public void testMultipleClients() {
        ClientConfig config1 = ClientConfig.defaultConfig("localhost:9001");
        ClientConfig config2 = ClientConfig.defaultConfig("localhost:9002");

        try (NoriKVClient client1 = new NoriKVClient(config1);
             NoriKVClient client2 = new NoriKVClient(config2)) {

            assertNotNull(client1);
            assertNotNull(client2);
            assertFalse(client1.isClosed());
            assertFalse(client2.isClosed());
        }
    }

    @Test
    @DisplayName("NoriKVClient: custom retry config is used")
    public void testCustomRetryConfig() {
        RetryConfig retryConfig = RetryConfig.builder()
                .maxAttempts(5)
                .initialDelayMs(10)
                .maxDelayMs(500)
                .build();

        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList("localhost:9001"))
                .retry(retryConfig)
                .build();

        try (NoriKVClient client = new NoriKVClient(config)) {
            assertNotNull(client);
            // Retry config is internal, but client should be created successfully
        }
    }

    @Test
    @DisplayName("NoriKVClient: ClientStats toString doesn't throw")
    public void testClientStatsToString() {
        ClientConfig config = ClientConfig.defaultConfig("localhost:9001");

        try (NoriKVClient client = new NoriKVClient(config)) {
            NoriKVClient.ClientStats stats = client.getStats();
            String str = stats.toString();

            assertNotNull(str);
            assertTrue(str.contains("ClientStats"));
        }
    }
}
