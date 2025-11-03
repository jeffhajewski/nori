/**
 * Generate hash test vectors for cross-SDK validation.
 *
 * This script generates hash values and shard assignments that can be used
 * to validate parity between TypeScript, Python, Go, and Java SDKs.
 */

import { initializeHasher, xxhash64, jumpConsistentHash, getShardForKey } from '../src/hash';

async function main() {
  await initializeHasher();

  const testKeys = [
    'hello',
    'world',
    'user:123',
    'test-key',
    '',
    '🌍',
    'foo',
    'bar',
    'a',
    'ab',
    'abc',
    'test-value-1',
    'test-value-2',
  ];

  console.log('# Hash Test Vectors');
  console.log('# Generated from TypeScript SDK');
  console.log('');

  console.log('## xxhash64 Test Vectors');
  console.log('```');
  for (const key of testKeys) {
    const hash = xxhash64(key);
    console.log(`${JSON.stringify(key)}: ${hash}n`);
  }
  console.log('```');
  console.log('');

  console.log('## Shard Assignment Test Vectors (1024 shards)');
  console.log('```');
  for (const key of testKeys) {
    const shard = getShardForKey(key, 1024);
    console.log(`${JSON.stringify(key)}: ${shard}`);
  }
  console.log('```');
  console.log('');

  console.log('## Jump Consistent Hash Test Vectors');
  console.log('```');
  const jchTestCases = [
    [0n, 100],
    [1n, 100],
    [12345678901234567890n, 1024],
    [0xFFFFFFFFFFFFFFFFn, 1024],
  ];

  for (const [hash, buckets] of jchTestCases) {
    const bucket = jumpConsistentHash(hash as bigint, buckets as number);
    console.log(`hash=${hash}n, buckets=${buckets}: ${bucket}`);
  }
  console.log('```');
}

main().catch(console.error);
