package com.norikv.client.types;

import java.util.Objects;

/**
 * Identifies a specific version of a key-value pair.
 * Corresponds to the Raft log entry where it was committed.
 */
public final class Version {
    private final long term;
    private final long index;

    /**
     * Creates a new Version.
     *
     * @param term Raft term when this version was written
     * @param index Raft log index
     */
    public Version(long term, long index) {
        this.term = term;
        this.index = index;
    }

    /**
     * Gets the Raft term.
     *
     * @return the term
     */
    public long getTerm() {
        return term;
    }

    /**
     * Gets the Raft log index.
     *
     * @return the index
     */
    public long getIndex() {
        return index;
    }

    @Override
    public boolean equals(Object o) {
        if (this == o) return true;
        if (o == null || getClass() != o.getClass()) return false;
        Version version = (Version) o;
        return term == version.term && index == version.index;
    }

    @Override
    public int hashCode() {
        return Objects.hash(term, index);
    }

    @Override
    public String toString() {
        return "Version{term=" + term + ", index=" + index + "}";
    }
}
