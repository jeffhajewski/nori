package com.norikv.client.types;

/**
 * Indicates connection failure to the cluster.
 */
public class ConnectionException extends NoriKVException {
    private final String address;

    /**
     * Creates a new ConnectionException.
     *
     * @param message the error message
     * @param address the address that failed to connect
     */
    public ConnectionException(String message, String address) {
        super("CONNECTION_ERROR", message);
        this.address = address;
    }

    /**
     * Creates a new ConnectionException with a cause.
     *
     * @param message the error message
     * @param address the address that failed to connect
     * @param cause the underlying cause
     */
    public ConnectionException(String message, String address, Throwable cause) {
        super("CONNECTION_ERROR", message, cause);
        this.address = address;
    }

    /**
     * Gets the address that failed to connect.
     *
     * @return the address
     */
    public String getAddress() {
        return address;
    }
}
