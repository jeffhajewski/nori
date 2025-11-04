package com.norikv.client.types;

/**
 * Retry policy configuration.
 */
public final class RetryConfig {
    private final int maxAttempts;
    private final int initialDelayMs;
    private final int maxDelayMs;
    private final double backoffMultiplier;
    private final int jitterMs;
    private final boolean retryOnNotLeader;

    private RetryConfig(Builder builder) {
        this.maxAttempts = builder.maxAttempts;
        this.initialDelayMs = builder.initialDelayMs;
        this.maxDelayMs = builder.maxDelayMs;
        this.backoffMultiplier = builder.backoffMultiplier;
        this.jitterMs = builder.jitterMs;
        this.retryOnNotLeader = builder.retryOnNotLeader;
    }

    public int getMaxAttempts() {
        return maxAttempts;
    }

    public int getInitialDelayMs() {
        return initialDelayMs;
    }

    public int getMaxDelayMs() {
        return maxDelayMs;
    }

    public double getBackoffMultiplier() {
        return backoffMultiplier;
    }

    public int getJitterMs() {
        return jitterMs;
    }

    public boolean isRetryOnNotLeader() {
        return retryOnNotLeader;
    }

    /**
     * Returns the default retry configuration.
     *
     * @return default retry configuration
     */
    public static RetryConfig defaultConfig() {
        return builder().build();
    }

    /**
     * Creates a new builder.
     *
     * @return a new builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Builder for RetryConfig.
     */
    public static final class Builder {
        private int maxAttempts = 3;
        private int initialDelayMs = 10;
        private int maxDelayMs = 1000;
        private double backoffMultiplier = 2.0;
        private int jitterMs = 100;
        private boolean retryOnNotLeader = true;

        private Builder() {
        }

        /**
         * Sets the maximum number of retry attempts (default: 3).
         *
         * @param maxAttempts maximum retry attempts
         * @return this builder
         */
        public Builder maxAttempts(int maxAttempts) {
            this.maxAttempts = maxAttempts;
            return this;
        }

        /**
         * Sets the initial delay in milliseconds (default: 10).
         *
         * @param initialDelayMs initial delay in milliseconds
         * @return this builder
         */
        public Builder initialDelayMs(int initialDelayMs) {
            this.initialDelayMs = initialDelayMs;
            return this;
        }

        /**
         * Sets the maximum delay in milliseconds (default: 1000).
         *
         * @param maxDelayMs maximum delay in milliseconds
         * @return this builder
         */
        public Builder maxDelayMs(int maxDelayMs) {
            this.maxDelayMs = maxDelayMs;
            return this;
        }

        /**
         * Sets the backoff multiplier (default: 2.0).
         *
         * @param backoffMultiplier backoff multiplier
         * @return this builder
         */
        public Builder backoffMultiplier(double backoffMultiplier) {
            this.backoffMultiplier = backoffMultiplier;
            return this;
        }

        /**
         * Sets the jitter in milliseconds (default: 100).
         *
         * @param jitterMs jitter in milliseconds
         * @return this builder
         */
        public Builder jitterMs(int jitterMs) {
            this.jitterMs = jitterMs;
            return this;
        }

        /**
         * Enables or disables retry on NOT_LEADER errors (default: true).
         *
         * @param retryOnNotLeader true to enable retry on NOT_LEADER
         * @return this builder
         */
        public Builder retryOnNotLeader(boolean retryOnNotLeader) {
            this.retryOnNotLeader = retryOnNotLeader;
            return this;
        }

        /**
         * Builds the RetryConfig.
         *
         * @return the RetryConfig
         * @throws IllegalArgumentException if configuration is invalid
         */
        public RetryConfig build() {
            if (maxAttempts <= 0) {
                throw new IllegalArgumentException("maxAttempts must be > 0");
            }
            if (initialDelayMs < 0) {
                throw new IllegalArgumentException("initialDelayMs must be >= 0");
            }
            if (maxDelayMs < initialDelayMs) {
                throw new IllegalArgumentException("maxDelayMs must be >= initialDelayMs");
            }
            if (backoffMultiplier < 1.0) {
                throw new IllegalArgumentException("backoffMultiplier must be >= 1.0");
            }
            if (jitterMs < 0) {
                throw new IllegalArgumentException("jitterMs must be >= 0");
            }
            return new RetryConfig(this);
        }
    }
}
