package com.norikv.client.types;

/**
 * Exception thrown when a vector is not found.
 */
public class VectorNotFoundException extends NoriKVException {
    private final String vectorId;

    /**
     * Creates a new VectorNotFoundException.
     *
     * @param message the error message
     * @param vectorId the ID of the vector that was not found (may be null)
     */
    public VectorNotFoundException(String message, String vectorId) {
        super("NOT_FOUND", message);
        this.vectorId = vectorId;
    }

    /**
     * Creates a new VectorNotFoundException with a cause.
     *
     * @param message the error message
     * @param vectorId the ID of the vector that was not found (may be null)
     * @param cause the underlying cause
     */
    public VectorNotFoundException(String message, String vectorId, Throwable cause) {
        super("NOT_FOUND", message, cause);
        this.vectorId = vectorId;
    }

    /**
     * Gets the ID of the vector that was not found.
     *
     * @return the vector ID, or null if not specified
     */
    public String getVectorId() {
        return vectorId;
    }
}
