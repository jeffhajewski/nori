/**
 * Custom error classes for NoriKV client.
 */

/**
 * Base error class for all NoriKV errors.
 */
export class NoriKVError extends Error {
  constructor(message: string, public readonly code?: string) {
    super(message);
    this.name = 'NoriKVError';
    Object.setPrototypeOf(this, NoriKVError.prototype);
  }
}

/**
 * Error indicating the contacted node is not the leader for the requested shard.
 * Client should retry on the leader node indicated in the error.
 */
export class NotLeaderError extends NoriKVError {
  constructor(
    message: string,
    /** The actual leader address (if known) */
    public readonly leaderHint?: string,
    /** The shard ID */
    public readonly shardId?: number
  ) {
    super(message, 'NOT_LEADER');
    this.name = 'NotLeaderError';
    Object.setPrototypeOf(this, NotLeaderError.prototype);
  }
}

/**
 * Error indicating the key already exists (for ifNotExists operations).
 */
export class AlreadyExistsError extends NoriKVError {
  constructor(
    message: string,
    public readonly key: Uint8Array
  ) {
    super(message, 'ALREADY_EXISTS');
    this.name = 'AlreadyExistsError';
    Object.setPrototypeOf(this, AlreadyExistsError.prototype);
  }
}

/**
 * Error indicating version mismatch (for ifMatchVersion operations).
 */
export class VersionMismatchError extends NoriKVError {
  constructor(
    message: string,
    public readonly key: Uint8Array,
    public readonly expectedVersion: any,
    public readonly actualVersion: any
  ) {
    super(message, 'VERSION_MISMATCH');
    this.name = 'VersionMismatchError';
    Object.setPrototypeOf(this, VersionMismatchError.prototype);
  }
}

/**
 * Error indicating the service is temporarily unavailable.
 * Client should retry with backoff.
 */
export class UnavailableError extends NoriKVError {
  constructor(message: string) {
    super(message, 'UNAVAILABLE');
    this.name = 'UnavailableError';
    Object.setPrototypeOf(this, UnavailableError.prototype);
  }
}

/**
 * Error indicating the request exceeded the deadline/timeout.
 */
export class DeadlineExceededError extends NoriKVError {
  constructor(message: string, public readonly timeoutMs: number) {
    super(message, 'DEADLINE_EXCEEDED');
    this.name = 'DeadlineExceededError';
    Object.setPrototypeOf(this, DeadlineExceededError.prototype);
  }
}

/**
 * Error indicating invalid request parameters.
 */
export class InvalidArgumentError extends NoriKVError {
  constructor(message: string) {
    super(message, 'INVALID_ARGUMENT');
    this.name = 'InvalidArgumentError';
    Object.setPrototypeOf(this, InvalidArgumentError.prototype);
  }
}

/**
 * Error indicating connection failure to the cluster.
 */
export class ConnectionError extends NoriKVError {
  constructor(message: string, public readonly address?: string) {
    super(message, 'CONNECTION_ERROR');
    this.name = 'ConnectionError';
    Object.setPrototypeOf(this, ConnectionError.prototype);
  }
}

/**
 * Error indicating no nodes are available.
 */
export class NoNodesAvailableError extends NoriKVError {
  constructor(message: string = 'No nodes available in the cluster') {
    super(message, 'NO_NODES_AVAILABLE');
    this.name = 'NoNodesAvailableError';
    Object.setPrototypeOf(this, NoNodesAvailableError.prototype);
  }
}

/**
 * Error indicating maximum retry attempts have been exhausted.
 */
export class RetryExhaustedError extends NoriKVError {
  constructor(
    message: string,
    public readonly attempts: number,
    public readonly lastError?: Error
  ) {
    super(message, 'RETRY_EXHAUSTED');
    this.name = 'RetryExhaustedError';
    Object.setPrototypeOf(this, RetryExhaustedError.prototype);
  }
}

/**
 * Convert gRPC status code to NoriKV error.
 */
export function fromGrpcError(error: any, metadata?: any): NoriKVError {
  const code = error.code;
  const message = error.details || error.message || 'Unknown error';

  // Extract leader hint from metadata if present
  const leaderHint = metadata?.get?.('leader-hint')?.[0];

  switch (code) {
    case 14: // UNAVAILABLE
      if (message.includes('NOT_LEADER')) {
        return new NotLeaderError(message, leaderHint);
      }
      return new UnavailableError(message);

    case 4: // DEADLINE_EXCEEDED
      return new DeadlineExceededError(message, 0);

    case 3: // INVALID_ARGUMENT
      return new InvalidArgumentError(message);

    case 6: // ALREADY_EXISTS
      return new AlreadyExistsError(message, new Uint8Array());

    case 9: // FAILED_PRECONDITION (version mismatch)
      return new VersionMismatchError(message, new Uint8Array(), null, null);

    default:
      return new NoriKVError(message, String(code));
  }
}
