package com.norikv.client.internal.retry;

import com.norikv.client.types.*;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.DisplayName;
import static org.junit.jupiter.api.Assertions.*;

import java.util.concurrent.Callable;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Test suite for RetryPolicy.
 */
public class RetryPolicyTest {

    @Test
    @DisplayName("RetryPolicy: succeeds on first attempt")
    public void testSuccessOnFirstAttempt() throws NoriKVException {
        RetryConfig config = RetryConfig.defaultConfig();
        RetryPolicy policy = new RetryPolicy(config);

        String result = policy.execute(() -> "success");
        assertEquals("success", result);
    }

    @Test
    @DisplayName("RetryPolicy: succeeds after retries")
    public void testSuccessAfterRetries() throws NoriKVException {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            int attempt = attempts.incrementAndGet();
            if (attempt < 3) {
                throw new ConnectionException("Connection failed", "localhost:9001");
            }
            return "success";
        };

        String result = policy.execute(operation);
        assertEquals("success", result);
        assertEquals(3, attempts.get(), "Should have made 3 attempts");
    }

    @Test
    @DisplayName("RetryPolicy: throws RetryExhaustedException after max attempts")
    public void testRetryExhausted() {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            attempts.incrementAndGet();
            throw new ConnectionException("Connection failed", "localhost:9001");
        };

        assertThrows(RetryExhaustedException.class, () -> policy.execute(operation));
        assertEquals(3, attempts.get(), "Should have made 3 attempts");
    }

    @Test
    @DisplayName("RetryPolicy: retries NotLeaderException when configured")
    public void testRetryNotLeader() throws NoriKVException {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1)
                .retryOnNotLeader(true)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            int attempt = attempts.incrementAndGet();
            if (attempt < 3) {
                throw new NotLeaderException("Not leader", "localhost:9002", 0);
            }
            return "success";
        };

        String result = policy.execute(operation);
        assertEquals("success", result);
        assertEquals(3, attempts.get(), "Should have made 3 attempts");
    }

    @Test
    @DisplayName("RetryPolicy: does not retry NotLeaderException when disabled")
    public void testNoRetryNotLeaderWhenDisabled() {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1)
                .retryOnNotLeader(false)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            attempts.incrementAndGet();
            throw new NotLeaderException("Not leader", "localhost:9002", 0);
        };

        assertThrows(NotLeaderException.class, () -> policy.execute(operation));
        assertEquals(1, attempts.get(), "Should have made only 1 attempt");
    }

    @Test
    @DisplayName("RetryPolicy: does not retry non-retryable errors")
    public void testNoRetryNonRetryableErrors() {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        // AlreadyExistsException should not be retried
        AtomicInteger attemptsExists = new AtomicInteger(0);
        Callable<String> operationExists = () -> {
            attemptsExists.incrementAndGet();
            throw new AlreadyExistsException("Key exists", "key".getBytes());
        };
        assertThrows(AlreadyExistsException.class, () -> policy.execute(operationExists));
        assertEquals(1, attemptsExists.get(), "Should not retry AlreadyExistsException");

        // VersionMismatchException should not be retried
        AtomicInteger attemptsVersion = new AtomicInteger(0);
        Callable<String> operationVersion = () -> {
            attemptsVersion.incrementAndGet();
            throw new VersionMismatchException("Version mismatch", "key".getBytes(),
                    new Version(1, 1), new Version(2, 2));
        };
        assertThrows(VersionMismatchException.class, () -> policy.execute(operationVersion));
        assertEquals(1, attemptsVersion.get(), "Should not retry VersionMismatchException");

        // KeyNotFoundException should not be retried
        AtomicInteger attemptsKey = new AtomicInteger(0);
        Callable<String> operationKey = () -> {
            attemptsKey.incrementAndGet();
            throw new KeyNotFoundException("Key not found", "key".getBytes());
        };
        assertThrows(KeyNotFoundException.class, () -> policy.execute(operationKey));
        assertEquals(1, attemptsKey.get(), "Should not retry KeyNotFoundException");
    }

    @Test
    @DisplayName("RetryPolicy: exponential backoff calculation")
    public void testExponentialBackoff() {
        RetryConfig config = RetryConfig.builder()
                .initialDelayMs(10)
                .maxDelayMs(1000)
                .backoffMultiplier(2.0)
                .jitterMs(0) // No jitter for predictable testing
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        // Attempt 1: 10 * 2^0 = 10ms
        int delay1 = policy.calculateDelay(1);
        assertEquals(10, delay1);

        // Attempt 2: 10 * 2^1 = 20ms
        int delay2 = policy.calculateDelay(2);
        assertEquals(20, delay2);

        // Attempt 3: 10 * 2^2 = 40ms
        int delay3 = policy.calculateDelay(3);
        assertEquals(40, delay3);

        // Attempt 4: 10 * 2^3 = 80ms
        int delay4 = policy.calculateDelay(4);
        assertEquals(80, delay4);
    }

    @Test
    @DisplayName("RetryPolicy: caps delay at maxDelayMs")
    public void testMaxDelayCap() {
        RetryConfig config = RetryConfig.builder()
                .initialDelayMs(100)
                .maxDelayMs(200)
                .backoffMultiplier(2.0)
                .jitterMs(0)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        // Attempt 1: 100ms
        int delay1 = policy.calculateDelay(1);
        assertEquals(100, delay1);

        // Attempt 2: 200ms
        int delay2 = policy.calculateDelay(2);
        assertEquals(200, delay2);

        // Attempt 3: would be 400ms, but capped at 200ms
        int delay3 = policy.calculateDelay(3);
        assertEquals(200, delay3);

        // Attempt 4: would be 800ms, but capped at 200ms
        int delay4 = policy.calculateDelay(4);
        assertEquals(200, delay4);
    }

    @Test
    @DisplayName("RetryPolicy: adds jitter to delay")
    public void testJitter() {
        RetryConfig config = RetryConfig.builder()
                .initialDelayMs(10)
                .maxDelayMs(1000)
                .backoffMultiplier(2.0)
                .jitterMs(100)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        // Jitter adds [0, 100]ms, so delay should be in range [10, 110]
        int delay = policy.calculateDelay(1);
        assertTrue(delay >= 10 && delay <= 110,
                "Delay " + delay + " should be in range [10, 110]");
    }

    @Test
    @DisplayName("RetryPolicy: custom retry decider")
    public void testCustomRetryDecider() throws NoriKVException {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(5)
                .initialDelayMs(1)
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            int attempt = attempts.incrementAndGet();
            if (attempt < 4) {
                throw new ConnectionException("Connection failed", "localhost:9001");
            }
            return "success";
        };

        // Custom decider: retry only on attempts 1-3
        RetryPolicy.RetryDecider decider = (error, attempt) -> attempt < 4;

        String result = policy.executeWithDecider(operation, decider);
        assertEquals("success", result);
        assertEquals(4, attempts.get(), "Should have made 4 attempts");
    }

    @Test
    @DisplayName("RetryPolicy: isRetryable correctly identifies retryable errors")
    public void testIsRetryable() {
        RetryConfig config = RetryConfig.defaultConfig();
        RetryPolicy policy = new RetryPolicy(config);

        // Retryable errors
        assertTrue(policy.isRetryable(
                new NotLeaderException("Not leader", "hint", 0)));
        assertTrue(policy.isRetryable(
                new ConnectionException("Connection failed", "addr")));
        assertTrue(policy.isRetryable(
                new NoriKVException("UNAVAILABLE", "Unavailable")));
        assertTrue(policy.isRetryable(
                new NoriKVException("DEADLINE_EXCEEDED", "Timeout")));

        // Non-retryable errors
        assertFalse(policy.isRetryable(
                new AlreadyExistsException("Exists", "key".getBytes())));
        assertFalse(policy.isRetryable(
                new VersionMismatchException("Mismatch", "key".getBytes(), null, null)));
        assertFalse(policy.isRetryable(
                new KeyNotFoundException("Not found", "key".getBytes())));
        assertFalse(policy.isRetryable(
                new NoriKVException("INVALID_ARGUMENT", "Invalid")));
    }

    @Test
    @DisplayName("RetryPolicy: handles interrupted exception")
    public void testInterrupted() {
        RetryConfig config = RetryConfig.builder()
                .maxAttempts(3)
                .initialDelayMs(1000) // Long delay
                .build();
        RetryPolicy policy = new RetryPolicy(config);

        AtomicInteger attempts = new AtomicInteger(0);
        Callable<String> operation = () -> {
            attempts.incrementAndGet();
            throw new ConnectionException("Connection failed", "localhost:9001");
        };

        // Interrupt the thread after a short delay
        Thread testThread = Thread.currentThread();
        Thread interrupter = new Thread(() -> {
            try {
                Thread.sleep(50);
                testThread.interrupt();
            } catch (InterruptedException e) {
                // Ignore
            }
        });
        interrupter.start();

        assertThrows(ConnectionException.class, () -> policy.execute(operation));
        assertTrue(Thread.interrupted(), "Thread should be interrupted");
    }
}
