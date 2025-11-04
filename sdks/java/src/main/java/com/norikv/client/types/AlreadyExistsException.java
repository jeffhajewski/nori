package com.norikv.client.types;

/**
 * Indicates the key already exists (for IfNotExists operations).
 */
public class AlreadyExistsException extends NoriKVException {
    private final byte[] key;

    /**
     * Creates a new AlreadyExistsException.
     *
     * @param message the error message
     * @param key the key that already exists
     */
    public AlreadyExistsException(String message, byte[] key) {
        super("ALREADY_EXISTS", message);
        this.key = key;
    }

    /**
     * Creates a new AlreadyExistsException with a cause.
     *
     * @param message the error message
     * @param key the key that already exists
     * @param cause the underlying cause
     */
    public AlreadyExistsException(String message, byte[] key, Throwable cause) {
        super("ALREADY_EXISTS", message, cause);
        this.key = key;
    }

    /**
     * Gets the key that already exists.
     *
     * @return the key
     */
    public byte[] getKey() {
        return key;
    }
}
