package com.norikv.client.types;

/**
 * Options for Delete operations.
 */
public final class DeleteOptions {
    private final Version ifMatchVersion;
    private final String idempotencyKey;

    private DeleteOptions(Builder builder) {
        this.ifMatchVersion = builder.ifMatchVersion;
        this.idempotencyKey = builder.idempotencyKey;
    }

    /**
     * Gets the expected version for conditional delete (CAS).
     *
     * @return the expected version, or null for unconditional delete
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
     * Builder for DeleteOptions.
     */
    public static final class Builder {
        private Version ifMatchVersion;
        private String idempotencyKey;

        private Builder() {
        }

        /**
         * Sets the expected version for conditional delete (CAS).
         * Only deletes if the current version matches.
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
         *
         * @param key the idempotency key
         * @return this builder
         */
        public Builder idempotencyKey(String key) {
            this.idempotencyKey = key;
            return this;
        }

        /**
         * Builds the DeleteOptions.
         *
         * @return the DeleteOptions
         */
        public DeleteOptions build() {
            return new DeleteOptions(this);
        }
    }
}
