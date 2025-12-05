//! WebSocket handlers for event streaming.

use crate::aggregator::EventAggregator;
use crate::event::{current_timestamp_ms, EventBatch};
use crate::subscription::{ClientMessage, ServerMessage, SubscriptionFilter};
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Query, State, WebSocketUpgrade,
    },
    response::Response,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Query parameters for WebSocket connection.
#[derive(Debug, Deserialize)]
pub struct WsParams {
    /// Filter by node IDs (comma-separated).
    pub nodes: Option<String>,
    /// Filter by event types (comma-separated).
    pub types: Option<String>,
    /// Filter by shard IDs (comma-separated).
    pub shards: Option<String>,
    /// Use JSON encoding (default: MessagePack in release, JSON in debug).
    pub json: Option<bool>,
}

/// WebSocket upgrade handler.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<WsParams>,
    State(aggregator): State<Arc<EventAggregator>>,
) -> Response {
    let filter = SubscriptionFilter::from_query_params(
        params.nodes.as_deref(),
        params.types.as_deref(),
        params.shards.as_deref(),
    );

    let use_json = params.json.unwrap_or(cfg!(debug_assertions));

    info!(
        nodes = ?filter.nodes,
        types = ?filter.types,
        shards = ?filter.shards,
        json = use_json,
        "WebSocket connection"
    );

    ws.on_upgrade(move |socket| handle_socket(socket, aggregator, filter, use_json))
}

/// Handle an established WebSocket connection.
async fn handle_socket(
    socket: WebSocket,
    aggregator: Arc<EventAggregator>,
    mut filter: SubscriptionFilter,
    use_json: bool,
) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to event broadcast
    let mut event_rx = aggregator.subscribe();

    // Send initial connection message
    let ring_buffer = aggregator.ring_buffer();
    let connected_msg = ServerMessage::Connected {
        version: env!("CARGO_PKG_VERSION").to_string(),
        oldest_timestamp_ms: ring_buffer.oldest_timestamp(),
        newest_timestamp_ms: ring_buffer.newest_timestamp(),
    };

    if let Err(e) = send_message(&mut sender, &connected_msg, use_json).await {
        error!("Failed to send connected message: {}", e);
        return;
    }

    // State
    let mut paused = false;

    loop {
        tokio::select! {
            // Receive events from aggregator
            result = event_rx.recv() => {
                match result {
                    Ok(batch) => {
                        if paused {
                            continue;
                        }

                        // Apply filter
                        if let Some(filtered_batch) = filter.apply(&batch) {
                            let msg = ServerMessage::Batch(filtered_batch);
                            if let Err(e) = send_message(&mut sender, &msg, use_json).await {
                                debug!("Client disconnected: {}", e);
                                break;
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("Client lagged by {} messages", n);
                        // Continue anyway - client will miss some events
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("Broadcast channel closed");
                        break;
                    }
                }
            }

            // Receive messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(client_msg) => {
                                handle_client_message(
                                    client_msg,
                                    &mut filter,
                                    &mut paused,
                                    &mut sender,
                                    &aggregator,
                                    use_json,
                                ).await;
                            }
                            Err(e) => {
                                warn!("Invalid client message: {}", e);
                                let _ = send_message(
                                    &mut sender,
                                    &ServerMessage::Error { message: format!("Invalid message: {}", e) },
                                    use_json,
                                ).await;
                            }
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        // Try to decode as MessagePack
                        match rmp_serde::from_slice::<ClientMessage>(&data) {
                            Ok(client_msg) => {
                                handle_client_message(
                                    client_msg,
                                    &mut filter,
                                    &mut paused,
                                    &mut sender,
                                    &aggregator,
                                    use_json,
                                ).await;
                            }
                            Err(e) => {
                                warn!("Invalid binary message: {}", e);
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if sender.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Ignore pong
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!("Client sent close");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!("WebSocket error: {}", e);
                        break;
                    }
                    None => {
                        info!("Client disconnected");
                        break;
                    }
                }
            }
        }
    }

    info!("WebSocket connection closed");
}

/// Handle a message from the client.
async fn handle_client_message<S>(
    msg: ClientMessage,
    filter: &mut SubscriptionFilter,
    paused: &mut bool,
    sender: &mut S,
    aggregator: &Arc<EventAggregator>,
    use_json: bool,
) where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    match msg {
        ClientMessage::Filter(new_filter) => {
            debug!("Client updated filter: {:?}", new_filter);
            *filter = new_filter;
        }
        ClientMessage::Pause => {
            debug!("Client paused");
            *paused = true;
        }
        ClientMessage::Resume => {
            debug!("Client resumed");
            *paused = false;
        }
        ClientMessage::History { from_ms, to_ms } => {
            debug!("Client requested history: {} - {}", from_ms, to_ms);
            let batches = aggregator.ring_buffer().range(from_ms, to_ms);
            for batch in batches {
                if let Some(filtered) = filter.apply(&batch) {
                    let msg = ServerMessage::Batch(filtered);
                    if send_message(sender, &msg, use_json).await.is_err() {
                        break;
                    }
                }
            }
        }
        ClientMessage::Ping => {
            let _ = send_message(
                sender,
                &ServerMessage::Pong {
                    timestamp_ms: current_timestamp_ms(),
                },
                use_json,
            )
            .await;
        }
    }
}

/// Send a server message to the client.
async fn send_message<S>(sender: &mut S, msg: &ServerMessage, use_json: bool) -> Result<(), String>
where
    S: SinkExt<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let message = if use_json {
        Message::Text(serde_json::to_string(msg).map_err(|e| e.to_string())?)
    } else {
        Message::Binary(rmp_serde::to_vec(msg).map_err(|e| e.to_string())?)
    };

    sender.send(message).await.map_err(|e| e.to_string())
}
