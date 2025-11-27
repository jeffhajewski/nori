package com.norikv.client.types;

import java.util.Collections;
import java.util.List;
import java.util.Objects;

/**
 * Result of a vector similarity search.
 */
public final class VectorSearchResult {
    private final List<VectorMatch> matches;
    private final long searchTimeUs;

    /**
     * Creates a new VectorSearchResult.
     *
     * @param matches the list of matching vectors, sorted by distance
     * @param searchTimeUs the search time in microseconds
     */
    public VectorSearchResult(List<VectorMatch> matches, long searchTimeUs) {
        this.matches = matches != null ? Collections.unmodifiableList(matches) : Collections.emptyList();
        this.searchTimeUs = searchTimeUs;
    }

    /**
     * Gets the matching vectors, sorted by distance (closest first).
     *
     * @return immutable list of matches
     */
    public List<VectorMatch> getMatches() {
        return matches;
    }

    /**
     * Gets the search time in microseconds.
     *
     * @return search time in microseconds
     */
    public long getSearchTimeUs() {
        return searchTimeUs;
    }

    /**
     * Returns the number of matches.
     *
     * @return number of matches
     */
    public int size() {
        return matches.size();
    }

    /**
     * Returns true if there are no matches.
     *
     * @return true if empty
     */
    public boolean isEmpty() {
        return matches.isEmpty();
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        VectorSearchResult that = (VectorSearchResult) o;
        return searchTimeUs == that.searchTimeUs &&
                Objects.equals(matches, that.matches);
    }

    @Override
    public int hashCode() {
        return Objects.hash(matches, searchTimeUs);
    }

    @Override
    public String toString() {
        return "VectorSearchResult{" +
                "matches=" + matches.size() +
                ", searchTimeUs=" + searchTimeUs +
                '}';
    }
}
