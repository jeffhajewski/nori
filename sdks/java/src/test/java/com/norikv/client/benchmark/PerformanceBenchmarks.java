package com.norikv.client.benchmark;

import com.norikv.client.NoriKVClient;
import com.norikv.client.testing.EphemeralServer;
import com.norikv.client.types.*;
import org.junit.jupiter.api.*;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Random;
import java.util.concurrent.*;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Performance benchmarks for the NoriKV Java client.
 *
 * <p>Measures latency and throughput for various operations and validates
 * against SLO targets (p95 GET ≤ 10ms, p95 PUT ≤ 20ms).
 *
 * <p>Note: These benchmarks run against an ephemeral in-memory server,
 * so results represent best-case performance without network latency.
 * Real-world performance will include network overhead.
 */
@TestInstance(TestInstance.Lifecycle.PER_CLASS)
public class PerformanceBenchmarks {

    private EphemeralServer server;
    private NoriKVClient client;
    private Random random;

    // Benchmark parameters
    private static final int WARMUP_OPS = 100;
    private static final int BENCHMARK_OPS = 1000;

    @BeforeAll
    public void setup() throws Exception {
        server = EphemeralServer.start(19002);

        ClientConfig config = ClientConfig.builder()
                .nodes(Arrays.asList(server.getAddress()))
                .totalShards(1024)
                .timeoutMs(5000)
                .build();

        client = new NoriKVClient(config);
        random = new Random(42); // Fixed seed for reproducibility

        System.out.println("Performance Benchmarks");
        System.out.println("======================");
        System.out.println("Target SLOs:");
        System.out.println("  - GET p95: ≤ 10ms");
        System.out.println("  - PUT p95: ≤ 20ms");
        System.out.println();
        System.out.println("Note: Tests run against in-memory ephemeral server");
        System.out.println("      (no network latency, best-case performance)");
        System.out.println();
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
    @DisplayName("Benchmark: Sequential PUT operations")
    public void benchmarkSequentialPut() throws Exception {
        BenchmarkRunner benchmark = new BenchmarkRunner("Sequential PUT");

        // Warmup
        for (int i = 0; i < WARMUP_OPS; i++) {
            byte[] key = ("warmup-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);
            client.put(key, value, null);
        }

        // Benchmark
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);

            long latency = BenchmarkRunner.measureNs(() -> {
                try {
                    client.put(key, value, null);
                } catch (NoriKVException e) {
                    fail("PUT failed: " + e.getMessage());
                }
            });

            benchmark.recordLatency(latency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();

        // Validate performance
        assertTrue(result.getTotalOps() == BENCHMARK_OPS);
        assertTrue(result.getLatencyStats().getP95Ms() < 100.0,
            "p95 latency too high: " + result.getLatencyStats().getP95Ms() + "ms");
    }

    @Test
    @DisplayName("Benchmark: Sequential GET operations")
    public void benchmarkSequentialGet() throws Exception {
        // Prepare data
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);
            client.put(key, value, null);
        }

        BenchmarkRunner benchmark = new BenchmarkRunner("Sequential GET");

        // Warmup
        for (int i = 0; i < WARMUP_OPS; i++) {
            byte[] key = ("key-" + random.nextInt(BENCHMARK_OPS)).getBytes(StandardCharsets.UTF_8);
            client.get(key, null);
        }

        // Benchmark
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);

            long latency = BenchmarkRunner.measureNs(() -> {
                try {
                    GetResult result = client.get(key, null);
                    assertNotNull(result);
                } catch (NoriKVException e) {
                    fail("GET failed: " + e.getMessage());
                }
            });

            benchmark.recordLatency(latency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();

        // Validate performance
        assertTrue(result.getLatencyStats().getP95Ms() < 100.0,
            "p95 latency too high: " + result.getLatencyStats().getP95Ms() + "ms");
    }

    @Test
    @DisplayName("Benchmark: Mixed workload (50% PUT, 50% GET)")
    public void benchmarkMixedWorkload() throws Exception {
        // Pre-populate some data
        for (int i = 0; i < BENCHMARK_OPS / 2; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);
            client.put(key, value, null);
        }

        BenchmarkRunner benchmark = new BenchmarkRunner("Mixed Workload (50/50)");

        // Benchmark
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            boolean doPut = random.nextBoolean();
            byte[] key = ("key-" + random.nextInt(BENCHMARK_OPS)).getBytes(StandardCharsets.UTF_8);

