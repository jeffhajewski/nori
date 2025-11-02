//! Server configuration.
//!
//! Loads and validates configuration from YAML files or environment variables.
//! See context/60_ops.yaml for schema documentation.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Server configuration.
///
/// Example YAML:
/// ```yaml
/// node_id: "node1"
/// rpc_addr: "0.0.0.0:7447"
/// data_dir: "/var/lib/norikv"
/// cluster:
///   seed_nodes: ["10.0.1.10:7447", "10.0.1.11:7447"]
///   total_shards: 1024
///   replication_factor: 3
/// telemetry:
///   prometheus:
///     enabled: true
///     port: 9090
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Unique node identifier
    pub node_id: String,

    /// RPC listen address (gRPC + HTTP)
    #[serde(default = "default_rpc_addr")]
    pub rpc_addr: String,

    /// Data directory for LSM and Raft persistence
    pub data_dir: PathBuf,

    /// Cluster configuration
    #[serde(default)]
    pub cluster: ClusterConfig,

    /// Telemetry configuration
    #[serde(default)]
    pub telemetry: TelemetryConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Seed nodes for SWIM membership bootstrap
    #[serde(default)]
    pub seed_nodes: Vec<String>,

    /// Total number of virtual shards
    #[serde(default = "default_total_shards")]
    pub total_shards: u32,

    /// Replication factor
    #[serde(default = "default_replication_factor")]
    pub replication_factor: usize,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            seed_nodes: Vec::new(),
            total_shards: default_total_shards(),
            replication_factor: default_replication_factor(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Prometheus configuration
    #[serde(default)]
    pub prometheus: PrometheusConfig,

    /// OTLP configuration
    #[serde(default)]
    pub otlp: OtlpConfig,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            prometheus: PrometheusConfig::default(),
            otlp: OtlpConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrometheusConfig {
    /// Enable Prometheus metrics endpoint
    #[serde(default)]
    pub enabled: bool,

    /// Prometheus metrics port
    #[serde(default = "default_prometheus_port")]
    pub port: u16,
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: default_prometheus_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OtlpConfig {
    /// Enable OTLP exporter
    #[serde(default)]
    pub enabled: bool,

    /// OTLP endpoint
    pub endpoint: Option<String>,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
        }
    }
}

fn default_rpc_addr() -> String {
    "0.0.0.0:7447".to_string()
}

fn default_total_shards() -> u32 {
    1024
}

fn default_replication_factor() -> usize {
    3
}

fn default_prometheus_port() -> u16 {
    9090
}

impl ServerConfig {
    /// Load configuration from a YAML file.
    pub fn load_from_file(path: &str) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::IoError(format!("Failed to read config file: {}", e)))?;

        let config: ServerConfig = serde_yaml::from_str(&content)
            .map_err(|e| ConfigError::ParseError(format!("Failed to parse YAML: {}", e)))?;

        config.validate()?;
        Ok(config)
    }

    /// Load configuration from environment variables.
    ///
    /// Supported variables:
    /// - NORIKV_NODE_ID
    /// - NORIKV_RPC_ADDR
    /// - NORIKV_DATA_DIR
    /// - NORIKV_SEED_NODES (comma-separated)
    pub fn load_from_env() -> Result<Self, ConfigError> {
        let node_id = std::env::var("NORIKV_NODE_ID")
            .map_err(|_| ConfigError::MissingField("NORIKV_NODE_ID".to_string()))?;

        let rpc_addr = std::env::var("NORIKV_RPC_ADDR").unwrap_or_else(|_| default_rpc_addr());

        let data_dir = std::env::var("NORIKV_DATA_DIR")
            .map_err(|_| ConfigError::MissingField("NORIKV_DATA_DIR".to_string()))?;

        let seed_nodes = std::env::var("NORIKV_SEED_NODES")
            .ok()
            .map(|s| s.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default();

        let config = ServerConfig {
            node_id,
            rpc_addr,
            data_dir: PathBuf::from(data_dir),
            cluster: ClusterConfig {
                seed_nodes,
                total_shards: default_total_shards(),
                replication_factor: default_replication_factor(),
            },
            telemetry: TelemetryConfig::default(),
        };

        config.validate()?;
        Ok(config)
    }

    /// Validate configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate node_id
        if self.node_id.is_empty() {
            return Err(ConfigError::InvalidField(
                "node_id cannot be empty".to_string(),
            ));
        }

        // Validate rpc_addr
        self.rpc_addr
            .parse::<SocketAddr>()
            .map_err(|e| ConfigError::InvalidField(format!("Invalid rpc_addr: {}", e)))?;

        // Validate data_dir exists or can be created
        if !self.data_dir.exists() {
            std::fs::create_dir_all(&self.data_dir).map_err(|e| {
                ConfigError::InvalidField(format!("Cannot create data_dir: {}", e))
            })?;
        }

        // Validate data_dir is writable
        if self.data_dir.exists() && !self.data_dir.is_dir() {
            return Err(ConfigError::InvalidField(
                "data_dir exists but is not a directory".to_string(),
            ));
        }

        // Validate cluster config
        if self.cluster.total_shards == 0 {
            return Err(ConfigError::InvalidField(
                "total_shards must be > 0".to_string(),
            ));
        }

        if self.cluster.replication_factor == 0 {
            return Err(ConfigError::InvalidField(
                "replication_factor must be > 0".to_string(),
            ));
        }

        // Validate seed nodes
        for seed in &self.cluster.seed_nodes {
            seed.parse::<SocketAddr>().map_err(|e| {
                ConfigError::InvalidField(format!("Invalid seed node address {}: {}", seed, e))
            })?;
        }

        Ok(())
    }

    /// Get the Raft data directory.
    pub fn raft_dir(&self) -> PathBuf {
        self.data_dir.join("raft")
    }

    /// Get the LSM data directory.
    pub fn lsm_dir(&self) -> PathBuf {
        self.data_dir.join("lsm")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("I/O error: {0}")]
    IoError(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid field: {0}")]
    InvalidField(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ServerConfig {
            node_id: "test_node".to_string(),
            rpc_addr: default_rpc_addr(),
            data_dir: PathBuf::from("/tmp/norikv-test"),
            cluster: ClusterConfig::default(),
            telemetry: TelemetryConfig::default(),
        };

        assert!(config.validate().is_ok());
        assert_eq!(config.cluster.total_shards, 1024);
        assert_eq!(config.cluster.replication_factor, 3);
    }

    #[test]
    fn test_invalid_rpc_addr() {
        let config = ServerConfig {
            node_id: "test_node".to_string(),
            rpc_addr: "invalid_addr".to_string(),
            data_dir: PathBuf::from("/tmp/norikv-test"),
            cluster: ClusterConfig::default(),
            telemetry: TelemetryConfig::default(),
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_empty_node_id() {
        let config = ServerConfig {
            node_id: "".to_string(),
            rpc_addr: default_rpc_addr(),
            data_dir: PathBuf::from("/tmp/norikv-test"),
            cluster: ClusterConfig::default(),
            telemetry: TelemetryConfig::default(),
        };

        assert!(config.validate().is_err());
    }
}
