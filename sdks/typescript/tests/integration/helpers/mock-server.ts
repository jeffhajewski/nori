/**
 * Mock gRPC server for integration testing.
 *
 * Provides an in-memory implementation of NoriKV gRPC services for testing
 * SDK behavior without requiring a real server.
 */

import * as grpc from '@grpc/grpc-js';
import { KvService, MetaService, type PutRequest, type PutResponse, type GetRequest, type GetResponse, type DeleteRequest, type DeleteResponse, type ClusterView } from '@norikv/client/proto/norikv';

interface MockKVStore {
  [key: string]: {
    value: Buffer;
    version: { term: number; index: number };
    ttl?: number;
    createdAt: number;
  };
}

interface CallRecord {
  method: string;
  request: any;
  timestamp: number;
}

export class MockNoriKVServer {
  private server: grpc.Server;
  private store: MockKVStore = {};
  private callHistory: CallRecord[] = [];
  private port: number = 0;
  private currentTerm: number = 1;
  private currentIndex: number = 0;
  private leaderId: string = 'mock-leader';
  private notLeaderError: boolean = false;
  private errorToThrow: grpc.ServiceError | null = null;
  private clusterView: ClusterView | null = null;

  constructor() {
    this.server = new grpc.Server();
    this.setupKvService();
    this.setupMetaService();
  }

  /**
   * Set up mock KV service handlers.
   */
  private setupKvService(): void {
    const kvService: grpc.UntypedServiceImplementation = {
      Put: (call: grpc.ServerUnaryCall<PutRequest, PutResponse>, callback: grpc.sendUnaryData<PutResponse>) => {
        this.recordCall('Put', call.request);

        if (this.errorToThrow) {
          callback(this.errorToThrow, null);
          return;
        }

        if (this.notLeaderError) {
          const error: grpc.ServiceError = {
            name: 'NOT_LEADER',
            message: `Not the leader. Try ${this.leaderId}`,
            code: grpc.status.FAILED_PRECONDITION,
            details: this.leaderId,
          };
          callback(error, null);
          return;
        }

        const key = call.request.key.toString('hex');
        this.currentIndex++;

        const version = {
          term: this.currentTerm,
          index: this.currentIndex,
        };

        this.store[key] = {
          value: call.request.value,
          version,
          ttl: call.request.ttlMs > 0 ? call.request.ttlMs : undefined,
          createdAt: Date.now(),
        };

        const response: PutResponse = {
          version,
          meta: {},
        };

        callback(null, response);
      },

      Get: (call: grpc.ServerUnaryCall<GetRequest, GetResponse>, callback: grpc.sendUnaryData<GetResponse>) => {
        this.recordCall('Get', call.request);

        if (this.errorToThrow) {
          callback(this.errorToThrow, null);
          return;
        }

        const key = call.request.key.toString('hex');
        const entry = this.store[key];

        if (!entry) {
          const response: GetResponse = {
            value: Buffer.alloc(0),
            version: undefined,
            meta: {},
          };
          callback(null, response);
          return;
        }

        // Check TTL
        if (entry.ttl && Date.now() - entry.createdAt > entry.ttl) {
          delete this.store[key];
          const response: GetResponse = {
            value: Buffer.alloc(0),
            version: undefined,
            meta: {},
          };
          callback(null, response);
          return;
        }

        const response: GetResponse = {
          value: entry.value,
          version: entry.version,
          meta: {},
        };

        callback(null, response);
      },

      Delete: (call: grpc.ServerUnaryCall<DeleteRequest, DeleteResponse>, callback: grpc.sendUnaryData<DeleteResponse>) => {
        this.recordCall('Delete', call.request);

        if (this.errorToThrow) {
          callback(this.errorToThrow, null);
          return;
        }

        if (this.notLeaderError) {
          const error: grpc.ServiceError = {
            name: 'NOT_LEADER',
            message: `Not the leader. Try ${this.leaderId}`,
            code: grpc.status.FAILED_PRECONDITION,
            details: this.leaderId,
          };
          callback(error, null);
          return;
        }

        const key = call.request.key.toString('hex');
        const existed = key in this.store;
        delete this.store[key];

        const response: DeleteResponse = {
          tombstoned: existed,
        };

        callback(null, response);
      },
    };

    this.server.addService(KvService, kvService);
  }

