/**
 * Connection pool management for NoriKV cluster nodes.
 *
 * This module handles:
 * - gRPC channel lifecycle (connect, reconnect, disconnect)
 * - Health checking with exponential backoff
 * - Connection pooling per node
 * - Graceful shutdown
 */

import * as grpc from '@grpc/grpc-js';
import { ConnectionError, UnavailableError } from '@norikv/client/errors';
import type { NodeConnection } from '@norikv/client/types';
import { createKvClient, createMetaClient, createVectorClient, closeClient } from '@norikv/client/grpc-services';
import type { KvClient, MetaClient, VectorClient } from '@norikv/client/proto/norikv';

/**
 * Configuration for connection pool.
 */
export interface ConnectionPoolConfig {
  /** Maximum connections per node (default: 10) */
  maxConnectionsPerNode?: number;

  /** Connection timeout in milliseconds (default: 5000) */
  connectTimeout?: number;

  /** Health check interval in milliseconds (default: 30000) */
  healthCheckInterval?: number;

  /** Initial reconnect delay in milliseconds (default: 100) */
  reconnectDelayMs?: number;

  /** Maximum reconnect delay in milliseconds (default: 30000) */
  maxReconnectDelayMs?: number;

  /** Backoff multiplier for reconnect attempts (default: 2) */
  reconnectBackoffMultiplier?: number;
}

/**
 * Manages gRPC connections to cluster nodes.
 */
export class ConnectionPool {
  private connections = new Map<string, NodeConnection>();
  private kvClients = new Map<string, KvClient>();
  private metaClients = new Map<string, MetaClient>();
  private vectorClients = new Map<string, VectorClient>();
  private reconnectTimers = new Map<string, NodeJS.Timeout>();
  private healthCheckTimers = new Map<string, NodeJS.Timeout>();
  private reconnectAttempts = new Map<string, number>();
  private closed = false;

  private readonly config: Required<ConnectionPoolConfig>;

  constructor(config: ConnectionPoolConfig = {}) {
    this.config = {
      maxConnectionsPerNode: config.maxConnectionsPerNode ?? 10,
      connectTimeout: config.connectTimeout ?? 5000,
      healthCheckInterval: config.healthCheckInterval ?? 30000,
      reconnectDelayMs: config.reconnectDelayMs ?? 100,
      maxReconnectDelayMs: config.maxReconnectDelayMs ?? 30000,
      reconnectBackoffMultiplier: config.reconnectBackoffMultiplier ?? 2,
    };
  }

  /**
   * Get or create a connection to a node.
   *
   * @param address - Node address (host:port)
   * @returns gRPC channel
   * @throws ConnectionError if connection fails
   */
  async getConnection(address: string): Promise<grpc.Channel> {
    if (this.closed) {
      throw new ConnectionError('Connection pool is closed', address);
    }

    let conn = this.connections.get(address);

    if (!conn) {
      conn = await this.createConnection(address);
      this.connections.set(address, conn);
      this.startHealthCheck(address);
    }

    // Check if connection is usable
    const state = conn.channel?.getConnectivityState(false);
    if (state === grpc.connectivityState.SHUTDOWN ||
        state === grpc.connectivityState.TRANSIENT_FAILURE) {
      // Try to reconnect
      await this.reconnect(address);
      conn = this.connections.get(address);
      if (!conn) {
        throw new ConnectionError('Failed to reconnect', address);
      }
    }

    if (!conn.channel) {
      throw new ConnectionError('No active channel', address);
    }

    return conn.channel;
  }

  /**
   * Get or create a KV service client for a node.
   *
   * @param address - Node address (host:port)
   * @returns KvClient instance
   * @throws ConnectionError if connection fails
   */
  async getKvClient(address: string): Promise<KvClient> {
    if (this.closed) {
      throw new ConnectionError('Connection pool is closed', address);
    }

    // Ensure connection exists
    await this.getConnection(address);

    let client = this.kvClients.get(address);
    if (!client) {
      client = createKvClient(address);
      this.kvClients.set(address, client);
    }

    return client;
  }

  /**
   * Get or create a Meta service client for a node.
   *
   * @param address - Node address (host:port)
   * @returns MetaClient instance
   * @throws ConnectionError if connection fails
   */
  async getMetaClient(address: string): Promise<MetaClient> {
    if (this.closed) {
      throw new ConnectionError('Connection pool is closed', address);
    }

    // Ensure connection exists
    await this.getConnection(address);

    let client = this.metaClients.get(address);
    if (!client) {
      client = createMetaClient(address);
      this.metaClients.set(address, client);
    }

    return client;
  }

