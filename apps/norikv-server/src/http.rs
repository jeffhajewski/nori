//! HTTP REST API server.
//!
//! Exposes health checks, metrics, and KV operations via HTTP REST API.

use crate::health::HealthChecker;
use crate::metrics::PrometheusMeter;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{delete, get, put},
    Json, Router,
};
use norikv_transport_grpc::KvBackend;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// HTTP server state.
#[derive(Clone)]
pub struct HttpServerState {
    health_checker: Arc<HealthChecker>,
    meter: Arc<PrometheusMeter>,
    kv_backend: Arc<dyn KvBackend>,
}

/// HTTP server for REST API endpoints.
///
/// Provides:
/// - GET /health - Comprehensive health status (all shards)
/// - GET /health/quick - Fast health check (shard 0 only)
/// - GET /healthz - Kubernetes liveness probe (alias for /health/quick)
/// - GET /readyz - Kubernetes readiness probe (alias for /health)
/// - GET /metrics - Prometheus metrics
/// - PUT /kv/{key} - Store a key-value pair
/// - GET /kv/{key} - Retrieve a value by key
/// - DELETE /kv/{key} - Delete a key
pub struct HttpServer {
    addr: SocketAddr,
    state: HttpServerState,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    server_handle: Option<JoinHandle<Result<(), std::io::Error>>>,
}

impl HttpServer {
    /// Create a new HTTP server.
    pub fn new(
        addr: SocketAddr,
        health_checker: Arc<HealthChecker>,
        meter: Arc<PrometheusMeter>,
        kv_backend: Arc<dyn KvBackend>,
    ) -> Self {
        Self {
            addr,
            state: HttpServerState {
                health_checker,
                meter,
                kv_backend,
            },
            shutdown_tx: None,
            server_handle: None,
        }
    }

    /// Start the HTTP server.
    pub async fn start(&mut self) -> Result<(), HttpServerError> {
        tracing::info!("Starting HTTP server on {}", self.addr);

        // Build router with all endpoints
        let app = Router::new()
            // Health endpoints
            .route("/health", get(health_handler))
            .route("/health/quick", get(health_quick_handler))
            // Kubernetes-standard health probe endpoints
            .route("/healthz", get(health_quick_handler)) // Liveness probe
            .route("/readyz", get(health_handler))         // Readiness probe
            // Metrics endpoint
            .route("/metrics", get(metrics_handler))
            // KV REST API endpoints
            .route("/kv/:key", put(kv_put_handler))
            .route("/kv/:key", get(kv_get_handler))
            .route("/kv/:key", delete(kv_delete_handler))
            .with_state(self.state.clone());

        // Create shutdown signal
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        self.shutdown_tx = Some(shutdown_tx);

        // Start server with graceful shutdown
        let listener = tokio::net::TcpListener::bind(self.addr)
            .await
            .map_err(|e| HttpServerError::Startup(format!("Failed to bind: {}", e)))?;

        let server_handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    shutdown_rx.await.ok();
                })
                .await
        });

        self.server_handle = Some(server_handle);

        tracing::info!("HTTP server started successfully");
        Ok(())
    }

    /// Shutdown the HTTP server gracefully.
    pub async fn shutdown(mut self) -> Result<(), HttpServerError> {
        tracing::info!("Shutting down HTTP server");

        // Send shutdown signal
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }

        // Wait for server to finish
        if let Some(handle) = self.server_handle.take() {
            handle
                .await
                .map_err(|e| HttpServerError::Shutdown(format!("Join error: {}", e)))?
                .map_err(|e| HttpServerError::Shutdown(format!("Server error: {}", e)))?;
        }

        tracing::info!("HTTP server shutdown complete");
        Ok(())
    }
}

/// Health check endpoint handler.
///
/// GET /health
///
/// Returns comprehensive health status for all shards.
async fn health_handler(
    State(state): State<HttpServerState>,
) -> Result<Json<crate::health::ServerHealthStatus>, AppError> {
    let status = state.health_checker.check().await;
    Ok(Json(status))
}

/// Quick health check endpoint handler.
///
/// GET /health/quick
///
/// Returns 200 OK if server is healthy (shard 0 responsive), 503 otherwise.
/// Useful for load balancer health checks.
async fn health_quick_handler(State(state): State<HttpServerState>) -> Response {
    let is_healthy = state.health_checker.check_quick().await;

    if is_healthy {
        (StatusCode::OK, "OK").into_response()
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "UNAVAILABLE").into_response()
    }
}

/// Metrics endpoint handler.
///
/// GET /metrics
///
/// Returns Prometheus-formatted metrics.
async fn metrics_handler(State(state): State<HttpServerState>) -> Response {
    let metrics = state.meter.export();

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4")],
        metrics,
    )
        .into_response()
}