  /**
   * Set up mock Meta service handlers.
   */
  private setupMetaService(): void {
    const metaService: grpc.UntypedServiceImplementation = {
      WatchCluster: (call: grpc.ServerWritableStream<ClusterView, ClusterView>) => {
        this.recordCall('WatchCluster', call.request);

        // Send initial cluster view
        if (this.clusterView) {
          call.write(this.clusterView);
        } else {
          // Default cluster view
          const defaultView: ClusterView = {
            epoch: 1,
            nodes: [
              { id: 'node-1', addr: `localhost:${this.port}`, role: 'leader' },
            ],
            shards: [],
          };
          call.write(defaultView);
        }

        // Keep stream open until client cancels
        call.on('cancelled', () => {
          call.end();
        });
      },
    };

    this.server.addService(MetaService, metaService);
  }

  /**
   * Start the mock server on a random available port.
   */
  async start(): Promise<number> {
    return new Promise((resolve, reject) => {
      this.server.bindAsync(
        '127.0.0.1:0', // Port 0 = random available port
        grpc.ServerCredentials.createInsecure(),
        (err, port) => {
          if (err) {
            reject(err);
          } else {
            this.server.start();
            this.port = port;
            resolve(port);
          }
        }
      );
    });
  }

  /**
   * Stop the mock server.
   */
  async stop(): Promise<void> {
    return new Promise((resolve) => {
      this.server.tryShutdown(() => {
        resolve();
      });
    });
  }

  /**
   * Get the server address.
   */
  getAddress(): string {
    return `127.0.0.1:${this.port}`;
  }

  /**
   * Clear all stored data.
   */
  clearStore(): void {
    this.store = {};
    this.callHistory = [];
  }

  /**
   * Get a value from the store directly (for testing).
   */
  getStoreValue(key: string | Buffer): Buffer | null {
    const keyHex = typeof key === 'string' ? Buffer.from(key).toString('hex') : key.toString('hex');
    const entry = this.store[keyHex];
    return entry ? entry.value : null;
  }

  /**
   * Set mock to return NOT_LEADER error.
   */
  setNotLeader(leaderId: string = 'mock-leader-2'): void {
    this.notLeaderError = true;
    this.leaderId = leaderId;
  }

  /**
   * Clear NOT_LEADER error state.
   */
  clearNotLeader(): void {
    this.notLeaderError = false;
  }

  /**
   * Set a custom error to throw on next request.
   */
  setError(error: grpc.ServiceError): void {
    this.errorToThrow = error;
  }

  /**
   * Clear custom error.
   */
  clearError(): void {
    this.errorToThrow = null;
  }

  /**
   * Set the cluster view to return in WatchCluster.
   */
  setClusterView(view: ClusterView): void {
    this.clusterView = view;
  }

  /**
   * Get call history for assertions.
   */
  getCallHistory(): CallRecord[] {
    return [...this.callHistory];
  }

  /**
   * Get calls for a specific method.
   */
  getCallsForMethod(method: string): CallRecord[] {
    return this.callHistory.filter(c => c.method === method);
  }

  /**
   * Clear call history.
   */
  clearCallHistory(): void {
    this.callHistory = [];
  }

  /**
   * Record a method call.
   */
  private recordCall(method: string, request: any): void {
    this.callHistory.push({
      method,
      request,
      timestamp: Date.now(),
    });
  }
}

/**
 * Create and start a mock server for testing.
 */
export async function createMockServer(): Promise<MockNoriKVServer> {
  const server = new MockNoriKVServer();
  await server.start();
  return server;
}