  /**
   * Get or create a Vector service client for a node.
   *
   * @param address - Node address (host:port)
   * @returns VectorClient instance
   * @throws ConnectionError if connection fails
   */
  async getVectorClient(address: string): Promise<VectorClient> {
    if (this.closed) {
      throw new ConnectionError('Connection pool is closed', address);
    }

    // Ensure connection exists
    await this.getConnection(address);

    let client = this.vectorClients.get(address);
    if (!client) {
      client = createVectorClient(address);
      this.vectorClients.set(address, client);
    }

    return client;
  }

  /**
   * Create a new connection to a node.
   */
  private async createConnection(address: string): Promise<NodeConnection> {
    const channel = new grpc.Channel(
      address,
      grpc.credentials.createInsecure(),
      {
        'grpc.max_receive_message_length': 100 * 1024 * 1024, // 100MB
        'grpc.max_send_message_length': 100 * 1024 * 1024,
        'grpc.keepalive_time_ms': 30000,
        'grpc.keepalive_timeout_ms': 10000,
        'grpc.keepalive_permit_without_calls': 1,
        'grpc.http2.max_pings_without_data': 0,
      }
    );

    const conn: NodeConnection = {
      address,
      channel,
      status: 'connecting',
      lastHealthCheck: new Date(),
    };

    // Wait for connection to be ready
    await this.waitForReady(channel, address);

    conn.status = 'connected';
    this.reconnectAttempts.set(address, 0);

    // Watch for connectivity state changes
    this.watchConnectivity(address, channel);

    return conn;
  }

  /**
   * Wait for channel to be ready with timeout.
   */
  private async waitForReady(channel: grpc.Channel, address: string): Promise<void> {
    const deadline = Date.now() + this.config.connectTimeout;

    return new Promise((resolve, reject) => {
      const checkReady = () => {
        const state = channel.getConnectivityState(true); // Try to connect

        if (state === grpc.connectivityState.READY) {
          resolve();
        } else if (state === grpc.connectivityState.SHUTDOWN) {
          reject(new ConnectionError('Channel shut down', address));
        } else if (Date.now() >= deadline) {
          reject(new ConnectionError('Connection timeout', address));
        } else {
          // Wait for state change
          channel.watchConnectivityState(
            state,
            deadline,
            (error) => {
              if (error) {
                reject(new ConnectionError(`Connection failed: ${error.message}`, address));
              } else {
                checkReady();
              }
            }
          );
        }
      };

      checkReady();
    });
  }

  /**
   * Watch connectivity state changes and auto-reconnect.
   */
  private watchConnectivity(address: string, channel: grpc.Channel): void {
    const watch = () => {
      if (this.closed) return;

      const state = channel.getConnectivityState(false);
      const conn = this.connections.get(address);

      if (!conn) return;

      // Update connection status
      switch (state) {
        case grpc.connectivityState.READY:
          conn.status = 'connected';
          conn.lastHealthCheck = new Date();
          this.reconnectAttempts.set(address, 0);
          break;
        case grpc.connectivityState.CONNECTING:
          conn.status = 'connecting';
          break;
        case grpc.connectivityState.IDLE:
          conn.status = 'disconnected';
          break;
        case grpc.connectivityState.TRANSIENT_FAILURE:
          conn.status = 'failed';
          this.scheduleReconnect(address);
          break;
        case grpc.connectivityState.SHUTDOWN:
          conn.status = 'disconnected';
          return; // Don't watch anymore
      }

      // Continue watching
      channel.watchConnectivityState(
        state,
        Date.now() + 30000,
        () => watch()
      );
    };

    watch();
  }

  /**
   * Schedule a reconnect attempt with exponential backoff.
   */
  private scheduleReconnect(address: string): void {
    // Clear any existing timer
    const existingTimer = this.reconnectTimers.get(address);
    if (existingTimer) {
      clearTimeout(existingTimer);
    }

    const attempts = this.reconnectAttempts.get(address) ?? 0;
    const delay = Math.min(
      this.config.reconnectDelayMs * Math.pow(this.config.reconnectBackoffMultiplier, attempts),
      this.config.maxReconnectDelayMs
    );

    const timer = setTimeout(() => {
      this.reconnect(address).catch((err) => {
        console.error(`Failed to reconnect to ${address}:`, err);
      });
    }, delay);

    this.reconnectTimers.set(address, timer);
    this.reconnectAttempts.set(address, attempts + 1);
  }

