package com.norikv.client.types;

/**
 * Indicates maximum retry attempts have been exhausted.
 */
public class RetryExhaustedException extends NoriKVException {
    private final int attempts;
    private final NoriKVException lastError;

    /**
     * Creates a new RetryExhaustedException.
     *
     * @param attempts the number of attempts made
     * @param lastError the last error encountered
     */
    public RetryExhaustedException(int attempts, NoriKVException lastError) {
        super("RETRY_EXHAUSTED",
              String.format("Retry exhausted after %d attempts: %s", attempts,
                          lastError != null ? lastError.getMessage() : "unknown error"),
              lastError);
        this.attempts = attempts;
        this.lastError = lastError;
    }

    /**
     * Gets the number of attempts made.
     *
     * @return the number of attempts
     */
    public int getAttempts() {
        return attempts;
    }

    /**
     * Gets the last error encountered.
     *
     * @return the last error
     */
    public NoriKVException getLastError() {
        return lastError;
    }
}
