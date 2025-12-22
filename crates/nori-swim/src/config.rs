//! SWIM protocol configuration.
//!
//! Tunable parameters for probe intervals, timeouts, and gossip behavior.

use std::time::Duration;

/// SWIM protocol configuration.
#[derive(Debug, Clone)]
pub struct SwimConfig {
    /// Interval between probes to random members.
    /// Default: 1000ms
    pub probe_interval: Duration,

    /// Timeout for ping responses (direct and indirect).
    /// Default: 500ms
    pub ping_timeout: Duration,

    /// Time a member stays in Suspect state before being marked Failed.
    /// Default: 5000ms
    pub suspicion_timeout: Duration,

    /// Number of members to ask for indirect probes when direct ping fails.
    /// Default: 3
    pub indirect_fanout: usize,

    /// Maximum gossip entries to piggyback on each message.
    /// Default: 10
    pub max_gossip_per_msg: usize,

    /// Capacity of the event broadcast channel.
    /// Default: 100
    pub event_channel_capacity: usize,
}

impl Default for SwimConfig {
    fn default() -> Self {
        Self {
            probe_interval: Duration::from_millis(1000),
            ping_timeout: Duration::from_millis(500),
            suspicion_timeout: Duration::from_millis(5000),
            indirect_fanout: 3,
            max_gossip_per_msg: 10,
            event_channel_capacity: 100,
        }
    }
}

impl SwimConfig {
    /// Create a new config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the probe interval.
    pub fn with_probe_interval(mut self, interval: Duration) -> Self {
        self.probe_interval = interval;
        self
    }

    /// Set the ping timeout.
    pub fn with_ping_timeout(mut self, timeout: Duration) -> Self {
        self.ping_timeout = timeout;
        self
    }

    /// Set the suspicion timeout.
    pub fn with_suspicion_timeout(mut self, timeout: Duration) -> Self {
        self.suspicion_timeout = timeout;
        self
    }

    /// Set the indirect probe fanout.
    pub fn with_indirect_fanout(mut self, fanout: usize) -> Self {
        self.indirect_fanout = fanout;
        self
    }

    /// Validate configuration values.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.probe_interval.is_zero() {
            return Err(ConfigError::InvalidValue("probe_interval must be > 0".into()));
        }
        if self.ping_timeout.is_zero() {
            return Err(ConfigError::InvalidValue("ping_timeout must be > 0".into()));
        }
        if self.suspicion_timeout.is_zero() {
            return Err(ConfigError::InvalidValue(
                "suspicion_timeout must be > 0".into(),
            ));
        }
        if self.ping_timeout >= self.probe_interval {
            return Err(ConfigError::InvalidValue(
                "ping_timeout must be < probe_interval".into(),
            ));
        }
        if self.indirect_fanout == 0 {
            return Err(ConfigError::InvalidValue(
                "indirect_fanout must be > 0".into(),
            ));
        }
        Ok(())
    }
}

/// Configuration errors.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Invalid configuration value: {0}")]
    InvalidValue(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SwimConfig::default();
        assert_eq!(config.probe_interval, Duration::from_millis(1000));
        assert_eq!(config.ping_timeout, Duration::from_millis(500));
        assert_eq!(config.suspicion_timeout, Duration::from_millis(5000));
        assert_eq!(config.indirect_fanout, 3);
        assert_eq!(config.max_gossip_per_msg, 10);
    }

    #[test]
    fn test_config_validation() {
        let config = SwimConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_probe_interval() {
        let config = SwimConfig::default().with_probe_interval(Duration::ZERO);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_ping_timeout_must_be_less_than_probe_interval() {
        let config = SwimConfig::default()
            .with_probe_interval(Duration::from_millis(100))
            .with_ping_timeout(Duration::from_millis(200));
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_builder_pattern() {
        let config = SwimConfig::new()
            .with_probe_interval(Duration::from_millis(500))
            .with_ping_timeout(Duration::from_millis(200))
            .with_suspicion_timeout(Duration::from_millis(2000))
            .with_indirect_fanout(5);

        assert_eq!(config.probe_interval, Duration::from_millis(500));
        assert_eq!(config.ping_timeout, Duration::from_millis(200));
        assert_eq!(config.suspicion_timeout, Duration::from_millis(2000));
        assert_eq!(config.indirect_fanout, 5);
    }
}
