mod config;
mod node;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Parse CLI args for config file path
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "norikv.yaml".to_string());

    tracing::info!("Loading configuration from: {}", config_path);

    // Load configuration (try file first, fall back to env)
    let config = if std::path::Path::new(&config_path).exists() {
        config::ServerConfig::load_from_file(&config_path)?
    } else {
        tracing::warn!("Config file not found, loading from environment variables");
        config::ServerConfig::load_from_env()?
    };

    tracing::info!("Starting NoriKV node: {}", config.node_id);
    tracing::info!("RPC address: {}", config.rpc_addr);
    tracing::info!("Data directory: {}", config.data_dir.display());

    // Create and start node
    let node = node::Node::new(config).await?;
    node.start().await?;

    tracing::info!("NoriKV server is ready");

    // Wait for shutdown signal (SIGINT/SIGTERM)
    tokio::signal::ctrl_c().await?;

    tracing::info!("Received shutdown signal, gracefully shutting down...");

    // Graceful shutdown
    node.shutdown().await?;

    tracing::info!("Shutdown complete");
    Ok(())
}
