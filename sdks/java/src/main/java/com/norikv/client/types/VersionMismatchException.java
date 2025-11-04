package com.norikv.client.types;

/**
 * Indicates version mismatch (for IfMatchVersion operations).
 */
public class VersionMismatchException extends NoriKVException {
    private final byte[] key;
    private final Version expectedVersion;
    private final Version actualVersion;

    /**
     * Creates a new VersionMismatchException.
     *
     * @param message the error message
     * @param key the key being accessed
     * @param expectedVersion the expected version
     * @param actualVersion the actual current version
     */
    public VersionMismatchException(String message, byte[] key, Version expectedVersion, Version actualVersion) {
        super("VERSION_MISMATCH", message);
        this.key = key;
        this.expectedVersion = expectedVersion;
        this.actualVersion = actualVersion;
    }

    /**
     * Creates a new VersionMismatchException with a cause.
     *
     * @param message the error message
     * @param key the key being accessed
     * @param expectedVersion the expected version
     * @param actualVersion the actual current version
     * @param cause the underlying cause
     */
    public VersionMismatchException(String message, byte[] key, Version expectedVersion, Version actualVersion, Throwable cause) {
        super("VERSION_MISMATCH", message, cause);
        this.key = key;
        this.expectedVersion = expectedVersion;
        this.actualVersion = actualVersion;
    }

    /**
     * Gets the key being accessed.
     *
     * @return the key
     */
    public byte[] getKey() {
        return key;
    }

    /**
     * Gets the expected version.
     *
     * @return the expected version
     */
    public Version getExpectedVersion() {
        return expectedVersion;
    }

    /**
     * Gets the actual current version.
     *
     * @return the actual version
     */
    public Version getActualVersion() {
        return actualVersion;
    }
}
