//! Hardware Certification & Edge Deployment Module
//!
//! v0.38.0 — Hardware Certification & Edge Deployment Guide
//!
//! Provides:
//! - Platform detection (CPU arch, RAM, storage type, board model)
//! - Hardware compatibility matrix (min specs per license tier)
//! - Performance benchmark engine (tag write, compression, polling throughput)
//! - SD card / flash storage write optimization verification
//! - Edge deployment guide generation (Raspberry Pi, Advantech, Siemens IPC)

pub mod benchmark;
pub mod compat_matrix;
pub mod edge_deploy;
pub mod platform;

pub use platform::*;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Full hardware certification report for the current system.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HardwareCertReport {
    /// Detected platform information
    pub platform: PlatformInfo,
    /// Compatibility assessment against all tiers
    pub compatibility: TierCompatibility,
    /// Storage health assessment (SD card, SSD, HDD)
    pub storage_assessment: StorageAssessment,
    /// Overall certification status
    pub certified: bool,
    /// Certification level (which tiers are supported)
    pub certified_tiers: Vec<String>,
    /// Warnings about hardware limitations
    pub warnings: Vec<String>,
    /// Recommendations for optimization
    pub recommendations: Vec<String>,
}

/// Compatibility assessment across the hardware sizing profiles (named after the
/// former plans, they only describe how much hardware a deployment needs).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TierCompatibility {
    /// Engineering compatibility
    pub engineering: TierAssessment,
    /// Starter profile compatibility
    pub starter: TierAssessment,
    /// Professional profile compatibility
    pub professional: TierAssessment,
    /// Enterprise profile compatibility
    pub enterprise: TierAssessment,
}

/// Assessment result for a single tier.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TierAssessment {
    /// Whether the hardware meets minimum requirements
    pub compatible: bool,
    /// Maximum recommended tag count at this tier
    pub max_tags: u32,
    /// Maximum recommended polling rate (Hz)
    pub max_poll_rate: u32,
    /// Whether compression is recommended at this tier
    pub compression_recommended: bool,
    /// Any limitations for this tier on this hardware
    pub limitations: Vec<String>,
}

/// Storage health and type assessment.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StorageAssessment {
    /// Storage type detected (sd_card, ssd, hdd, emmc, unknown)
    pub storage_type: String,
    /// Whether write optimization is critical for this storage type
    pub write_optimization_critical: bool,
    /// Estimated write endurance (years at 10K tags @ 1Hz)
    pub estimated_endurance_years: f64,
    /// Whether batch write optimization is active
    pub batch_writes_active: bool,
    /// Current write amplification factor (lower is better, 1.0 = ideal)
    pub write_amplification: f64,
    /// Recommendations for storage longevity
    pub storage_recommendations: Vec<String>,
}

/// Benchmark results for the current hardware.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchmarkResults {
    /// Platform info (snapshot at benchmark time)
    pub platform: PlatformInfo,
    /// Tag write throughput (writes/second)
    pub tag_write_throughput: f64,
    /// Batch insert throughput (rows/second)
    pub batch_insert_throughput: f64,
    /// Compression throughput (rows/second for Gorilla encoding)
    pub compression_throughput: f64,
    /// Query throughput (reads/second for 10K rows)
    pub query_throughput: f64,
    /// Memory usage during benchmark (MB)
    pub peak_memory_mb: f64,
    /// Total benchmark duration (seconds)
    pub duration_secs: f64,
    /// Recommended maximum tag count based on results
    pub recommended_max_tags: u32,
    /// Recommended polling rate (Hz) based on results
    pub recommended_poll_rate: u32,
    /// SD card write amplification factor (if applicable)
    pub write_amplification: f64,
    /// Individual benchmark details
    pub details: Vec<BenchmarkDetail>,
}

/// Individual benchmark measurement.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BenchmarkDetail {
    /// Benchmark name
    pub name: String,
    /// Operations performed
    pub operations: u64,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Throughput (operations/second)
    pub throughput: f64,
    /// Whether this benchmark passed the minimum threshold
    pub passed: bool,
}

