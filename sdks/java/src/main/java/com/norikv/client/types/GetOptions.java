package com.norikv.client.types;

/**
 * Options for Get operations.
 */
public final class GetOptions {
    private final ConsistencyLevel consistency;

    private GetOptions(Builder builder) {
        this.consistency = builder.consistency != null ? builder.consistency : ConsistencyLevel.LEASE;
    }

    /**
     * Gets the consistency level for this read.
     *
     * @return the consistency level (default: LEASE)
     */
    public ConsistencyLevel getConsistency() {
        return consistency;
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
     * Builder for GetOptions.
     */
    public static final class Builder {
        private ConsistencyLevel consistency;

        private Builder() {
        }

        /**
         * Sets the consistency level for this read.
         *
         * @param consistency the consistency level (default: LEASE)
         * @return this builder
         */
        public Builder consistency(ConsistencyLevel consistency) {
            this.consistency = consistency;
            return this;
        }

        /**
         * Builds the GetOptions.
         *
         * @return the GetOptions
         */
        public GetOptions build() {
            return new GetOptions(this);
        }
    }
}
