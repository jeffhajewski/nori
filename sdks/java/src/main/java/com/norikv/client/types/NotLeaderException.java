package com.norikv.client.types;

/**
 * Indicates the contacted node is not the leader for the requested shard.
 * Client should retry on the leader node indicated in the leader hint.
 */
public class NotLeaderException extends NoriKVException {
    private final String leaderHint;
    private final int shardId;

    /**
     * Creates a new NotLeaderException.
     *
     * @param message the error message
     * @param leaderHint address of the current leader (if known)
     * @param shardId shard ID that was accessed
     */
    public NotLeaderException(String message, String leaderHint, int shardId) {
        super("NOT_LEADER", message);
        this.leaderHint = leaderHint;
        this.shardId = shardId;
    }

    /**
     * Creates a new NotLeaderException with a cause.
     *
     * @param message the error message
     * @param leaderHint address of the current leader (if known)
     * @param shardId shard ID that was accessed
     * @param cause the underlying cause
     */
    public NotLeaderException(String message, String leaderHint, int shardId, Throwable cause) {
        super("NOT_LEADER", message, cause);
        this.leaderHint = leaderHint;
        this.shardId = shardId;
    }

    /**
     * Gets the leader hint.
     *
     * @return address of the current leader, or null if unknown
     */
    public String getLeaderHint() {
        return leaderHint;
    }

    /**
     * Gets the shard ID.
     *
     * @return the shard ID that was accessed
     */
    public int getShardId() {
        return shardId;
    }
}
