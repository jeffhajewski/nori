package com.norikv.client.types;

/**
 * Indicates the requested key was not found.
 */
public class KeyNotFoundException extends NoriKVException {
    private final byte[] key;

    /**
     * Creates a new KeyNotFoundException.
     *
     * @param message the error message
     * @param key the key that was not found
     */
    public KeyNotFoundException(String message, byte[] key) {
        super("KEY_NOT_FOUND", message);
        this.key = key;
    }

    /**
     * Creates a new KeyNotFoundException with a cause.
     *
     * @param message the error message
     * @param key the key that was not found
     * @param cause the underlying cause
     */
    public KeyNotFoundException(String message, byte[] key, Throwable cause) {
        super("KEY_NOT_FOUND", message, cause);
        this.key = key;
    }

    /**
     * Gets the key that was not found.
     *
     * @return the key
     */
    public byte[] getKey() {
        return key;
    }
}
