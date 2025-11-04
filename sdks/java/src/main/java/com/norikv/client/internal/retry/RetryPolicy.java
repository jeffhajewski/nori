package com.norikv.client.internal.retry;

import com.norikv.client.types.*;

import java.util.Random;
import java.util.concurrent.Callable;

/**
 * Implements retry logic with exponential backoff and jitter.
 *
 * <p>This class handles retrying failed operations according to the configured
 * retry policy. It supports:
 * <ul>
 * <li>Exponential backoff with configurable multiplier</li>
 * <li>Maximum delay cap</li>
 * <li>Random jitter to avoid thundering herd</li>
 * <li>Selective retry based on error type</li>
 * </ul>
 *
 * <p>Thread-safe for concurrent use.
 */
public final class RetryPolicy {
    private final RetryConfig config;
    private final Random random;

    /**
     * Creates a new RetryPolicy.
     *
     * @param config the retry configuration
     */
    public RetryPolicy(RetryConfig config) {
        if (config == null) {
            throw new IllegalArgumentException("config cannot be null");
        }
        this.config = config;
        this.random = new Random();
    }

    /**
     * Executes an operation with retry logic.
     *
     * <p>The operation will be retried according to the configured policy
     * if it throws a retryable exception. Non-retryable exceptions are
     * propagated immediately.
     *
     * @param operation the operation to execute
     * @param <T> the return type
     * @return the result of the operation
     * @throws NoriKVException if all retry attempts are exhausted or a non-retryable error occurs
     */
    public <T> T execute(Callable<T> operation) throws NoriKVException {
        int attempt = 0;
        NoriKVException lastError = null;

        while (attempt < config.getMaxAttempts()) {
            attempt++;

            try {
                return operation.call();
            } catch (NoriKVException e) {
                lastError = e;

                // Check if the error is retryable
                if (!isRetryable(e)) {
                    throw e;
                }

                // If this was the last attempt, don't sleep
                if (attempt >= config.getMaxAttempts()) {
                    break;
                }

                // Calculate delay and sleep
                int delayMs = calculateDelay(attempt);
                try {
                    Thread.sleep(delayMs);
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    throw new ConnectionException("Retry interrupted", null, ie);
                }
            } catch (Exception e) {
                // Wrap unexpected exceptions
                throw new NoriKVException("INTERNAL_ERROR", "Unexpected error during operation", e);
            }
        }

        // All retries exhausted
        throw new RetryExhaustedException(attempt, lastError);
    }

    /**
     * Executes an operation with retry logic, allowing custom retry decision.
     *
     * @param operation the operation to execute
     * @param retryDecider custom function to decide if an error is retryable
     * @param <T> the return type
     * @return the result of the operation
     * @throws NoriKVException if all retry attempts are exhausted or a non-retryable error occurs
     */
    public <T> T executeWithDecider(Callable<T> operation, RetryDecider retryDecider) throws NoriKVException {
        int attempt = 0;
        NoriKVException lastError = null;

        while (attempt < config.getMaxAttempts()) {
            attempt++;

            try {
                return operation.call();
            } catch (NoriKVException e) {
                lastError = e;

                // Check if the error is retryable using custom decider
                if (!retryDecider.shouldRetry(e, attempt)) {
                    throw e;
                }

                // If this was the last attempt, don't sleep
                if (attempt >= config.getMaxAttempts()) {
                    break;
                }

                // Calculate delay and sleep
                int delayMs = calculateDelay(attempt);
                try {
                    Thread.sleep(delayMs);
                } catch (InterruptedException ie) {
                    Thread.currentThread().interrupt();
                    throw new ConnectionException("Retry interrupted", null, ie);
                }
            } catch (Exception e) {
                // Wrap unexpected exceptions
                throw new NoriKVException("INTERNAL_ERROR", "Unexpected error during operation", e);
            }
        }

        // All retries exhausted
        throw new RetryExhaustedException(attempt, lastError);
    }

    /**
     * Determines if an error is retryable based on the configured policy.
     *
     * @param error the error to check
     * @return true if the error should be retried
     */
    public boolean isRetryable(NoriKVException error) {
        // NotLeaderException is retryable if configured
        if (error instanceof NotLeaderException) {
            return config.isRetryOnNotLeader();
        }

        // Connection errors are always retryable
        if (error instanceof ConnectionException) {
            return true;
        }

        // Unavailable errors are retryable (temporary service issues)
        if (error.getCode().equals("UNAVAILABLE")) {
            return true;
        }

        // Deadline exceeded can be retried (might succeed with more time)
        if (error.getCode().equals("DEADLINE_EXCEEDED")) {
            return true;
        }

        // These errors are NOT retryable:
        // - AlreadyExistsException: key already exists
        // - VersionMismatchException: CAS failed
        // - KeyNotFoundException: key doesn't exist
        // - Invalid arguments
        return false;
    }

    /**
     * Calculates the delay before the next retry attempt.
     *
     * <p>Uses exponential backoff with jitter:
     * delay = min(initialDelay * (multiplier ^ (attempt-1)), maxDelay) + random jitter
     *
     * @param attempt the current attempt number (1-based)
     * @return the delay in milliseconds
     */
    int calculateDelay(int attempt) {
        // Exponential backoff: initialDelay * (multiplier ^ (attempt-1))
        double baseDelay = config.getInitialDelayMs() * Math.pow(config.getBackoffMultiplier(), attempt - 1);

        // Cap at max delay
        int cappedDelay = (int) Math.min(baseDelay, config.getMaxDelayMs());

        // Add random jitter: [0, jitterMs]
        int jitter = config.getJitterMs() > 0 ? random.nextInt(config.getJitterMs() + 1) : 0;

        return cappedDelay + jitter;
    }

    /**
     * Gets the retry configuration.
     *
     * @return the retry configuration
     */
    public RetryConfig getConfig() {
        return config;
    }

    /**
     * Functional interface for custom retry decisions.
     */
    @FunctionalInterface
    public interface RetryDecider {
        /**
         * Determines if an operation should be retried.
         *
         * @param error the error that occurred
         * @param attempt the current attempt number (1-based)
         * @return true if the operation should be retried
         */
        boolean shouldRetry(NoriKVException error, int attempt);
    }
}
