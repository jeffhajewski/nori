package com.norikv.client.types;

import java.util.List;
import java.util.Objects;

/**
 * A single match result from a vector search.
 */
public final class VectorMatch {
    private final String id;
    private final float distance;
    private final List<Float> vector;

    /**
     * Creates a new VectorMatch.
     *
     * @param id the ID of the matching vector
     * @param distance the distance from the query vector
     * @param vector the vector data (may be null if not requested)
     */
    public VectorMatch(String id, float distance, List<Float> vector) {
        this.id = Objects.requireNonNull(id, "id cannot be null");
        this.distance = distance;
        this.vector = vector;
    }

    /**
     * Gets the ID of the matching vector.
     *
     * @return the vector ID
     */
    public String getId() {
        return id;
    }

    /**
     * Gets the distance from the query vector.
     *
     * @return the distance
     */
    public float getDistance() {
        return distance;
    }

    /**
     * Gets the vector data, if it was requested.
     *
     * @return the vector data, or null if not requested
     */
    public List<Float> getVector() {
        return vector;
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        VectorMatch that = (VectorMatch) o;
        return Float.compare(that.distance, distance) == 0 &&
                Objects.equals(id, that.id) &&
                Objects.equals(vector, that.vector);
    }

    @Override
    public int hashCode() {
        return Objects.hash(id, distance, vector);
    }

    @Override
    public String toString() {
        return "VectorMatch{" +
                "id='" + id + '\'' +
                ", distance=" + distance +
                ", vector=" + (vector != null ? "[" + vector.size() + " dims]" : "null") +
                '}';
    }
}
