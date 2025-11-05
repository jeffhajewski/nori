package com.norikv.client.integration;

import com.norikv.client.NoriKVClient;
import com.norikv.client.testing.EphemeralServer;
import com.norikv.client.types.*;
import org.junit.jupiter.api.*;
import static org.junit.jupiter.api.Assertions.*;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/**
 * Integration tests using the ephemeral server.
 *
 * <p>These tests exercise the full client stack against a real gRPC server.
 */
@TestInstance(TestInstance.Lifecycle.PER_CLASS)
public class EphemeralServerIntegrationTest {

    private EphemeralServer server;
    private NoriKVClient client;

    @BeforeAll
    public void startServer() throws Exception {
        server = EphemeralServer.start(19001);

        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList(server.getAddress()))
                .totalShards(1024)
                .timeoutMs(5000)
                .build();

        client = new NoriKVClient(config);
    }

    @AfterAll
    public void stopServer() {
        if (client != null) {
            client.close();
        }
        if (server != null) {
            server.stop();
        }
    }

    @BeforeEach
    public void clearData() {
        server.clear();
    }

    @Test
    @DisplayName("Integration: basic put and get")
    public void testBasicPutGet() throws Exception {
        byte[] key = "user:alice".getBytes(StandardCharsets.UTF_8);
        byte[] value = "{\"name\":\"Alice\"}".getBytes(StandardCharsets.UTF_8);

        // Put
        Version version = client.put(key, value, null);
        assertNotNull(version);
        assertTrue(version.getIndex() > 0);

        // Get
        GetResult result = client.get(key, null);
        assertArrayEquals(value, result.getValue());
        assertEquals(version, result.getVersion());
    }

    @Test
    @DisplayName("Integration: get non-existent key throws KeyNotFoundException")
    public void testGetNonExistent() {
        byte[] key = "nonexistent".getBytes(StandardCharsets.UTF_8);
        assertThrows(KeyNotFoundException.class, () -> client.get(key, null));
    }

    @Test
    @DisplayName("Integration: put with TTL expires")
    public void testPutWithTTL() throws Exception {
        byte[] key = "session:temp".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        PutOptions options = PutOptions.builder()
                .ttlMs(100L) // 100ms TTL
                .build();

        client.put(key, value, options);

        // Read immediately - should exist
        GetResult result = client.get(key, null);
        assertArrayEquals(value, result.getValue());

        // Wait for expiration
        Thread.sleep(150);

        // Read again - should be gone
        assertThrows(KeyNotFoundException.class, () -> client.get(key, null));
    }

    @Test
    @DisplayName("Integration: put with idempotency key")
    public void testPutIdempotency() throws Exception {
        byte[] key = "order:123".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        PutOptions options = PutOptions.builder()
                .idempotencyKey("order-creation-123")
                .build();

        // First write
        Version v1 = client.put(key, value, options);

        // Retry with same idempotency key
        Version v2 = client.put(key, value, options);

        // Should return same version
        assertEquals(v1, v2);
    }

    @Test
    @DisplayName("Integration: conditional put with version match")
    public void testConditionalPut() throws Exception {
        byte[] key = "counter".getBytes(StandardCharsets.UTF_8);
        byte[] value1 = "1".getBytes(StandardCharsets.UTF_8);
        byte[] value2 = "2".getBytes(StandardCharsets.UTF_8);

        // Initial write
        Version v1 = client.put(key, value1, null);

        // Conditional update with matching version
        PutOptions options = PutOptions.builder()
                .ifMatchVersion(v1)
                .build();

        Version v2 = client.put(key, value2, options);
        assertNotEquals(v1, v2);

        // Verify new value
        GetResult result = client.get(key, null);
        assertArrayEquals(value2, result.getValue());
        assertEquals(v2, result.getVersion());
    }

    @Test
    @DisplayName("Integration: conditional put with stale version fails")
    public void testConditionalPutStaleVersion() throws Exception {
        byte[] key = "counter".getBytes(StandardCharsets.UTF_8);
        byte[] value1 = "1".getBytes(StandardCharsets.UTF_8);
        byte[] value2 = "2".getBytes(StandardCharsets.UTF_8);
        byte[] value3 = "3".getBytes(StandardCharsets.UTF_8);

        // Initial write
        Version v1 = client.put(key, value1, null);

        // Update to v2
        Version v2 = client.put(key, value2, null);

        // Try to update with stale version v1
        PutOptions staleOptions = PutOptions.builder()
                .ifMatchVersion(v1)
                .build();

        assertThrows(VersionMismatchException.class, () -> client.put(key, value3, staleOptions));

        // Verify value is still v2
        GetResult result = client.get(key, null);
        assertArrayEquals(value2, result.getValue());
        assertEquals(v2, result.getVersion());
    }

    @Test
    @DisplayName("Integration: delete removes key")
    public void testDelete() throws Exception {
        byte[] key = "temp".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        // Create key
        client.put(key, value, null);
        GetResult result = client.get(key, null);
        assertNotNull(result);

        // Delete key
        boolean deleted = client.delete(key, null);
        assertTrue(deleted);

        // Verify key is gone
        assertThrows(KeyNotFoundException.class, () -> client.get(key, null));
    }

    @Test
    @DisplayName("Integration: delete non-existent key returns false")
    public void testDeleteNonExistent() throws Exception {
        byte[] key = "nonexistent".getBytes(StandardCharsets.UTF_8);
        boolean deleted = client.delete(key, null);
        assertFalse(deleted);
    }

    @Test
    @DisplayName("Integration: conditional delete with version match")
    public void testConditionalDelete() throws Exception {
        byte[] key = "temp".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        // Create key
        Version version = client.put(key, value, null);

        // Conditional delete with matching version
        DeleteOptions options = DeleteOptions.builder()
                .ifMatchVersion(version)
                .build();

        boolean deleted = client.delete(key, options);
        assertTrue(deleted);

        // Verify key is gone
        assertThrows(KeyNotFoundException.class, () -> client.get(key, null));
    }

    @Test
    @DisplayName("Integration: conditional delete with stale version fails")
    public void testConditionalDeleteStaleVersion() throws Exception {
        byte[] key = "temp".getBytes(StandardCharsets.UTF_8);
        byte[] value1 = "data1".getBytes(StandardCharsets.UTF_8);
        byte[] value2 = "data2".getBytes(StandardCharsets.UTF_8);

        // Create key
        Version v1 = client.put(key, value1, null);

        // Update key
        Version v2 = client.put(key, value2, null);

        // Try to delete with stale version
        DeleteOptions staleOptions = DeleteOptions.builder()
                .ifMatchVersion(v1)
                .build();

        assertThrows(VersionMismatchException.class, () -> client.delete(key, staleOptions));

        // Verify key still exists with v2
        GetResult result = client.get(key, null);
        assertArrayEquals(value2, result.getValue());
        assertEquals(v2, result.getVersion());
    }

    @Test
    @DisplayName("Integration: delete with idempotency key")
    public void testDeleteIdempotency() throws Exception {
        byte[] key = "order:123".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        // Create key
        client.put(key, value, null);

        DeleteOptions options = DeleteOptions.builder()
                .idempotencyKey("delete-order-123")
                .build();

        // First delete
        boolean deleted1 = client.delete(key, options);
        assertTrue(deleted1);

        // Retry with same idempotency key
        boolean deleted2 = client.delete(key, options);
        assertTrue(deleted2); // Returns true from cache
    }

    @Test
    @DisplayName("Integration: multiple keys")
    public void testMultipleKeys() throws Exception {
        int numKeys = 100;

        // Write keys
        for (int i = 0; i < numKeys; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = ("value-" + i).getBytes(StandardCharsets.UTF_8);
            client.put(key, value, null);
        }

        // Read keys
        for (int i = 0; i < numKeys; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            GetResult result = client.get(key, null);
            String expectedValue = "value-" + i;
            assertEquals(expectedValue, new String(result.getValue(), StandardCharsets.UTF_8));
        }

        // Verify count
        assertEquals(numKeys, server.size());
    }

    @Test
    @DisplayName("Integration: consistency levels accepted")
    public void testConsistencyLevels() throws Exception {
        byte[] key = "test".getBytes(StandardCharsets.UTF_8);
        byte[] value = "data".getBytes(StandardCharsets.UTF_8);

        client.put(key, value, null);

        // Lease read
        GetOptions leaseOptions = GetOptions.builder()
                .consistency(ConsistencyLevel.LEASE)
                .build();
        GetResult leaseResult = client.get(key, leaseOptions);
        assertNotNull(leaseResult);

        // Linearizable read
        GetOptions linearOptions = GetOptions.builder()
                .consistency(ConsistencyLevel.LINEARIZABLE)
                .build();
        GetResult linearResult = client.get(key, linearOptions);
        assertNotNull(linearResult);

        // Stale read
        GetOptions staleOptions = GetOptions.builder()
                .consistency(ConsistencyLevel.STALE_OK)
                .build();
        GetResult staleResult = client.get(key, staleOptions);
        assertNotNull(staleResult);
    }

    @Test
    @DisplayName("Integration: version increases monotonically")
    public void testVersionMonotonicity() throws Exception {
        byte[] key = "counter".getBytes(StandardCharsets.UTF_8);

        Version prev = null;
        for (int i = 0; i < 10; i++) {
            byte[] value = String.valueOf(i).getBytes(StandardCharsets.UTF_8);
            Version version = client.put(key, value, null);

            if (prev != null) {
                assertTrue(version.getIndex() > prev.getIndex(),
                        "Version should increase: " + prev.getIndex() + " -> " + version.getIndex());
            }
            prev = version;
        }
    }
}
