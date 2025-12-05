//! norikv-vizd: Event aggregator and WebSocket bridge for NoriKV dashboard.
//!
//! This daemon receives VizEvents from cluster nodes, batches them in 50ms
//! coalesce windows, stores in a ring buffer for playback, and streams to
//! dashboard clients via WebSocket.
//!
//! # Endpoints
//!
//! - `GET /ws/events` - WebSocket for real-time event streaming
//! - `GET /api/history` - Query historical events
//! - `GET /api/stats` - Aggregator statistics
//! - `GET /health` - Health check

mod aggregator;
mod event;
mod ringbuffer;
mod subscription;
mod ws;

use aggregator::{AggregatorStats, EventAggregator};
use axum::{
    extract::{Query, State},
    response::Json,
    routing::get,
    Router,
};
use event::EventBatch;
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, sync::Arc};
use tokio::signal;
use tower_http::cors::{Any, CorsLayer};
use tracing::{info, Level};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Server configuration.
#[derive(Debug, Clone)]
struct Config {
    /// HTTP/WebSocket listen address.
    listen_addr: SocketAddr,
    /// Enable CORS for dashboard development.
    enable_cors: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 9090)),
            enable_cors: true,
        }
    }
}

impl Config {
    fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(addr) = std::env::var("VIZD_LISTEN_ADDR") {
            if let Ok(parsed) = addr.parse() {
                config.listen_addr = parsed;
            }
        }

        if let Ok(cors) = std::env::var("VIZD_ENABLE_CORS") {
            config.enable_cors = cors == "1" || cors.eq_ignore_ascii_case("true");
        }

        config
    }
}

/// Application state is just the aggregator.

#[tokio::main]
async fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(Level::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let config = Config::from_env();
    info!(
        version = env!("CARGO_PKG_VERSION"),
        listen = %config.listen_addr,
        cors = config.enable_cors,
        "Starting norikv-vizd"
    );

    // Create aggregator
    let aggregator = EventAggregator::new();

    // Start coalesce loop
    let coalesce_aggregator = Arc::clone(&aggregator);
    tokio::spawn(async move {
        coalesce_aggregator.run_coalesce_loop().await;
    });

    // Build router
    let mut app = Router::new()
        .route("/ws/events", get(ws::ws_handler))
        .route("/api/history", get(history_handler))
        .route("/api/stats", get(stats_handler))
        .route("/health", get(health_handler))
        .with_state(aggregator);

    // Add CORS for dashboard development
    if config.enable_cors {
        app = app.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );
    }

    // Start server
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .expect("Failed to bind to address");

    info!("Listening on {}", config.listen_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server error");

    info!("Shutdown complete");
}

/// Query parameters for history endpoint.
#[derive(Debug, Deserialize)]
struct HistoryParams {
    /// Start timestamp (ms since epoch).
    from: u64,
    /// End timestamp (ms since epoch).
    to: u64,
    /// Filter by node IDs (comma-separated).
    nodes: Option<String>,
    /// Filter by event types (comma-separated).
    types: Option<String>,
    /// Filter by shard IDs (comma-separated).
    shards: Option<String>,
}

/// History API response.
#[derive(Serialize)]
struct HistoryResponse {
    batches: Vec<EventBatch>,
    total_events: usize,
}

/// GET /api/history - Query historical events.
async fn history_handler(
    Query(params): Query<HistoryParams>,
    State(aggregator): State<Arc<EventAggregator>>,
) -> Json<HistoryResponse> {
    let filter = subscription::SubscriptionFilter::from_query_params(
        params.nodes.as_deref(),
        params.types.as_deref(),
        params.shards.as_deref(),
    );

    let batches: Vec<_> = aggregator
        .ring_buffer()
        .range(params.from, params.to)
        .into_iter()
        .filter_map(|b| filter.apply(&b))
        .collect();

    let total_events = batches.iter().map(|b| b.events.len()).sum();

    Json(HistoryResponse {
        batches,
        total_events,
    })
}

/// GET /api/stats - Aggregator statistics.
async fn stats_handler(State(aggregator): State<Arc<EventAggregator>>) -> Json<AggregatorStats> {
    Json(aggregator.stats())
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

/// GET /health - Health check.
async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Graceful shutdown signal handler.
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert_eq!(config.listen_addr.port(), 9090);
        assert!(config.enable_cors);
    }
}
