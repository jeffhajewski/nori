package com.norikv.client.types;

/**
 * Options for dropping a vector index.
 */
public final class DropVectorIndexOptions {
    private final String idempotencyKey;

    private DropVectorIndexOptions(Builder builder) {
        this.idempotencyKey = builder.idempotencyKey;
    }

    /**
     * Gets the idempotency key.
     *
     * @return the idempotency key, or null
     */
    public String getIdempotencyKey() {
        return idempotencyKey;
    }

    /**
     * Creates a new builder.
     *
     * @return the builder
     */
    public static Builder builder() {
        return new Builder();
    }

    /**
     * Builder for DropVectorIndexOptions.
     */
    public static final class Builder {
        private String idempotencyKey;

        private Builder() {}

        /**
         * Sets the idempotency key for safe retries.
         *
         * @param idempotencyKey the idempotency key
         * @return this builder
         */
        public Builder idempotencyKey(String idempotencyKey) {
            this.idempotencyKey = idempotencyKey;
            return this;
        }

        /**
         * Builds the options.
         *
         * @return the options
         */
        public DropVectorIndexOptions build() {
            return new DropVectorIndexOptions(this);
        }
    }
}
