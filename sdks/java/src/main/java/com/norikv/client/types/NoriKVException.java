package com.norikv.client.types;

/**
 * Base exception for all NoriKV errors.
 */
public class NoriKVException extends Exception {
    private final String code;

    /**
     * Creates a new NoriKVException.
     *
     * @param code the error code
     * @param message the error message
     */
    public NoriKVException(String code, String message) {
        super(message);
        this.code = code;
    }

    /**
     * Creates a new NoriKVException with a cause.
     *
     * @param code the error code
     * @param message the error message
     * @param cause the underlying cause
     */
    public NoriKVException(String code, String message, Throwable cause) {
        super(message, cause);
        this.code = code;
    }

    /**
     * Gets the error code.
     *
     * @return the error code
     */
    public String getCode() {
        return code;
    }

    @Override
    public String toString() {
        if (getCause() != null) {
            return code + ": " + getMessage() + " (caused by: " + getCause() + ")";
        }
        return code + ": " + getMessage();
    }
}
