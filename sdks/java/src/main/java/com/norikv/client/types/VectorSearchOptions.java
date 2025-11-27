package com.norikv.client.types;

/**
 * Options for vector similarity search.
 */
public final class VectorSearchOptions {
    private final boolean includeVectors;

    private VectorSearchOptions(Builder builder) {
        this.includeVectors = builder.includeVectors;
    }

    /**
     * Whether to include vector data in results.
     *
     * @return true if vectors should be included
     */
    public boolean isIncludeVectors() {
        return includeVectors;
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
     * Builder for VectorSearchOptions.
     */
    public static final class Builder {
        private boolean includeVectors = false;

        private Builder() {}

        /**
         * Sets whether to include full vector data in results.
         *
         * @param includeVectors true to include vectors
         * @return this builder
         */
        public Builder includeVectors(boolean includeVectors) {
            this.includeVectors = includeVectors;
            return this;
        }

        /**
         * Builds the options.
         *
         * @return the options
         */
        public VectorSearchOptions build() {
            return new VectorSearchOptions(this);
        }
    }
}
