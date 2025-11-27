package com.norikv.client.types;

/**
 * Type of vector index algorithm.
 */
public enum VectorIndexType {
    /**
     * Brute force linear scan. Exact results, O(n) complexity.
     */
    BRUTE_FORCE("brute_force"),

    /**
     * Hierarchical Navigable Small World graph. Approximate results, O(log n) complexity.
     */
    HNSW("hnsw");

    private final String value;

    VectorIndexType(String value) {
        this.value = value;
    }

    /**
     * Gets the string value of this index type.
     *
     * @return the string value
     */
    public String getValue() {
        return value;
    }

    /**
     * Creates a VectorIndexType from a string value.
     *
     * @param value the string value
     * @return the corresponding VectorIndexType
     * @throws IllegalArgumentException if the value is not recognized
     */
    public static VectorIndexType fromValue(String value) {
        for (VectorIndexType vit : values()) {
            if (vit.value.equals(value)) {
                return vit;
            }
        }
        throw new IllegalArgumentException("Unknown vector index type: " + value);
    }
}
