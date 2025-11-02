/**
 * Basic usage example for NoriKV TypeScript client.
 *
 * This example demonstrates:
 * - Connecting to a cluster
 * - Putting key-value pairs
 * - Getting values
 * - Deleting keys
 * - Error handling
 */

import {
  NoriKVClient,
  bytesToString,
  AlreadyExistsError,
  VersionMismatchError,
} from '../src/index.js';

async function main() {
  // Create client with seed nodes
  const client = new NoriKVClient({
    nodes: ['localhost:50051', 'localhost:50052', 'localhost:50053'],
    totalShards: 1024,
    timeout: 5000,
    retry: {
      maxAttempts: 3,
      initialDelayMs: 10,
      maxDelayMs: 1000,
    },
  });

  try {
    // Connect to cluster
    console.log('Connecting to NoriKV cluster...');
    await client.connect();
    console.log('Connected!');

    // === Basic Put/Get/Delete ===

    console.log('\n=== Basic Operations ===');

    // Put a value
    console.log('Putting key: user:123');
    const version1 = await client.put('user:123', 'Alice');
    console.log(`Put succeeded, version: ${version1.term}:${version1.index}`);

    // Get the value
    console.log('Getting key: user:123');
    const result1 = await client.get('user:123');
    if (result1.value) {
      console.log(`Value: ${bytesToString(result1.value)}`);
      console.log(`Version: ${result1.version?.term}:${result1.version?.index}`);
    }

    // Update the value
    console.log('Updating key: user:123');
    const version2 = await client.put('user:123', 'Bob');
    console.log(`Update succeeded, new version: ${version2.term}:${version2.index}`);

    // Delete the key
    console.log('Deleting key: user:123');
    await client.delete('user:123');
    console.log('Delete succeeded');

    // Try to get deleted key
    console.log('Getting deleted key: user:123');
    const result2 = await client.get('user:123');
    console.log(`Value after delete: ${result2.value === null ? 'null' : bytesToString(result2.value)}`);

    // === Conditional Puts ===

    console.log('\n=== Conditional Operations ===');

    // Put if not exists
    console.log('Put if not exists: user:456');
    await client.put('user:456', 'Charlie', { ifNotExists: true });
    console.log('First put succeeded');

    try {
      console.log('Try to put again with ifNotExists...');
      await client.put('user:456', 'David', { ifNotExists: true });
      console.log('ERROR: Should have thrown AlreadyExistsError');
    } catch (err) {
      if (err instanceof AlreadyExistsError) {
        console.log('Correctly rejected: key already exists');
      } else {
        throw err;
      }
    }

    // === Optimistic Locking (CAS) ===

    console.log('\n=== Optimistic Locking ===');

    // Get current version
    const currentResult = await client.get('user:456');
    console.log(`Current value: ${bytesToString(currentResult.value!)}`);
    console.log(`Current version: ${currentResult.version?.term}:${currentResult.version?.index}`);

    // Update with version check
    console.log('Updating with version check...');
    await client.put('user:456', 'Eve', {
      ifMatchVersion: currentResult.version!,
    });
    console.log('CAS update succeeded');

    // Try to update with stale version
    try {
      console.log('Try to update with stale version...');
      await client.put('user:456', 'Frank', {
        ifMatchVersion: currentResult.version!, // This is now stale
      });
      console.log('ERROR: Should have thrown VersionMismatchError');
    } catch (err) {
      if (err instanceof VersionMismatchError) {
        console.log('Correctly rejected: version mismatch');
      } else {
        throw err;
      }
    }

    // === TTL (Time-To-Live) ===

    console.log('\n=== TTL ===');

    console.log('Putting key with 5-second TTL: session:abc');
    await client.put('session:abc', 'temporary-data', { ttlMs: 5000 });
    console.log('Put succeeded, key will expire in 5 seconds');

    const sessionResult = await client.get('session:abc');
    console.log(`Value: ${bytesToString(sessionResult.value!)}`);

    // === Idempotency ===

    console.log('\n=== Idempotency ===');

    console.log('Putting with idempotency key...');
    await client.put('order:789', 'pending', {
      idempotencyKey: 'order-create-789',
    });
    console.log('First put succeeded');

    // Retry with same idempotency key (simulating network retry)
    console.log('Retrying with same idempotency key...');
    await client.put('order:789', 'pending', {
      idempotencyKey: 'order-create-789',
    });
    console.log('Retry succeeded (deduplicated by server)');

    // === Consistency Levels ===

    console.log('\n=== Consistency Levels ===');

    await client.put('config:feature-flag', 'enabled');

    // Lease-based read (default, fast linearizable)
    console.log('Lease read (default):');
    const leaseResult = await client.get('config:feature-flag', {
      consistency: 'lease',
    });
    console.log(`Value: ${bytesToString(leaseResult.value!)}`);

    // Strict linearizable read (read-index + quorum)
    console.log('Linearizable read:');
    const linearResult = await client.get('config:feature-flag', {
      consistency: 'linearizable',
    });
    console.log(`Value: ${bytesToString(linearResult.value!)}`);

    // Stale read (eventual consistency, fastest)
    console.log('Stale read:');
    const staleResult = await client.get('config:feature-flag', {
      consistency: 'stale_ok',
    });
    console.log(`Value: ${bytesToString(staleResult.value!)}`);

    // === Cluster Topology ===

    console.log('\n=== Cluster Topology ===');

    const clusterView = client.getClusterView();
    if (clusterView) {
      console.log(`Cluster epoch: ${clusterView.epoch}`);
      console.log(`Nodes: ${clusterView.nodes.length}`);
      for (const node of clusterView.nodes) {
        console.log(`  - ${node.id} @ ${node.addr} (${node.role})`);
      }
      console.log(`Shards: ${clusterView.shards.length}`);
    }

    console.log('\n=== All operations completed successfully ===');
  } catch (err) {
    console.error('Error:', err);
    process.exit(1);
  } finally {
    // Always close the client
    console.log('\nClosing client...');
    await client.close();
    console.log('Client closed');
  }
}

// Run the example
main().catch((err) => {
  console.error('Fatal error:', err);
  process.exit(1);
});