/// Query parameters for GET /kv/{key}
#[derive(Debug, Deserialize)]
struct GetQueryParams {
    /// Read consistency level: lease (default), linearizable, or stale_ok (future use)
    #[serde(default)]
    #[allow(dead_code)]
    consistency: Option<String>,
}

/// KV PUT endpoint handler.
///
/// PUT /kv/{key}
///
/// Request body: raw bytes (the value)
/// Query params: ?ttl_ms=<milliseconds> (optional)
///
/// Returns 200 OK with version information on success.
async fn kv_put_handler(
    State(state): State<HttpServerState>,
    Path(key): Path<String>,
    body: Bytes,
) -> Result<Response, AppError> {
    let key_bytes = bytes::Bytes::from(key.into_bytes());
    let value_bytes = bytes::Bytes::from(body.to_vec());

    tracing::debug!("HTTP PUT /kv/<key> key_len={} value_len={}", key_bytes.len(), value_bytes.len());

    // Call backend (no TTL support in this basic version)
    match state.kv_backend.put(key_bytes, value_bytes, None).await {
        Ok(index) => {
            // Get version information
            let term = state.kv_backend.current_term();
            let response = serde_json::json!({
                "version": {
                    "term": term.as_u64(),
                    "index": index.0,
                }
            });

            Ok((StatusCode::OK, Json(response)).into_response())
        }
        Err(nori_raft::RaftError::NotLeader { leader }) => {
            // Return 503 with leader hint
            let leader_hint = leader.map(|l| l.to_string()).unwrap_or_default();
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                [("X-Leader-Hint", leader_hint.as_str())],
                "NOT_LEADER",
            )
                .into_response())
        }
        Err(e) => {
            tracing::error!("PUT failed: {:?}", e);
            Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("PUT failed: {:?}", e)).into_response())
        }
    }
}

/// KV GET endpoint handler.
///
/// GET /kv/{key}?consistency=lease|linearizable|stale_ok
///
/// Returns 200 OK with value if found, 404 if not found.
async fn kv_get_handler(
    State(state): State<HttpServerState>,
    Path(key): Path<String>,
    Query(_params): Query<GetQueryParams>,
) -> Result<Response, AppError> {
    let key_bytes = key.as_bytes();

    tracing::debug!("HTTP GET /kv/<key> key_len={}", key_bytes.len());

    // Note: consistency parameter is parsed but not yet used
    // Full implementation would call different backend methods based on consistency level

    match state.kv_backend.get(key_bytes).await {
        Ok(Some((value, _term, _log_index))) => {
            // Return value as raw bytes (HTTP API doesn't expose version yet)
            Ok((StatusCode::OK, value.to_vec()).into_response())
        }
        Ok(None) => {
            // Key not found
            Ok((StatusCode::NOT_FOUND, "Key not found").into_response())
        }
        Err(nori_raft::RaftError::NotLeader { leader }) => {
            // Return 503 with leader hint
            let leader_hint = leader.map(|l| l.to_string()).unwrap_or_default();
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                [("X-Leader-Hint", leader_hint.as_str())],
                "NOT_LEADER",
            )
                .into_response())
        }
        Err(e) => {
            tracing::error!("GET failed: {:?}", e);
            Ok((StatusCode::INTERNAL_SERVER_ERROR, format!("GET failed: {:?}", e)).into_response())
        }
    }
}

/// KV DELETE endpoint handler.
///
/// DELETE /kv/{key}
///
/// Returns 200 OK on success (whether key existed or not).
async fn kv_delete_handler(
    State(state): State<HttpServerState>,
    Path(key): Path<String>,
) -> Result<Response, AppError> {
    let key_bytes = bytes::Bytes::from(key.into_bytes());

    tracing::debug!("HTTP DELETE /kv/<key> key_len={}", key_bytes.len());

    match state.kv_backend.delete(key_bytes).await {
        Ok(_index) => {
            // Success
            Ok((StatusCode::OK, "Deleted").into_response())
        }
        Err(nori_raft::RaftError::NotLeader { leader }) => {
            // Return 503 with leader hint
            let leader_hint = leader.map(|l| l.to_string()).unwrap_or_default();
            Ok((
                StatusCode::SERVICE_UNAVAILABLE,
                [("X-Leader-Hint", leader_hint.as_str())],
                "NOT_LEADER",
            )
                .into_response())
        }
        Err(e) => {
            tracing::error!("DELETE failed: {:?}", e);
            Ok((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DELETE failed: {:?}", e),
            )
                .into_response())
        }
    }
}

/// HTTP server errors.
#[derive(Debug, thiserror::Error)]
pub enum HttpServerError {
    #[error("Startup error: {0}")]
    Startup(String),

    #[error("Shutdown error: {0}")]
    Shutdown(String),
}

/// Application error wrapper for handlers.
struct AppError(anyhow::Error);

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        tracing::error!("Handler error: {:?}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Internal error: {}", self.0),
        )
            .into_response()
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
