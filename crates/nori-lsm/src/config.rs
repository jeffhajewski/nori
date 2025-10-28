use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// ATLL (Adaptive Tiered-Leveled LSM) configuration.
///
/// Based on `lsm_atll_design.yaml` spec (lines 95-134).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ATLLConfig {
    /// Base directory for all LSM data (sst/, manifest/, wal/)
    pub data_dir: PathBuf,

    /// Target level size ratio (default: 10x per level)
    /// Spec name: `fanout_T`
    pub fanout: u8,

    /// Maximum number of levels (default: 7)
    /// Spec name: `levels_max`
    pub max_levels: u8,

    /// Initial number of guard keys for L1 (default: 32)
    /// Spec name: `slot_count_L1`
    pub l1_slot_count: u32,

    /// Slot overprovision factor for skewed distributions (default: 1.2)
    pub slot_overprovision_factor: f32,

    /// Default K values per level (bounded overlap)
    /// Spec name: `K_defaults`
    pub default_k: KDefaults,

    /// K value for hot slots (default: 1, pure leveled)
    /// Spec name: `K_hot`
    pub hot_k: u8,

    /// Fraction of level bytes allocated to slots (default: 0.9)
    pub slot_byte_budget_fraction: f32,

    /// Heat tracking thresholds
    pub heat_thresholds: HeatThresholds,

    /// Tombstone cleanup settings
    pub tombstone: TombstoneConfig,

    /// L0 configuration
    pub l0: L0Config,

    /// Filter (bloom/ribbon) budget and settings
    pub filters: FilterConfig,

    /// Value log configuration (optional, disabled by default)
    pub value_log: ValueLogConfig,

    /// ZNS device support (optional, disabled by default)
    pub zns: ZnsConfig,

    /// I/O and compaction settings
    pub io: IoConfig,

    /// Memtable configuration
    pub memtable: MemtableConfig,

    /// Resource budgets
    pub resources: ResourceConfig,
}

/// Default K values per level (bounded overlap).
/// Spec names: `K_defaults.{L1, L2, L3plus}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KDefaults {
    /// K for L1 (spec: `L1`)
    pub l1: u8,
    /// K for L2 (spec: `L2`)
    pub l2: u8,
    /// K for L3 and beyond (spec: `L3plus`)
    pub l3_plus: u8,
}