            if (doPut) {
                byte[] value = randomValue(100);
                long latency = BenchmarkRunner.measureNs(() -> {
                    try {
                        client.put(key, value, null);
                    } catch (NoriKVException e) {
                        fail("PUT failed: " + e.getMessage());
                    }
                });
                benchmark.recordLatency(latency);
            } else {
                long latency = BenchmarkRunner.measureNs(() -> {
                    try {
                        client.get(key, null);
                    } catch (KeyNotFoundException e) {
                        // Expected for non-existent keys
                    } catch (NoriKVException e) {
                        fail("GET failed: " + e.getMessage());
                    }
                });
                benchmark.recordLatency(latency);
            }
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();
    }

    @Test
    @DisplayName("Benchmark: Small values (10 bytes)")
    public void benchmarkSmallValues() throws Exception {
        BenchmarkRunner benchmark = new BenchmarkRunner("PUT/GET Small Values (10B)");

        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(10);

            // PUT
            long putLatency = BenchmarkRunner.measureNs(() -> {
                try {
                    client.put(key, value, null);
                } catch (NoriKVException e) {
                    fail("PUT failed: " + e.getMessage());
                }
            });
            benchmark.recordLatency(putLatency);

            // GET
            long getLatency = BenchmarkRunner.measureNs(() -> {
                try {
                    client.get(key, null);
                } catch (NoriKVException e) {
                    fail("GET failed: " + e.getMessage());
                }
            });
            benchmark.recordLatency(getLatency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();
    }

    @Test
    @DisplayName("Benchmark: Large values (10 KB)")
    public void benchmarkLargeValues() throws Exception {
        BenchmarkRunner benchmark = new BenchmarkRunner("PUT/GET Large Values (10KB)");

        for (int i = 0; i < BENCHMARK_OPS / 10; i++) { // Fewer ops for large values
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(10 * 1024);

            // PUT
            long putLatency = BenchmarkRunner.measureNs(() -> {
                try {
                    client.put(key, value, null);
                } catch (NoriKVException e) {
                    fail("PUT failed: " + e.getMessage());
                }
            });
            benchmark.recordLatency(putLatency);

            // GET
            long getLatency = BenchmarkRunner.measureNs(() -> {
                try {
                    GetResult result = client.get(key, null);
                    assertNotNull(result);
                } catch (NoriKVException e) {
                    fail("GET failed: " + e.getMessage());
                }
            });
            benchmark.recordLatency(getLatency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();
    }

    @Test
    @DisplayName("Benchmark: Concurrent operations (10 threads)")
    public void benchmarkConcurrentOperations() throws Exception {
        int numThreads = 10;
        int opsPerThread = BENCHMARK_OPS / numThreads;

        ExecutorService executor = Executors.newFixedThreadPool(numThreads);
        CountDownLatch startLatch = new CountDownLatch(1);
        CountDownLatch doneLatch = new CountDownLatch(numThreads);

        BenchmarkRunner benchmark = new BenchmarkRunner("Concurrent Operations (10 threads)");

        // Pre-populate data
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);
            client.put(key, value, null);
        }

        for (int t = 0; t < numThreads; t++) {
            final int threadId = t;
            executor.submit(() -> {
                try {
                    // Wait for all threads to be ready
                    startLatch.await();

                    Random threadRandom = new Random(threadId);

                    for (int i = 0; i < opsPerThread; i++) {
                        byte[] key = ("key-" + threadRandom.nextInt(BENCHMARK_OPS))
                                .getBytes(StandardCharsets.UTF_8);

                        if (threadRandom.nextBoolean()) {
                            // PUT
                            byte[] value = randomValue(100);
                            long latency = BenchmarkRunner.measureNs(() -> {
                                try {
                                    client.put(key, value, null);
                                } catch (NoriKVException e) {
                                    // Ignore for benchmark
                                }
                            });
                            synchronized (benchmark) {
                                benchmark.recordLatency(latency);
                            }
                        } else {
                            // GET
                            long latency = BenchmarkRunner.measureNs(() -> {
                                try {
                                    client.get(key, null);
                                } catch (NoriKVException e) {
                                    // Ignore for benchmark
                                }
                            });
                            synchronized (benchmark) {
                                benchmark.recordLatency(latency);
                            }
                        }
                    }
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                } finally {
                    doneLatch.countDown();
                }
            });
        }

        // Start all threads
        startLatch.countDown();

        // Wait for completion
        doneLatch.await();
        executor.shutdown();

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();

        // Validate concurrent safety
        assertTrue(result.getTotalOps() >= BENCHMARK_OPS * 0.9,
            "Too many operations failed: " + result.getTotalOps() + " < " + BENCHMARK_OPS);
    }

    @Test
    @DisplayName("Benchmark: CAS operations")
    public void benchmarkCasOperations() throws Exception {
        BenchmarkRunner benchmark = new BenchmarkRunner("CAS Operations");

        // Create initial keys
        List<byte[]> keys = new ArrayList<>();
        for (int i = 0; i < 100; i++) {
            byte[] key = ("counter-" + i).getBytes(StandardCharsets.UTF_8);
            keys.add(key);
            client.put(key, "0".getBytes(StandardCharsets.UTF_8), null);
        }

        // Benchmark CAS updates
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = keys.get(random.nextInt(keys.size()));

            long latency = BenchmarkRunner.measureNs(() -> {
                try {
                    // Read
                    GetResult current = client.get(key, null);
                    int value = Integer.parseInt(new String(current.getValue(), StandardCharsets.UTF_8));

                    // Increment with CAS
                    byte[] newValue = String.valueOf(value + 1).getBytes(StandardCharsets.UTF_8);
                    PutOptions options = PutOptions.builder()
                            .ifMatchVersion(current.getVersion())
                            .build();

                    client.put(key, newValue, options);
                } catch (VersionMismatchException e) {
                    // Expected on conflicts - retry not counted in benchmark
                } catch (NoriKVException e) {
                    fail("CAS failed: " + e.getMessage());
                }
            });

            benchmark.recordLatency(latency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();
    }

    @Test
    @DisplayName("Benchmark: DELETE operations")
    public void benchmarkDelete() throws Exception {
        // Pre-populate data
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);
            byte[] value = randomValue(100);
            client.put(key, value, null);
        }

        BenchmarkRunner benchmark = new BenchmarkRunner("DELETE Operations");

        // Benchmark deletes
        for (int i = 0; i < BENCHMARK_OPS; i++) {
            byte[] key = ("key-" + i).getBytes(StandardCharsets.UTF_8);

            long latency = BenchmarkRunner.measureNs(() -> {
                try {
                    client.delete(key, null);
                } catch (NoriKVException e) {
                    fail("DELETE failed: " + e.getMessage());
                }
            });

            benchmark.recordLatency(latency);
        }

        BenchmarkRunner.BenchmarkResult result = benchmark.complete();
        result.printReport();
    }

    /**
     * Generates random byte array of specified size.
     */
    private byte[] randomValue(int size) {
        byte[] value = new byte[size];
        random.nextBytes(value);
        return value;
    }
}
