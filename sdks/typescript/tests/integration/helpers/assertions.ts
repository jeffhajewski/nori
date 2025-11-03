/**
 * Custom assertion utilities for integration tests.
 */

import { expect } from 'vitest';
import type { Version, GetResult } from '@norikv/client/types';
import type { NoriKVClient } from '@norikv/client/client';

/**
 * Assert that a version object is valid.
 */
export function expectValidVersion(version: Version | null): void {
  expect(version).not.toBeNull();
  expect(version!.term).toBeGreaterThan(0n);
  expect(version!.index).toBeGreaterThan(0n);
}

/**
 * Assert that two versions are equal.
 */
export function expectVersionsEqual(v1: Version | null, v2: Version | null): void {
  if (v1 === null && v2 === null) {
    return;
  }
  expect(v1).not.toBeNull();
  expect(v2).not.toBeNull();
  expect(v1!.term).toBe(v2!.term);
  expect(v1!.index).toBe(v2!.index);
}

/**
 * Assert that a key exists in the store.
 */
export async function expectKeyExists(
  client: NoriKVClient,
  key: string,
  expectedValue?: string | Buffer
): Promise<GetResult> {
  const result = await client.get(key);
  expect(result.value).not.toBeNull();

  if (expectedValue) {
    const expected = typeof expectedValue === 'string'
      ? Buffer.from(expectedValue)
      : expectedValue;
    expect(result.value).toEqual(expected);
  }

  return result;
}

/**
 * Assert that a key does not exist in the store.
 */
export async function expectKeyNotExists(
  client: NoriKVClient,
  key: string
): Promise<void> {
  const result = await client.get(key);
  expect(result.value).toBeNull();
}

/**
 * Assert eventual consistency - key becomes available within timeout.
 */
export async function expectEventualConsistency(
  client: NoriKVClient,
  key: string,
  expectedValue: string | Buffer,
  options: {
    timeoutMs?: number;
    pollIntervalMs?: number;
  } = {}
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 5000;
  const pollIntervalMs = options.pollIntervalMs ?? 100;
  const expected = typeof expectedValue === 'string'
    ? Buffer.from(expectedValue)
    : expectedValue;

  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    try {
      const result = await client.get(key);
      if (result.value && result.value.equals(expected)) {
        return; // Success!
      }
    } catch (err) {
      // Ignore errors during polling
    }

    await sleep(pollIntervalMs);
  }

  throw new Error(
    `Key "${key}" did not reach expected value within ${timeoutMs}ms`
  );
}

/**
 * Assert that a value is eventually deleted.
 */
export async function expectEventualDeletion(
  client: NoriKVClient,
  key: string,
  options: {
    timeoutMs?: number;
    pollIntervalMs?: number;
  } = {}
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 5000;
  const pollIntervalMs = options.pollIntervalMs ?? 100;

  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    try {
      const result = await client.get(key);
      if (result.value === null) {
        return; // Success!
      }
    } catch (err) {
      // Ignore errors during polling
    }

    await sleep(pollIntervalMs);
  }

  throw new Error(
    `Key "${key}" was not deleted within ${timeoutMs}ms`
  );
}

/**
 * Assert that a buffer equals a string value.
 */
export function expectBufferEquals(
  buffer: Buffer | null,
  expectedString: string
): void {
  expect(buffer).not.toBeNull();
  expect(buffer!.toString('utf8')).toBe(expectedString);
}

/**
 * Assert that a promise rejects with a specific error message.
 */
export async function expectToRejectWith(
  promise: Promise<any>,
  errorMessage: string | RegExp
): Promise<void> {
  await expect(promise).rejects.toThrow(errorMessage);
}

/**
 * Assert that a promise rejects with a specific error type.
 */
export async function expectToRejectWithError(
  promise: Promise<any>,
  errorType: new (...args: any[]) => Error
): Promise<void> {
  await expect(promise).rejects.toBeInstanceOf(errorType);
}

/**
 * Wait for a condition to become true.
 */
export async function waitFor(
  condition: () => boolean | Promise<boolean>,
  options: {
    timeoutMs?: number;
    pollIntervalMs?: number;
    message?: string;
  } = {}
): Promise<void> {
  const timeoutMs = options.timeoutMs ?? 5000;
  const pollIntervalMs = options.pollIntervalMs ?? 100;
  const message = options.message ?? 'Condition not met within timeout';

  const startTime = Date.now();

  while (Date.now() - startTime < timeoutMs) {
    const result = await condition();
    if (result) {
      return;
    }
    await sleep(pollIntervalMs);
  }

  throw new Error(`${message} (timeout: ${timeoutMs}ms)`);
}

/**
 * Sleep for a specified number of milliseconds.
 */
export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

/**
 * Retry an operation with exponential backoff.
 */
export async function retryWithBackoff<T>(
  operation: () => Promise<T>,
  options: {
    maxAttempts?: number;
    initialDelayMs?: number;
    maxDelayMs?: number;
    backoffMultiplier?: number;
  } = {}
): Promise<T> {
  const maxAttempts = options.maxAttempts ?? 3;
  const initialDelayMs = options.initialDelayMs ?? 100;
  const maxDelayMs = options.maxDelayMs ?? 5000;
  const backoffMultiplier = options.backoffMultiplier ?? 2;

  let lastError: Error | null = null;
  let delayMs = initialDelayMs;

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await operation();
    } catch (err) {
      lastError = err instanceof Error ? err : new Error(String(err));

      if (attempt < maxAttempts) {
        await sleep(delayMs);
        delayMs = Math.min(delayMs * backoffMultiplier, maxDelayMs);
      }
    }
  }

  throw lastError || new Error('Operation failed after retries');
}

/**
 * Generate a random test key.
 */
export function randomKey(prefix: string = 'test'): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).substring(7)}`;
}

/**
 * Generate a random test value.
 */
export function randomValue(length: number = 16): string {
  return Math.random().toString(36).substring(2, 2 + length);
}
