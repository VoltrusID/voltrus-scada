//! EDR (Endpoint Detection & Response) Compatibility Module
//!
//! v0.36.0 — Windows Defender OT Compatibility
//!
//! Ensures Voltrus runs cleanly on Windows with Defender/EDR without
//! false positives or performance issues. Provides:
//! - Detection of running EDR/antivirus tools
//! - Recommended exclusion paths for the current installation
//! - File I/O pattern optimizations to minimize EDR scanning overhead
//! - Deployment guidance for IT teams deploying to OT networks

pub mod detect;
pub mod exclusions;
pub mod io_optimize;

pub use detect::*;
pub use exclusions::*;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Summary of EDR compatibility status for the system.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EdrCompatReport {
    /// Detected EDR/antivirus products on this system
    pub detected_products: Vec<EdrProduct>,
    /// Recommended exclusion paths for this installation
    pub recommended_exclusions: Vec<EdrExclusion>,
    /// File I/O optimization status
    pub io_optimization: IoOptimizationStatus,
    /// Whether the system is considered EDR-compatible
    pub compatible: bool,
    /// Warnings or issues detected
    pub warnings: Vec<String>,
    /// Recommendations for IT deployment
    pub recommendations: Vec<String>,
}

/// A detected EDR/antivirus product.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EdrProduct {
    /// Product name
    pub name: String,
    /// Vendor
    pub vendor: String,
    /// Whether the product is currently active (running)
    pub active: bool,
    /// Known compatibility notes (if any)
    pub notes: Option<String>,
    /// Product category (antivirus, edr, xdr, etc.)
    pub category: String,
}

/// A recommended exclusion path.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EdrExclusion {
    /// The path to exclude
    pub path: String,
    /// Type of exclusion (directory, file, process, extension)
    pub exclusion_type: String,
    /// Reason for this exclusion
    pub reason: String,
    /// Priority (critical = must have, recommended = should have, optional = nice to have)
    pub priority: String,
}

/// File I/O optimization status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IoOptimizationStatus {
    /// Whether batch write optimization is enabled
    pub batch_writes_enabled: bool,
    /// Current batch size for writes
    pub batch_size: usize,
    /// Whether WAL mode is active (SQLite)
    pub wal_mode_active: bool,
    /// Whether write-ahead checkpointing is optimized
    pub checkpoint_optimized: bool,
    /// Estimated write amplification factor (1.0 = no amplification)
    pub write_amplification: f64,
    /// Sequential write percentage (higher is better for EDR perf)
    pub sequential_write_pct: f64,
}

/// Default implementation for IoOptimizationStatus
impl Default for IoOptimizationStatus {
    fn default() -> Self {
        Self {
            batch_writes_enabled: true,
            batch_size: 500,
            wal_mode_active: true,
            checkpoint_optimized: true,
            write_amplification: 1.0,
            sequential_write_pct: 0.95,
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edr_product_serialization() {
        let product = EdrProduct {
            name: "Windows Defender".to_string(),
            vendor: "Microsoft".to_string(),
            active: true,
            notes: Some("Built-in, may need exclusions".to_string()),
            category: "antivirus".to_string(),
        };
        let json = serde_json::to_string(&product).unwrap();
        let back: EdrProduct = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Windows Defender");
        assert_eq!(back.vendor, "Microsoft");
        assert!(back.active);
    }

    #[test]
    fn test_edr_exclusion_serialization() {
        let exclusion = EdrExclusion {
            path: "C:\\Program Files\\Voltrus\\".to_string(),
            exclusion_type: "directory".to_string(),
            reason: "Database and telemetry data directory".to_string(),
            priority: "critical".to_string(),
        };
        let json = serde_json::to_string(&exclusion).unwrap();
        let back: EdrExclusion = serde_json::from_str(&json).unwrap();
        assert_eq!(back.path, "C:\\Program Files\\Voltrus\\");
        assert_eq!(back.exclusion_type, "directory");
        assert_eq!(back.priority, "critical");
    }

    #[test]
    fn test_io_optimization_default() {
        let status = IoOptimizationStatus::default();
        assert!(status.batch_writes_enabled);
        assert_eq!(status.batch_size, 500);
        assert!(status.wal_mode_active);
        assert!(status.checkpoint_optimized);
    }

    #[test]
    fn test_edr_compat_report() {
        let report = EdrCompatReport {
            detected_products: vec![EdrProduct {
                name: "Test AV".to_string(),
                vendor: "Test".to_string(),
                active: true,
                notes: None,
                category: "antivirus".to_string(),
            }],
            recommended_exclusions: vec![],
            io_optimization: IoOptimizationStatus::default(),
            compatible: true,
            warnings: vec![],
            recommendations: vec![],
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["compatible"], true);
        assert_eq!(parsed["detected_products"].as_array().unwrap().len(), 1);
    }
}
