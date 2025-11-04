package com.norikv.client.types;

/**
 * Options for Put operations.
 */
public final class PutOptions {
    private final Long ttlMs;
    private final boolean ifNotExists;
    private final Version ifMatchVersion;
    private final String idempotencyKey;

    private PutOptions(Builder builder) {
        this.ttlMs = builder.ttlMs;
        this.ifNotExists = builder.ifNotExists;
        this.ifMatchVersion = builder.ifMatchVersion;
        this.idempotencyKey = builder.idempotencyKey;
    }

    /**
     * Gets the TTL in milliseconds.
     *
     * @return TTL in milliseconds, or null for no expiration
     */
    public Long getTtlMs() {
        return ttlMs;
    }

    /**
     * Returns true if this is an IfNotExists operation.
     *
     * @return true if only writing when key doesn't exist
     */
    public boolean isIfNotExists() {
        return ifNotExists;
    }

    /**
     * Gets the expected version for conditional update (CAS).
     *
     * @return the expected version, or null for unconditional write
     */
    public Version getIfMatchVersion() {
        return ifMatchVersion;
    }

    /**
     * Gets the idempotency key for safe retries.
     *
     * @return the idempotency key, or null
     */
    public String getIdempotencyKey() {
        return idempotencyKey;
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
     * Builder for PutOptions.
     */
    public static final class Builder {
        private Long ttlMs;
        private boolean ifNotExists;
        private Version ifMatchVersion;
        private String idempotencyKey;

        private Builder() {
        }

        /**
         * Sets the TTL in milliseconds.
         *
         * @param ttlMs TTL in milliseconds (0 or null means no expiration)
         * @return this builder
         */
        public Builder ttlMs(Long ttlMs) {
            this.ttlMs = ttlMs;
            return this;
        }

        /**
         * Sets IfNotExists flag to only write if key doesn't exist.
         * Returns AlreadyExistsException if key exists.
         *
         * @param ifNotExists true to enable IfNotExists
         * @return this builder
         */
        public Builder ifNotExists(boolean ifNotExists) {
            this.ifNotExists = ifNotExists;
            return this;
        }

        /**
         * Sets the expected version for conditional update (CAS).
         * Only writes if the current version matches.
         *
         * @param version the expected version
         * @return this builder
         */
        public Builder ifMatchVersion(Version version) {
            this.ifMatchVersion = version;
            return this;
        }

        /**
         * Sets the idempotency key for safe retries.
         * Same key + idempotency_key always produces the same result.
         *
         * @param key the idempotency key
         * @return this builder
         */
        public Builder idempotencyKey(String key) {
            this.idempotencyKey = key;
            return this;
        }

        /**
         * Builds the PutOptions.
         *
         * @return the PutOptions
         */
        public PutOptions build() {
            return new PutOptions(this);
        }
    }
}