/// Hardware compatibility matrix entry (for docs/API).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HardwareCompatibilityEntry {
    /// Hardware platform name
    pub platform: String,
    /// CPU description
    pub cpu: String,
    /// RAM in MB
    pub ram_mb: u32,
    /// Storage type
    pub storage: String,
    /// Supported tiers
    pub supported_tiers: Vec<String>,
    /// Maximum recommended tags
    pub max_tags: u32,
    /// Maximum recommended polling rate (Hz)
    pub max_poll_rate: u32,
    /// Notes (e.g., "SD card optimization required")
    pub notes: String,
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hardware_cert_report_serialization() {
        let report = HardwareCertReport {
            platform: PlatformInfo::default(),
            compatibility: TierCompatibility {
                engineering: TierAssessment {
                    compatible: true,
                    max_tags: 50,
                    max_poll_rate: 1,
                    compression_recommended: false,
                    limitations: vec![],
                },
                starter: TierAssessment {
                    compatible: true,
                    max_tags: 500,
                    max_poll_rate: 1,
                    compression_recommended: true,
                    limitations: vec!["SD card: enable compression to extend card life".into()],
                },
                professional: TierAssessment {
                    compatible: false,
                    max_tags: 0,
                    max_poll_rate: 0,
                    compression_recommended: false,
                    limitations: vec!["Insufficient RAM for multi-site features".into()],
                },
                enterprise: TierAssessment {
                    compatible: false,
                    max_tags: 0,
                    max_poll_rate: 0,
                    compression_recommended: false,
                    limitations: vec!["Insufficient RAM".into()],
                },
            },
            storage_assessment: StorageAssessment {
                storage_type: "sd_card".into(),
                write_optimization_critical: true,
                estimated_endurance_years: 3.5,
                batch_writes_active: true,
                write_amplification: 1.2,
                storage_recommendations: vec!["Enable compression to reduce SD card writes".into()],
            },
            certified: true,
            certified_tiers: vec!["engineering".into(), "starter".into()],
            warnings: vec![],
            recommendations: vec!["Consider SSD upgrade for Professional tier".into()],
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["certified"], true);
        assert_eq!(parsed["certified_tiers"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_benchmark_results_serialization() {
        let results = BenchmarkResults {
            platform: PlatformInfo::default(),
            tag_write_throughput: 50000.0,
            batch_insert_throughput: 100000.0,
            compression_throughput: 200000.0,
            query_throughput: 80000.0,
            peak_memory_mb: 45.0,
            duration_secs: 30.0,
            recommended_max_tags: 10000,
            recommended_poll_rate: 1,
            write_amplification: 1.1,
            details: vec![BenchmarkDetail {
                name: "tag_write".into(),
                operations: 50000,
                duration_ms: 1000,
                throughput: 50000.0,
                passed: true,
            }],
        };
        let json = serde_json::to_string(&results).unwrap();
        let back: BenchmarkResults = serde_json::from_str(&json).unwrap();
        assert_eq!(back.recommended_max_tags, 10000);
    }

    #[test]
    fn test_compatibility_entry() {
        let entry = HardwareCompatibilityEntry {
            platform: "Raspberry Pi 5 (8GB)".into(),
            cpu: "Broadcom BCM2712 (4-core ARM Cortex-A76 @ 2.4GHz)".into(),
            ram_mb: 8192,
            storage: "MicroSD (A2 recommended) or NVMe HAT".into(),
            supported_tiers: vec!["engineering".into(), "starter".into(), "professional".into()],
            max_tags: 5000,
            max_poll_rate: 1,
            notes: "Use A2-rated SD card or NVMe HAT for best performance. Enable compression for long-term storage.".into(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: HardwareCompatibilityEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ram_mb, 8192);
        assert_eq!(back.supported_tiers.len(), 3);
    }
}
