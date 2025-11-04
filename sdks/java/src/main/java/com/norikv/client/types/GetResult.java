package com.norikv.client.types;

import java.util.Collections;
import java.util.Map;

/**
 * Result of a Get operation.
 */
public final class GetResult {
    private final byte[] value;
    private final Version version;
    private final Map<String, String> metadata;

    /**
     * Creates a new GetResult.
     *
     * @param value the retrieved value, or null if key doesn't exist
     * @param version the version of the value, or null if key doesn't exist
     * @param metadata metadata from the server (optional)
     */
    public GetResult(byte[] value, Version version, Map<String, String> metadata) {
        this.value = value;
        this.version = version;
        this.metadata = metadata != null ? Collections.unmodifiableMap(metadata) : Collections.emptyMap();
    }

    /**
     * Creates a new GetResult without metadata.
     *
     * @param value the retrieved value, or null if key doesn't exist
     * @param version the version of the value, or null if key doesn't exist
     */
    public GetResult(byte[] value, Version version) {
        this(value, version, null);
    }

    /**
     * Gets the retrieved value.
     *
     * @return the value, or null if key doesn't exist
     */
    public byte[] getValue() {
        return value;
    }

    /**
     * Gets the version of the value.
     *
     * @return the version, or null if key doesn't exist
     */
    public Version getVersion() {
        return version;
    }

    /**
     * Gets the metadata from the server.
     *
     * @return the metadata (never null, but may be empty)
     */
    public Map<String, String> getMetadata() {
        return metadata;
    }

    /**
     * Returns true if the key exists.
     *
     * @return true if the key exists
     */
    public boolean exists() {
        return value != null;
    }
}
