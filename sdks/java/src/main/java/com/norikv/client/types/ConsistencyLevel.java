package com.norikv.client.types;

/**
 * Specifies the consistency guarantees for read operations.
 */
public enum ConsistencyLevel {
    /**
     * Uses lease-based linearizable reads (fast, default).
     * The leader serves reads from its lease without quorum confirmation.
     */
    LEASE("lease"),

    /**
     * Uses strict linearizable reads with read-index protocol.
     * Requires quorum confirmation for guaranteed up-to-date reads.
     */
    LINEARIZABLE("linearizable"),

    /**
     * Allows stale reads from any replica (fastest).
     * May return stale data but provides highest throughput.
     */
    STALE_OK("stale_ok");

    private final String value;

    ConsistencyLevel(String value) {
        this.value = value;
    }

    /**
     * Gets the string value for this consistency level.
     *
     * @return the string value
     */
    public String getValue() {
        return value;
    }

    /**
     * Parses a consistency level from a string.
     *
     * @param value the string value
     * @return the consistency level
     * @throws IllegalArgumentException if the value is invalid
     */
    public static ConsistencyLevel fromString(String value) {
        for (ConsistencyLevel level : values()) {
            if (level.value.equals(value)) {
                return level;
            }
        }
        throw new IllegalArgumentException("Invalid consistency level: " + value);
    }
}
