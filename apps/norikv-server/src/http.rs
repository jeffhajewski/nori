//! HTTP REST API server.
//!
//! Exposes health checks and metrics via HTTP for monitoring tools.

use crate::health::HealthChecker;
use crate::metrics::PrometheusMeter;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::task::JoinHandle;

/// HTTP server state.
#[derive(Clone)]
pub struct HttpServerState {
    health_checker: Arc<HealthChecker>,
    meter: Arc<PrometheusMeter>,
}

/// HTTP server for REST API endpoints.
///
/// Provides:
/// - GET /health - Comprehensive health status (all shards)
/// - GET /health/quick - Fast health check (shard 0 only)
/// - GET /metrics - Prometheus metrics (Phase 5.3)
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
    ) -> Self {
        Self {
            addr,
            state: HttpServerState {
                health_checker,
                meter,
            },
            shutdown_tx: None,
            server_handle: None,
        }
    }

    /// Start the HTTP server.
    pub async fn start(&mut self) -> Result<(), HttpServerError> {
        tracing::info!("Starting HTTP server on {}", self.addr);

        // Build router with health endpoints
        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/health/quick", get(health_quick_handler))
            // Metrics endpoint placeholder (Phase 5.3)
            .route("/metrics", get(metrics_handler))
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
