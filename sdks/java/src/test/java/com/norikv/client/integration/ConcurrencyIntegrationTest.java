package com.norikv.client.integration;

import com.norikv.client.NoriKVClient;
import com.norikv.client.testing.EphemeralServer;
import com.norikv.client.types.*;
import org.junit.jupiter.api.*;

import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Integration tests focusing on concurrency and thread safety.
 *
 * <p>Tests concurrent access patterns, race conditions, and thread safety
 * guarantees of the client.
 */
@TestInstance(TestInstance.Lifecycle.PER_CLASS)
public class ConcurrencyIntegrationTest {

    private EphemeralServer server;
    private NoriKVClient client;

    @BeforeAll
    public void setup() throws Exception {
        server = EphemeralServer.start(19003);

        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList(server.getAddress()))
                .totalShards(1024)
                .timeoutMs(5000)
                .build();

        client = new NoriKVClient(config);
    }

    @AfterAll
    public void teardown() {
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
    @DisplayName("Concurrency: Multiple threads incrementing shared counter")
    public void testConcurrentCounterIncrement() throws Exception {
        int numThreads = 10;
        int incrementsPerThread = 50;
        int maxRetries = 500; // Very high retry limit for extreme contention
        byte[] key = "shared-counter".getBytes(StandardCharsets.UTF_8);

        // Initialize counter
        client.put(key, "0".getBytes(StandardCharsets.UTF_8), null);

        ExecutorService executor = Executors.newFixedThreadPool(numThreads);
        CountDownLatch startLatch = new CountDownLatch(1);
        CountDownLatch doneLatch = new CountDownLatch(numThreads);
        AtomicInteger successCount = new AtomicInteger(0);
        AtomicInteger conflictCount = new AtomicInteger(0);
        AtomicInteger failedCount = new AtomicInteger(0);

        for (int t = 0; t < numThreads; t++) {
            executor.submit(() -> {
                try {
                    startLatch.await();

                    for (int i = 0; i < incrementsPerThread; i++) {
                        boolean success = false;
                        int retries = 0;

                        while (!success && retries < maxRetries) {
                            try {
                                GetResult result = client.get(key, null);
                                int currentValue = Integer.parseInt(
                                    new String(result.getValue(), StandardCharsets.UTF_8));

                                int newValue = currentValue + 1;
                                PutOptions options = PutOptions.builder()
                                        .ifMatchVersion(result.getVersion())
                                        .build();

                                client.put(key, String.valueOf(newValue).getBytes(StandardCharsets.UTF_8), options);
                                successCount.incrementAndGet();
                                success = true;
                            } catch (VersionMismatchException e) {
                                conflictCount.incrementAndGet();
                                retries++;
                                if (retries < maxRetries) {
                                    // Exponential backoff with jitter
                                    long backoff = Math.min(1 << Math.min(retries, 10), 100);
                                    Thread.sleep(backoff);
                                }
                            }
                        }

                        if (!success) {
                            failedCount.incrementAndGet();
                            System.err.println("Failed to increment after " + maxRetries + " retries");
                        }
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        startLatch.countDown();
        boolean completed = doneLatch.await(60, TimeUnit.SECONDS);
        executor.shutdown();

        assertTrue(completed, "Test should complete within 60 seconds");

        // Verify final count
        GetResult finalResult = client.get(key, null);
        int finalValue = Integer.parseInt(new String(finalResult.getValue(), StandardCharsets.UTF_8));

        System.out.println("Final counter value: " + finalValue);
        System.out.println("Successful writes: " + successCount.get());
        System.out.println("Version conflicts: " + conflictCount.get());
        System.out.println("Failed writes: " + failedCount.get());

        // Assert no operations failed
        assertEquals(0, failedCount.get(),
            "No increments should fail after " + maxRetries + " retries");

        assertEquals(numThreads * incrementsPerThread, finalValue,
            "Counter should equal total increments");
        assertTrue(conflictCount.get() > 0, "Should have version conflicts with concurrent access");
    }

    @Test
    @DisplayName("Concurrency: High throughput read/write mix")
    public void testHighThroughputMixed() throws Exception {
        int numThreads = 20;
        int opsPerThread = 100;

        // Pre-populate some data
        for (int i = 0; i < 100; i++) {
            byte[] key = ("initial-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = ("value-" + i).getBytes(StandardCharsets.UTF_8);
            client.put(key, value, null);
        }

        ExecutorService executor = Executors.newFixedThreadPool(numThreads);
        CountDownLatch startLatch = new CountDownLatch(1);
        CountDownLatch doneLatch = new CountDownLatch(numThreads);
        AtomicInteger successfulOps = new AtomicInteger(0);
        AtomicInteger failedOps = new AtomicInteger(0);

        for (int t = 0; t < numThreads; t++) {
            final int threadId = t;
            executor.submit(() -> {
                try {
                    startLatch.await();
                    Random random = new Random(threadId);

                    for (int i = 0; i < opsPerThread; i++) {
                        try {
                            String keyStr = "key-" + random.nextInt(200);
                            byte[] key = keyStr.getBytes(StandardCharsets.UTF_8);

                            if (random.nextBoolean()) {
                                // Write
                                byte[] value = ("thread-" + threadId + "-op-" + i)
                                        .getBytes(StandardCharsets.UTF_8);
                                client.put(key, value, null);
                            } else {
                                // Read
                                try {
                                    client.get(key, null);
                                } catch (KeyNotFoundException e) {
                                    // Expected for non-existent keys
                                }
                            }
                            successfulOps.incrementAndGet();
                        } catch (Exception e) {
                            failedOps.incrementAndGet();
                        }
                    }
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        startLatch.countDown();
        boolean completed = doneLatch.await(60, TimeUnit.SECONDS);
        executor.shutdown();

        assertTrue(completed, "Test should complete within 60 seconds");

        int totalOps = numThreads * opsPerThread;
        double successRate = (successfulOps.get() * 100.0) / totalOps;

        System.out.println("Total operations: " + totalOps);
        System.out.println("Successful: " + successfulOps.get());
        System.out.println("Failed: " + failedOps.get());
        System.out.println("Success rate: " + String.format("%.2f%%", successRate));

        assertTrue(successRate > 99.0, "Success rate should be > 99%");
    }

    @Test
    @DisplayName("Concurrency: Rapid sequential operations on same key")
    public void testRapidSequentialSameKey() throws Exception {
        byte[] key = "rapid-test".getBytes(StandardCharsets.UTF_8);
        int numOps = 1000;

        for (int i = 0; i < numOps; i++) {
            byte[] value = ("value-" + i).getBytes(StandardCharsets.UTF_8);
            Version version = client.put(key, value, null);
            assertNotNull(version);

            GetResult result = client.get(key, null);
            assertArrayEquals(value, result.getValue());
        }

        // Final read to verify latest value
        GetResult finalResult = client.get(key, null);
        String expectedFinalValue = "value-" + (numOps - 1);
        assertEquals(expectedFinalValue, new String(finalResult.getValue(), StandardCharsets.UTF_8));
    }

    @Test
    @DisplayName("Concurrency: Multiple clients to same server")
    public void testMultipleClients() throws Exception {
        // Create additional clients
        List<NoriKVClient> clients = new ArrayList<>();
        for (int i = 0; i < 5; i++) {
            ClientConfig config = ClientConfig.builder()
                    .nodes(Arrays.asList(server.getAddress()))
                    .totalShards(1024)
                    .timeoutMs(5000)
                    .build();
            clients.add(new NoriKVClient(config));
        }

        try {
            int opsPerClient = 50;

            ExecutorService executor = Executors.newFixedThreadPool(clients.size());
            CountDownLatch doneLatch = new CountDownLatch(clients.size());

            for (int c = 0; c < clients.size(); c++) {
                final int clientId = c;
                final NoriKVClient clientInstance = clients.get(c);

                executor.submit(() -> {
                    try {
                        for (int i = 0; i < opsPerClient; i++) {
                            byte[] key = ("client-" + clientId + "-key-" + i).getBytes(StandardCharsets.UTF_8);
                            byte[] value = ("value-" + i).getBytes(StandardCharsets.UTF_8);

                            clientInstance.put(key, value, null);
                            GetResult result = clientInstance.get(key, null);
                            assertArrayEquals(value, result.getValue());
                        }
                    } catch (Exception e) {
                        e.printStackTrace();
                        fail("Client " + clientId + " failed: " + e.getMessage());
                    } finally {
                        doneLatch.countDown();
                    }
                });
            }

            boolean completed = doneLatch.await(30, TimeUnit.SECONDS);
            executor.shutdown();

            assertTrue(completed, "All clients should complete within 30 seconds");

        } finally {
            for (NoriKVClient c : clients) {
                c.close();
            }
        }
    }

    @Test
    @DisplayName("Concurrency: Stress test with varying value sizes")
    public void testVaryingValueSizes() throws Exception {
        int numThreads = 10;
        int opsPerThread = 50;

        ExecutorService executor = Executors.newFixedThreadPool(numThreads);
        CountDownLatch doneLatch = new CountDownLatch(numThreads);
        AtomicInteger operations = new AtomicInteger(0);

        for (int t = 0; t < numThreads; t++) {
            final int threadId = t;
            executor.submit(() -> {
                try {
                    Random random = new Random(threadId);

                    for (int i = 0; i < opsPerThread; i++) {
                        // Vary value size from 10 bytes to 10KB
                        int valueSize = 10 + random.nextInt(10 * 1024);
                        byte[] value = new byte[valueSize];
                        random.nextBytes(value);

                        byte[] key = ("thread-" + threadId + "-op-" + i).getBytes(StandardCharsets.UTF_8);

                        client.put(key, value, null);
                        GetResult result = client.get(key, null);
                        assertArrayEquals(value, result.getValue());

                        operations.incrementAndGet();
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        boolean completed = doneLatch.await(60, TimeUnit.SECONDS);
        executor.shutdown();

        assertTrue(completed, "Test should complete within 60 seconds");
        assertEquals(numThreads * opsPerThread, operations.get(),
            "All operations should complete successfully");
    }

    @Test
    @DisplayName("Concurrency: Delete while reading")
    public void testConcurrentReadDelete() throws Exception {
        int numKeys = 50;

        // Pre-populate keys
        for (int i = 0; i < numKeys; i++) {
            byte[] key = ("delete-test-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = ("value-" + i).getBytes(StandardCharsets.UTF_8);
            client.put(key, value, null);
        }

        ExecutorService executor = Executors.newFixedThreadPool(numKeys);
        CountDownLatch startLatch = new CountDownLatch(1);
        CountDownLatch doneLatch = new CountDownLatch(numKeys * 2); // readers + deleters

        AtomicInteger readSuccess = new AtomicInteger(0);
        AtomicInteger readNotFound = new AtomicInteger(0);
        AtomicInteger deleteSuccess = new AtomicInteger(0);

        // Start readers
        for (int i = 0; i < numKeys; i++) {
            final int keyId = i;
            executor.submit(() -> {
                try {
                    startLatch.await();
                    byte[] key = ("delete-test-" + keyId).getBytes(StandardCharsets.UTF_8);

                    for (int attempt = 0; attempt < 10; attempt++) {
                        try {
                            client.get(key, null);
                            readSuccess.incrementAndGet();
                        } catch (KeyNotFoundException e) {
                            readNotFound.incrementAndGet();
                        }
                        Thread.sleep(1);
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        // Start deleters
        for (int i = 0; i < numKeys; i++) {
            final int keyId = i;
            executor.submit(() -> {
                try {
                    startLatch.await();
                    Thread.sleep(5); // Let some reads happen first

                    byte[] key = ("delete-test-" + keyId).getBytes(StandardCharsets.UTF_8);
                    boolean deleted = client.delete(key, null);
                    if (deleted) {
                        deleteSuccess.incrementAndGet();
                    }
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        startLatch.countDown();
        boolean completed = doneLatch.await(30, TimeUnit.SECONDS);
        executor.shutdown();

        assertTrue(completed, "Test should complete within 30 seconds");

        System.out.println("Read success: " + readSuccess.get());
        System.out.println("Read not found: " + readNotFound.get());
        System.out.println("Deletes: " + deleteSuccess.get());

        assertTrue(readSuccess.get() > 0, "Some reads should succeed before deletes");
        // Note: Timing-dependent - reads might all complete before deletes start
        // The important thing is no crashes and operations are thread-safe
        assertEquals(numKeys, deleteSuccess.get(), "All deletes should succeed");
    }
}
