package com.norikv.client.types;

import java.util.Arrays;
import java.util.List;

/**
 * Configuration for the NoriKV client.
 */
public final class ClientConfig {
    private final List<String> nodes;
    private final int totalShards;
    private final int timeoutMs;
    private final RetryConfig retry;
    private final boolean watchCluster;
    private final int maxConnectionsPerNode;

    private ClientConfig(Builder builder) {
        this.nodes = builder.nodes;
        this.totalShards = builder.totalShards;
        this.timeoutMs = builder.timeoutMs;
        this.retry = builder.retry != null ? builder.retry : RetryConfig.defaultConfig();
        this.watchCluster = builder.watchCluster;
        this.maxConnectionsPerNode = builder.maxConnectionsPerNode;
    }

    public List<String> getNodes() {
        return nodes;
    }

    public int getTotalShards() {
        return totalShards;
    }

    public int getTimeoutMs() {
        return timeoutMs;
    }

    public RetryConfig getRetry() {
        return retry;
    }

    public boolean isWatchCluster() {
        return watchCluster;
    }

    public int getMaxConnectionsPerNode() {
        return maxConnectionsPerNode;
    }

    /**
     * Creates a default configuration with the given nodes.
     *
     * @param nodes list of node addresses to connect to
     * @return default configuration
     */
    public static ClientConfig defaultConfig(String... nodes) {
        return builder()
                .nodes(Arrays.asList(nodes))
                .build();
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
     * Builder for ClientConfig.
     */
    public static final class Builder {
        private List<String> nodes;
        private int totalShards = 1024;
        private int timeoutMs = 5000;
        private RetryConfig retry;
        private boolean watchCluster = true;
        private int maxConnectionsPerNode = 10;

        private Builder() {
        }

        /**
         * Sets the list of node addresses to connect to (at least one required).
         *
         * @param nodes list of node addresses
         * @return this builder
         */
        public Builder nodes(List<String> nodes) {
            this.nodes = nodes;
            return this;
        }

        /**
         * Sets the total number of virtual shards (default: 1024).
         *
         * @param totalShards total number of shards
         * @return this builder
         */
        public Builder totalShards(int totalShards) {
            this.totalShards = totalShards;
            return this;
        }

        /**
         * Sets the default request timeout in milliseconds (default: 5000).
         *
         * @param timeoutMs timeout in milliseconds
         * @return this builder
         */
        public Builder timeoutMs(int timeoutMs) {
            this.timeoutMs = timeoutMs;
            return this;
        }

        /**
         * Sets the retry policy configuration.
         *
         * @param retry retry configuration
         * @return this builder
         */
        public Builder retry(RetryConfig retry) {
            this.retry = retry;
            return this;
        }

        /**
         * Enables or disables cluster view watching (default: true).
         *
         * @param watchCluster true to enable cluster watching
         * @return this builder
         */
        public Builder watchCluster(boolean watchCluster) {
            this.watchCluster = watchCluster;
            return this;
        }

        /**
         * Sets the maximum gRPC connections per node (default: 10).
         *
         * @param maxConnectionsPerNode maximum connections per node
         * @return this builder
         */
        public Builder maxConnectionsPerNode(int maxConnectionsPerNode) {
            this.maxConnectionsPerNode = maxConnectionsPerNode;
            return this;
        }

        /**
         * Builds the ClientConfig.
         *
         * @return the ClientConfig
         * @throws IllegalArgumentException if configuration is invalid
         */
        public ClientConfig build() {
            if (nodes == null || nodes.isEmpty()) {
                throw new IllegalArgumentException("At least one node is required");
            }
            if (totalShards <= 0) {
                throw new IllegalArgumentException("totalShards must be > 0");
            }
            if (timeoutMs <= 0) {
                throw new IllegalArgumentException("timeoutMs must be > 0");
            }
            if (maxConnectionsPerNode <= 0) {
                throw new IllegalArgumentException("maxConnectionsPerNode must be > 0");
            }
            return new ClientConfig(this);
        }
    }
}
