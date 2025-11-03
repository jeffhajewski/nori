/**
 * Retry logic with exponential backoff for NoriKV client.
 *
 * This module handles:
 * - Retrying failed requests with exponential backoff
 * - Distinguishing retryable vs non-retryable errors
 * - Handling NOT_LEADER redirects
 * - Respecting idempotency semantics
 */

import {
  NoriKVError,
  NotLeaderError,
  UnavailableError,
  DeadlineExceededError,
  ConnectionError,
  RetryExhaustedError,
  AlreadyExistsError,
  VersionMismatchError,
  InvalidArgumentError,
} from '@norikv/client/errors';
import type { RetryConfig } from '@norikv/client/types';

/**
 * Context for a retry operation.
 */
export interface RetryContext {
  /** Current attempt number (1-indexed) */
  attempt: number;

  /** Last error encountered */
  lastError?: Error;

  /** Whether this operation is idempotent */
  isIdempotent: boolean;

  /** Idempotency key (if provided) */
  idempotencyKey?: string;
}

/**
 * Default retry configuration.
 */
export const DEFAULT_RETRY_CONFIG: Required<RetryConfig> = {
  maxAttempts: 3,
  initialDelayMs: 10,
  maxDelayMs: 1000,
  backoffMultiplier: 2,
  jitterMs: 100,
  retryOnNotLeader: true,
};

/**
 * Determine if an error is retryable.
 */
export function isRetryable(error: Error, context: RetryContext): boolean {
  // Non-retryable errors
  if (error instanceof InvalidArgumentError) {
    return false; // Client error - won't succeed on retry
  }

  if (error instanceof AlreadyExistsError) {
    return false; // Conflict - retry won't help
  }

  if (error instanceof VersionMismatchError) {
    return false; // Optimistic locking failure - caller must handle
  }

  // Retryable errors
  if (error instanceof NotLeaderError) {
    return true; // Can retry on the correct leader
  }

  if (error instanceof UnavailableError) {
    return true; // Temporary unavailability
  }

  if (error instanceof DeadlineExceededError) {
    return context.isIdempotent; // Only retry timeouts if idempotent
  }

  if (error instanceof ConnectionError) {
    return true; // Network error - might succeed on retry
  }

  if (error instanceof NoriKVError) {
    // Generic NoriKV error - retry if idempotent
    return context.isIdempotent;
  }

  // Unknown error - only retry if idempotent
  return context.isIdempotent;
}

/**
 * Compute the delay before the next retry attempt.
 */
export function computeRetryDelay(
  attempt: number,
  config: Required<RetryConfig>
): number {
  // Exponential backoff: delay = initialDelay * (multiplier ^ attempt)
  const exponentialDelay = config.initialDelayMs * Math.pow(config.backoffMultiplier, attempt - 1);

  // Cap at max delay
  const cappedDelay = Math.min(exponentialDelay, config.maxDelayMs);

  // Add jitter: random value in [-jitterMs, +jitterMs]
  const jitter = (Math.random() - 0.5) * 2 * config.jitterMs;

  return Math.max(0, cappedDelay + jitter);
}

/**
 * Sleep for a given duration.
 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Retry an operation with exponential backoff.
 *
 * @param operation - The operation to retry
 * @param config - Retry configuration
 * @param context - Retry context
 * @returns Result of the operation
 * @throws RetryExhaustedError if max attempts exceeded
 */
export async function withRetry<T>(
  operation: (ctx: RetryContext) => Promise<T>,
  config: RetryConfig,
  context: Omit<RetryContext, 'attempt'>
): Promise<T> {
  const fullConfig: Required<RetryConfig> = {
    ...DEFAULT_RETRY_CONFIG,
    ...config,
  };

  let lastError: Error | undefined;

  for (let attempt = 1; attempt <= fullConfig.maxAttempts; attempt++) {
    const ctx: RetryContext = {
      ...context,
      attempt,
      lastError,
    };

    try {
      return await operation(ctx);
    } catch (error) {
      lastError = error instanceof Error ? error : new Error(String(error));

      // Check if we should retry
      if (!isRetryable(lastError, ctx)) {
        throw lastError;
      }

      // Check if we've exhausted attempts
      if (attempt >= fullConfig.maxAttempts) {
        throw new RetryExhaustedError(
          `Operation failed after ${attempt} attempts`,
          attempt,
          lastError
        );
      }

      // Compute delay and sleep
      const delay = computeRetryDelay(attempt, fullConfig);
      await sleep(delay);
    }
  }

  // Should never reach here, but TypeScript needs it
  throw new RetryExhaustedError(
    `Operation failed after ${fullConfig.maxAttempts} attempts`,
    fullConfig.maxAttempts,
    lastError
  );
}

/**
 * Retry policy builder for fluent API.
 */
export class RetryPolicy {
  private config: Partial<RetryConfig> = {};

  /**
   * Set maximum number of attempts.
   */
  maxAttempts(attempts: number): this {
    this.config.maxAttempts = attempts;
    return this;
  }

  /**
   * Set initial backoff delay in milliseconds.
   */
  initialDelay(ms: number): this {
    this.config.initialDelayMs = ms;
    return this;
  }

  /**
   * Set maximum backoff delay in milliseconds.
   */
  maxDelay(ms: number): this {
    this.config.maxDelayMs = ms;
    return this;
  }

  /**
   * Set backoff multiplier.
   */
  backoffMultiplier(multiplier: number): this {
    this.config.backoffMultiplier = multiplier;
    return this;
  }

  /**
   * Set jitter in milliseconds.
   */
  jitter(ms: number): this {
    this.config.jitterMs = ms;
    return this;
  }

  /**
   * Enable or disable retries on NOT_LEADER errors.
   */
  retryOnNotLeader(enabled: boolean): this {
    this.config.retryOnNotLeader = enabled;
    return this;
  }

  /**
   * Build the retry configuration.
   */
  build(): RetryConfig {
    return { ...this.config };
  }

  /**
   * Execute an operation with this retry policy.
   */
  async execute<T>(
    operation: (ctx: RetryContext) => Promise<T>,
    context: Omit<RetryContext, 'attempt'>
  ): Promise<T> {
    return withRetry(operation, this.build(), context);
  }
}

/**
 * Create a new retry policy builder.
 */
export function retryPolicy(): RetryPolicy {
  return new RetryPolicy();
}