/// Heat tracking thresholds for adaptive K selection.
/// Spec names: `heat_thresholds.{H_hot, H_cold, half_life_ops}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatThresholds {
    /// Heat score threshold for "hot" classification (0.0-1.0, default: 0.8)
    /// Spec name: `H_hot`
    pub hot: f32,

    /// Heat score threshold for "cold" classification (0.0-1.0, default: 0.2)
    /// Spec name: `H_cold`
    pub cold: f32,

    /// EWMA half-life in number of operations (default: 100_000)
    pub half_life_ops: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TombstoneConfig {
    /// Tombstone density threshold for cleanup trigger (default: 0.15 = 15%)
    pub density_threshold: f32,

    /// Optional global TTL in seconds (None = no TTL)
    pub ttl_default_sec: Option<u64>,

    /// Enable range tombstones (default: true)
    pub range_tombstones_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct L0Config {
    /// Maximum L0 files before write stall (default: 12)
    pub max_files: usize,

    /// Split L0 flushes on L1 guard boundaries (default: true)
    pub admission_split_on_guards: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterConfig {
    /// Total filter budget in MiB (default: 256)
    pub total_budget_mib: usize,

    /// Preference order for filter types (default: ribbon, bloom, surf)
    pub type_preference_order: Vec<FilterType>,

    /// Target false positive rate for point lookups (default: 0.001)
    pub target_fp_point_lookups: f64,

    /// Use partitioned filters per run (default: true)
    pub partitioned_filters: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FilterType {
    Ribbon,
    Bloom,
    Surf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueLogConfig {
    /// Enable value log for large values (default: false)
    pub enabled: bool,

    /// Segment size in MB (default: 128)
    pub segment_mb: usize,

    /// GC threshold: reclaim when live data < this fraction (default: 0.6)
    pub gc_low_utilization: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZnsConfig {
    /// Enable ZNS device support (default: false)
    pub enabled: bool,

    /// Zone size in MB (default: 256)
    pub zone_size_mb: usize,

    /// Number of zones per slot (default: 2)
    pub zones_per_slot: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoConfig {
    /// Cooperative yield granularity in MB (default: 64)
    pub compaction_slice_mb: usize,

    /// Maximum concurrent background compactions (default: 4)
    pub max_background_compactions: usize,

    /// Optional rate limit in MB/s (None = no limit)
    pub rate_limit_mb_s: Option<usize>,

    /// Background compaction loop interval in seconds (default: 10)
    pub compaction_interval_sec: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemtableConfig {
    /// Memtable size trigger in bytes (default: 64 MiB)
    pub flush_trigger_bytes: usize,

    /// WAL age trigger in seconds (default: 30)
    pub wal_age_trigger_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConfig {
    /// Block cache size in MiB (default: 1024)
    pub block_cache_mib: usize,

    /// Index cache size in MiB (default: 128)
    pub index_cache_mib: usize,

    /// Memtable budget in MiB (default: 512)
    pub memtables_mib: usize,

    /// Filter budget in MiB (default: 256, synced with FilterConfig)
    pub filters_mib: usize,

    /// Soft limit for file descriptors (default: 16384)
    pub file_descriptors_soft_limit: usize,
}

impl Default for ATLLConfig {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("data"),
            fanout: 10,
            max_levels: 7,
            l1_slot_count: 32,
            slot_overprovision_factor: 1.2,
            default_k: KDefaults::default(),
            hot_k: 1,
            slot_byte_budget_fraction: 0.9,
            heat_thresholds: HeatThresholds::default(),
            tombstone: TombstoneConfig::default(),
            l0: L0Config::default(),
            filters: FilterConfig::default(),
            value_log: ValueLogConfig::default(),
            zns: ZnsConfig::default(),
            io: IoConfig::default(),
            memtable: MemtableConfig::default(),
            resources: ResourceConfig::default(),
        }
    }
}

impl Default for KDefaults {
    fn default() -> Self {
        Self {
            l1: 3,
            l2: 3,
            l3_plus: 2,
        }
    }
}

impl Default for HeatThresholds {
    fn default() -> Self {
        Self {
            hot: 0.8,
            cold: 0.2,
            half_life_ops: 100_000,
        }
    }
}

impl Default for TombstoneConfig {
    fn default() -> Self {
        Self {
            density_threshold: 0.15,
            ttl_default_sec: None,
            range_tombstones_enabled: true,
        }
    }
}

impl Default for L0Config {
    fn default() -> Self {
        Self {
            max_files: 12,
            admission_split_on_guards: true,
        }
    }
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            total_budget_mib: 256,
            type_preference_order: vec![FilterType::Ribbon, FilterType::Bloom, FilterType::Surf],
            target_fp_point_lookups: 0.001,
            partitioned_filters: true,
        }
    }
}

impl Default for ValueLogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            segment_mb: 128,
            gc_low_utilization: 0.6,
        }
    }
}

impl Default for ZnsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            zone_size_mb: 256,
            zones_per_slot: 2,
        }
    }
}

impl Default for IoConfig {
    fn default() -> Self {
        Self {
            compaction_slice_mb: 64,
            max_background_compactions: 4,
            rate_limit_mb_s: None,
            compaction_interval_sec: 10,
        }
    }
}

impl Default for MemtableConfig {
    fn default() -> Self {
        Self {
            flush_trigger_bytes: 64 * 1024 * 1024, // 64 MiB
            wal_age_trigger_sec: 30,
        }
    }
}

impl Default for ResourceConfig {
    fn default() -> Self {
        Self {
            block_cache_mib: 1024,
            index_cache_mib: 128,
            memtables_mib: 512,
            filters_mib: 256,
            file_descriptors_soft_limit: 16384,
        }
    }
}

