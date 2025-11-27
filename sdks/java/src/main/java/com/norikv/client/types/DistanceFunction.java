package com.norikv.client.types;

/**
 * Distance function for vector similarity search.
 */
public enum DistanceFunction {
    /**
     * Euclidean distance (L2 norm).
     */
    EUCLIDEAN("euclidean"),

    /**
     * Cosine similarity.
     */
    COSINE("cosine"),

    /**
     * Inner product (dot product).
     */
    INNER_PRODUCT("inner_product");

    private final String value;

    DistanceFunction(String value) {
        this.value = value;
    }

    /**
     * Gets the string value of this distance function.
     *
     * @return the string value
     */
    public String getValue() {
        return value;
    }

    /**
     * Creates a DistanceFunction from a string value.
     *
     * @param value the string value
     * @return the corresponding DistanceFunction
     * @throws IllegalArgumentException if the value is not recognized
     */
    public static DistanceFunction fromValue(String value) {
        for (DistanceFunction df : values()) {
            if (df.value.equals(value)) {
                return df;
            }
        }
        throw new IllegalArgumentException("Unknown distance function: " + value);
    }
}
