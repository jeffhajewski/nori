import { decode } from "@msgpack/msgpack";
import type { EventBatch, ServerMessage, ClientMessage } from "@/types/events";
import { useEventStore } from "@/stores/eventStore";

// ============================================================================
// Configuration
// ============================================================================

interface WsConfig {
  url: string;
  reconnectBaseMs: number;
  reconnectMaxMs: number;
  maxRetries: number;
  useJson: boolean;
}

const DEFAULT_CONFIG: WsConfig = {
  url: process.env.NEXT_PUBLIC_VIZD_WS_URL || "ws://localhost:9090/ws/events",
  reconnectBaseMs: 1000,
  reconnectMaxMs: 30000,
  maxRetries: Infinity,
  useJson: true, // Always use JSON for browser compatibility
};

// ============================================================================
// WebSocket Client
// ============================================================================

class VizWebSocketClient {
  private ws: WebSocket | null = null;
  private config: WsConfig;
  private retryCount = 0;
  private reconnectTimeout: ReturnType<typeof setTimeout> | null = null;
  private pingInterval: ReturnType<typeof setInterval> | null = null;

  constructor(config: Partial<WsConfig> = {}) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  connect(): void {
    if (this.ws?.readyState === WebSocket.CONNECTING || this.ws?.readyState === WebSocket.OPEN) {
      return;
    }

    useEventStore.getState().setWsStatus("connecting");

    const url = new URL(this.config.url);
    url.searchParams.set("json", "true");

    try {
      this.ws = new WebSocket(url.toString());
      this.ws.binaryType = "arraybuffer";

      this.ws.onopen = this.handleOpen.bind(this);
      this.ws.onmessage = this.handleMessage.bind(this);
      this.ws.onerror = this.handleError.bind(this);
      this.ws.onclose = this.handleClose.bind(this);
    } catch (error) {
      console.error("[WS] Connection error:", error);
      this.scheduleReconnect();
    }
  }

  private handleOpen(): void {
    this.retryCount = 0;
    useEventStore.getState().setWsStatus("connected");
    console.log("[WS] Connected to vizd");

    // Start ping interval
    this.pingInterval = setInterval(() => {
      this.send({ type: "ping" });
    }, 30000);
  }

  private handleMessage(event: MessageEvent): void {
    try {
      const msg = this.deserialize(event.data);
      this.processMessage(msg);
    } catch (error) {
      console.error("[WS] Message parse error:", error);
    }
  }

  private handleError(event: Event): void {
    console.error("[WS] WebSocket error:", event);
  }

  private handleClose(event: CloseEvent): void {
    console.log(`[WS] Connection closed: code=${event.code} reason=${event.reason}`);
    this.cleanup();
    this.scheduleReconnect();
  }

  private processMessage(msg: ServerMessage): void {
    switch (msg.type) {
      case "connected":
        console.log(`[WS] Server version: ${msg.version}`);
        useEventStore.getState().setServerVersion(msg.version);
        break;

      case "batch":
        useEventStore.getState().ingestBatch(msg.data as unknown as EventBatch);
        break;

      case "pong":
        // Keepalive acknowledged
        break;

      case "error":
        console.error("[WS] Server error:", msg.message);
        break;
    }
  }

  private deserialize(data: ArrayBuffer | string): ServerMessage {
    if (typeof data === "string") {
      return JSON.parse(data);
    }
    // MessagePack decode
    return decode(new Uint8Array(data)) as ServerMessage;
  }

  private scheduleReconnect(): void {
    if (this.retryCount >= this.config.maxRetries) {
      useEventStore.getState().setWsStatus("disconnected");
      return;
    }

    useEventStore.getState().setWsStatus("reconnecting");

    // Exponential backoff with jitter
    const backoffMs = Math.min(
      this.config.reconnectBaseMs * Math.pow(2, this.retryCount),
      this.config.reconnectMaxMs
    );
    const jitter = Math.random() * 0.3 * backoffMs;
    const delay = backoffMs + jitter;

    console.log(`[WS] Reconnecting in ${Math.round(delay)}ms (attempt ${this.retryCount + 1})`);

    this.reconnectTimeout = setTimeout(() => {
      this.retryCount++;
      this.connect();
    }, delay);
  }

  private cleanup(): void {
    if (this.pingInterval) {
      clearInterval(this.pingInterval);
      this.pingInterval = null;
    }
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout);
      this.reconnectTimeout = null;
    }
  }

  send(msg: ClientMessage): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  updateFilter(filter: { nodes?: number[]; types?: string[]; shards?: number[] }): void {
    this.send({ type: "filter", ...filter });
  }

  pause(): void {
    this.send({ type: "pause" });
  }

  resume(): void {
    this.send({ type: "resume" });
  }

  requestHistory(fromMs: number, toMs: number): void {
    this.send({ type: "history", from_ms: fromMs, to_ms: toMs });
  }

  disconnect(): void {
    this.cleanup();
    if (this.ws) {
      this.ws.close(1000, "Client disconnect");
      this.ws = null;
    }
    useEventStore.getState().setWsStatus("disconnected");
  }

  isConnected(): boolean {
    return this.ws?.readyState === WebSocket.OPEN;
  }
}

// Singleton instance
export const wsClient = new VizWebSocketClient();