impl ATLLConfig {
    /// Validates the configuration and returns an error if invalid.
    pub fn validate(&self) -> Result<()> {
        // Validate K values
        if self.hot_k > self.default_k.l1 {
            return Err(Error::Config(format!(
                "hot_k ({}) cannot exceed default_k.l1 ({})",
                self.hot_k, self.default_k.l1
            )));
        }

        if self.default_k.l1 == 0 || self.default_k.l2 == 0 || self.default_k.l3_plus == 0 {
            return Err(Error::Config("default_k values must all be > 0".to_string()));
        }

        // Validate heat thresholds
        if self.heat_thresholds.hot < self.heat_thresholds.cold {
            return Err(Error::Config(format!(
                "heat_thresholds.hot ({}) must be >= heat_thresholds.cold ({})",
                self.heat_thresholds.hot, self.heat_thresholds.cold
            )));
        }

        if self.heat_thresholds.hot > 1.0 || self.heat_thresholds.cold < 0.0 {
            return Err(Error::Config(
                "Heat thresholds must be in range [0.0, 1.0]".to_string(),
            ));
        }

        // Validate slot configuration
        if self.l1_slot_count == 0 {
            return Err(Error::Config("l1_slot_count must be > 0".to_string()));
        }

        if self.slot_overprovision_factor < 1.0 {
            return Err(Error::Config(
                "slot_overprovision_factor must be >= 1.0".to_string(),
            ));
        }

        // Validate levels
        if self.max_levels == 0 || self.max_levels > 15 {
            return Err(Error::Config(
                "max_levels must be in range [1, 15]".to_string(),
            ));
        }

        // Validate fanout
        if self.fanout < 2 {
            return Err(Error::Config("fanout must be >= 2".to_string()));
        }

        // Validate L0 configuration
        if self.l0.max_files == 0 {
            return Err(Error::Config("L0 max_files must be > 0".to_string()));
        }

        // Validate tombstone density
        if self.tombstone.density_threshold < 0.0 || self.tombstone.density_threshold > 1.0 {
            return Err(Error::Config(
                "Tombstone density_threshold must be in [0.0, 1.0]".to_string(),
            ));
        }

        // Validate filter budget
        if self.filters.total_budget_mib == 0 {
            return Err(Error::Config("Filter budget must be > 0".to_string()));
        }

        if self.filters.target_fp_point_lookups <= 0.0
            || self.filters.target_fp_point_lookups >= 1.0
        {
            return Err(Error::Config(
                "target_fp_point_lookups must be in (0.0, 1.0)".to_string(),
            ));
        }

        // Validate memtable config
        if self.memtable.flush_trigger_bytes == 0 {
            return Err(Error::Config("Memtable flush_trigger_bytes must be > 0".to_string()));
        }

        // Validate IO config
        if self.io.max_background_compactions == 0 {
            return Err(Error::Config(
                "max_background_compactions must be > 0".to_string(),
            ));
        }

        if self.io.compaction_slice_mb == 0 {
            return Err(Error::Config("compaction_slice_mb must be > 0".to_string()));
        }

        Ok(())
    }

    /// Returns the default K value for the given level.
    pub fn default_k_for_level(&self, level: u8) -> u8 {
        match level {
            1 => self.default_k.l1,
            2 => self.default_k.l2,
            _ => self.default_k.l3_plus,
        }
    }

    /// Returns the target size in bytes for a given level.
    pub fn level_target_bytes(&self, level: u8) -> u64 {
        if level == 0 {
            return 0; // L0 is unbounded
        }

        // L1 target: memtable_size * fanout (~ 640 MB with 64 MB memtable, fanout 10)
        let l1_target = self.memtable.flush_trigger_bytes as u64 * self.fanout as u64;

        // Ln target: L1 * fanout^(n-1)
        l1_target * (self.fanout as u64).pow((level - 1) as u32)
    }

    /// Returns the slot budget in bytes for a given level.
    pub fn slot_budget_bytes(&self, level: u8) -> u64 {
        if level == 0 {
            return 0; // L0 has no slots
        }

        let level_bytes = self.level_target_bytes(level);
        let slot_count = self.slot_count_for_level(level);

        (level_bytes as f64 * self.slot_byte_budget_fraction as f64 / slot_count as f64) as u64
    }

    /// Returns the number of slots for a given level.
    /// Currently static (all levels use same count), but can be made dynamic.
    pub fn slot_count_for_level(&self, level: u8) -> u32 {
        if level == 0 {
            return 0; // L0 has no slots
        }
        self.l1_slot_count // For now, all levels use same slot count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_validates() {
        let config = ATLLConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_k_hot() {
        let mut config = ATLLConfig::default();
        config.hot_k = 10;
        config.default_k.l1 = 3;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_invalid_heat_thresholds() {
        let mut config = ATLLConfig::default();
        config.heat_thresholds.hot = 0.5;
        config.heat_thresholds.cold = 0.8;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_level_target_bytes() {
        let config = ATLLConfig::default();

        // L0 is unbounded
        assert_eq!(config.level_target_bytes(0), 0);

        // L1 = memtable_size * fanout_T = 64MB * 10 = 640MB
        assert_eq!(config.level_target_bytes(1), 64 * 1024 * 1024 * 10);

        // L2 = L1 * fanout_T = 640MB * 10 = 6.4GB
        assert_eq!(config.level_target_bytes(2), 64 * 1024 * 1024 * 100);
    }

    #[test]
    fn test_slot_budget() {
        let config = ATLLConfig::default();

        // L1 target: 640MB, 32 slots, 0.9 budget fraction
        // slot_budget = 640MB * 0.9 / 32 = 18874368 bytes (~18MB)
        let budget = config.slot_budget_bytes(1);

        // Expected value: 18874368 bytes, allowing for rounding difference of ±1
        assert!(
            budget >= 18874367 && budget <= 18874369,
            "Expected budget ~18874368 bytes, got {} bytes",
            budget
        );
    }

    #[test]
    fn test_default_k_for_level() {
        let config = ATLLConfig::default();
        assert_eq!(config.default_k_for_level(1), 3);
        assert_eq!(config.default_k_for_level(2), 3);
        assert_eq!(config.default_k_for_level(3), 2);
        assert_eq!(config.default_k_for_level(7), 2);
    }
}