  /**
   * Reconnect to a node.
   */
  private async reconnect(address: string): Promise<void> {
    if (this.closed) return;

    // Close old clients
    const oldKvClient = this.kvClients.get(address);
    if (oldKvClient) {
      closeClient(oldKvClient);
      this.kvClients.delete(address);
    }

    const oldMetaClient = this.metaClients.get(address);
    if (oldMetaClient) {
      closeClient(oldMetaClient);
      this.metaClients.delete(address);
    }

    const oldVectorClient = this.vectorClients.get(address);
    if (oldVectorClient) {
      closeClient(oldVectorClient);
      this.vectorClients.delete(address);
    }

    // Close old connection
    const oldConn = this.connections.get(address);
    if (oldConn?.channel) {
      oldConn.channel.close();
    }

    // Remove from map
    this.connections.delete(address);

    // Create new connection
    try {
      const newConn = await this.createConnection(address);
      this.connections.set(address, newConn);
    } catch (err) {
      console.error(`Reconnection to ${address} failed:`, err);
      this.scheduleReconnect(address);
      throw err;
    }
  }

  /**
   * Start periodic health checks for a node.
   */
  private startHealthCheck(address: string): void {
    const timer = setInterval(() => {
      this.healthCheck(address).catch((err) => {
        console.error(`Health check failed for ${address}:`, err);
      });
    }, this.config.healthCheckInterval);

    this.healthCheckTimers.set(address, timer);
  }

  /**
   * Perform a health check on a node.
   */
  private async healthCheck(address: string): Promise<void> {
    const conn = this.connections.get(address);
    if (!conn?.channel) return;

    const state = conn.channel.getConnectivityState(false);

    if (state === grpc.connectivityState.READY) {
      conn.lastHealthCheck = new Date();
      conn.status = 'connected';
    } else if (state === grpc.connectivityState.TRANSIENT_FAILURE ||
               state === grpc.connectivityState.SHUTDOWN) {
      conn.status = 'failed';
      this.scheduleReconnect(address);
    }
  }

  /**
   * Get all active connections.
   */
  getConnections(): Map<string, NodeConnection> {
    return new Map(this.connections);
  }

  /**
   * Get connection status for a node.
   */
  getConnectionStatus(address: string): NodeConnection | undefined {
    return this.connections.get(address);
  }

  /**
   * Remove a connection from the pool.
   */
  removeConnection(address: string): void {
    const conn = this.connections.get(address);
    if (!conn) return;

    // Close gRPC clients
    const kvClient = this.kvClients.get(address);
    if (kvClient) {
      closeClient(kvClient);
      this.kvClients.delete(address);
    }

    const metaClient = this.metaClients.get(address);
    if (metaClient) {
      closeClient(metaClient);
      this.metaClients.delete(address);
    }

    const vectorClient = this.vectorClients.get(address);
    if (vectorClient) {
      closeClient(vectorClient);
      this.vectorClients.delete(address);
    }

    // Clear timers
    const reconnectTimer = this.reconnectTimers.get(address);
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      this.reconnectTimers.delete(address);
    }

    const healthCheckTimer = this.healthCheckTimers.get(address);
    if (healthCheckTimer) {
      clearInterval(healthCheckTimer);
      this.healthCheckTimers.delete(address);
    }

    // Close channel
    if (conn.channel) {
      conn.channel.close();
    }

    // Remove from maps
    this.connections.delete(address);
    this.reconnectAttempts.delete(address);
  }

  /**
   * Close all connections and stop all timers.
   */
  async close(): Promise<void> {
    if (this.closed) return;
    this.closed = true;

    // Close all gRPC clients
    for (const client of this.kvClients.values()) {
      closeClient(client);
    }
    this.kvClients.clear();

    for (const client of this.metaClients.values()) {
      closeClient(client);
    }
    this.metaClients.clear();

    for (const client of this.vectorClients.values()) {
      closeClient(client);
    }
    this.vectorClients.clear();

    // Clear all timers
    for (const timer of this.reconnectTimers.values()) {
      clearTimeout(timer);
    }
    this.reconnectTimers.clear();

    for (const timer of this.healthCheckTimers.values()) {
      clearInterval(timer);
    }
    this.healthCheckTimers.clear();

    // Close all channels
    for (const conn of this.connections.values()) {
      if (conn.channel) {
        conn.channel.close();
      }
    }

    this.connections.clear();
    this.reconnectAttempts.clear();
  }

  /**
   * Check if the pool is closed.
   */
  isClosed(): boolean {
    return this.closed;
  }
}
