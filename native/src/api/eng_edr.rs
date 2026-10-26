//! Data Manager › EDR compatibility: the server's `/api/v1/edr/*` routes
//! (`src/api/edr_compat.rs`), mirrored. The report, exclusions, I/O status
//! and the Group Policy guide are open to every signed-in user; the
//! Windows Defender script (`/exclusions/ps1`) is an operator route.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::VoltrusApi;

pub const REPORT: &str = "/api/v1/edr/report";
pub const EXCLUSIONS: &str = "/api/v1/edr/exclusions";
pub const PS1: &str = "/api/v1/edr/exclusions/ps1";
pub const IO_STATUS: &str = "/api/v1/edr/io-status";
pub const GPO_GUIDE: &str = "/api/v1/edr/gpo-guide";
/// The name the server's `Content-Disposition` suggests for the script.
pub const PS1_FILE: &str = "voltrus-defender-exclusions.ps1";

/// `edr_compat::EdrProduct`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct EdrProduct {
    pub name: String,
    pub vendor: String,
    pub active: bool,
    pub notes: Option<String>,
    /// `antivirus`, `edr`, `xdr`, `ot_security`, …
    pub category: String,
}

/// `edr_compat::EdrExclusion`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct EdrExclusion {
    pub path: String,
    /// `directory`, `file`, `process` or `extension`.
    pub exclusion_type: String,
    pub reason: String,
    /// `critical`, `recommended` or `optional`.
    pub priority: String,
}

/// `edr_compat::IoOptimizationStatus` (also `GET /io-status`).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct IoStatus {
    pub batch_writes_enabled: bool,
    pub batch_size: usize,
    pub wal_mode_active: bool,
    pub checkpoint_optimized: bool,
    pub write_amplification: f64,
    /// 0–1.
    pub sequential_write_pct: f64,
}

/// `edr_compat::EdrCompatReport`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct EdrReport {
    pub detected_products: Vec<EdrProduct>,
    pub recommended_exclusions: Vec<EdrExclusion>,
    pub io_optimization: IoStatus,
    pub compatible: bool,
    pub warnings: Vec<String>,
    pub recommendations: Vec<String>,
}

/// `GET /api/v1/edr/exclusions`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Exclusions {
    pub exclusions: Vec<EdrExclusion>,
    pub count: usize,
}

impl EdrReport {
    /// `2 product(s) detected / 1 warning(s)` (the web banner's line).
    pub fn banner(&self) -> String {
        format!(
            "{} product(s) detected / {} warning(s)",
            self.detected_products.len(),
            self.warnings.len()
        )
    }
}

/// The web client's names for the product categories.
pub fn category_label(category: &str) -> String {
    match category {
        "antivirus" => "Antivirus".to_owned(),
        "edr" => "EDR".to_owned(),
        "xdr" => "XDR".to_owned(),
        "ot_security" => "OT Security".to_owned(),
        other => other.to_owned(),
    }
}

/// How urgent an exclusion is: critical 0, recommended 1, anything else 2.
pub fn priority_rank(priority: &str) -> u8 {
    match priority {
        "critical" => 0,
        "recommended" => 1,
        _ => 2,
    }
}

/// Exclusions whose path, type, reason or priority has `query` in it.
pub fn filter_exclusions<'a>(list: &'a [EdrExclusion], query: &str) -> Vec<&'a EdrExclusion> {
    let query = query.trim().to_lowercase();
    list.iter()
        .filter(|exclusion| {
            query.is_empty()
                || [
                    &exclusion.path,
                    &exclusion.exclusion_type,
                    &exclusion.reason,
                    &exclusion.priority,
                ]
                .iter()
                .any(|text| text.to_lowercase().contains(&query))
        })
        .collect()
}

/// A text answer as lines for a virtualised list (CR LF or LF; tabs as
/// four spaces, since the list draws one line per row).
pub fn text_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.trim_end_matches('\r').replace('\t', "    "))
        .collect()
}

/// `1.0x`, `95%` as the web report shows them.
pub fn amplification_label(value: f64) -> String {
    format!("{value:.1}x")
}

pub fn percent_label(fraction: f64) -> String {
    format!("{:.0}%", fraction * 100.0)
}

impl VoltrusApi {
    pub async fn edr_report(&self) -> Result<EdrReport> {
        self.get_json(REPORT).await
    }

    pub async fn edr_exclusions(&self) -> Result<Exclusions> {
        self.get_json(EXCLUSIONS).await
    }

    pub async fn edr_io_status(&self) -> Result<IoStatus> {
        self.get_json(IO_STATUS).await
    }

    /// The Group Policy deployment guide (plain text).
    pub async fn edr_gpo_guide(&self) -> Result<String> {
        self.get_text(GPO_GUIDE).await
    }

    /// The Windows Defender exclusion script (plain text; operators).
    pub async fn edr_defender_ps1(&self) -> Result<String> {
        self.get_text(PS1).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_decodes_as_get_edr_report_sends_it() {
        let body = r#"{
            "detected_products": [{"name": "Microsoft Defender Antivirus", "vendor": "Microsoft",
                "active": true, "notes": null, "category": "antivirus"},
              {"name": "Claroty", "vendor": "Claroty", "active": false,
                "notes": "Network-level only", "category": "ot_security"}],
            "recommended_exclusions": [{"path": "C:\\ProgramData\\Voltrus\\voltrus.db",
                "exclusion_type": "file", "reason": "SQLite database", "priority": "critical"},
              {"path": "voltrus.exe", "exclusion_type": "process", "reason": "Main process",
                "priority": "recommended"}],
            "io_optimization": {"batch_writes_enabled": true, "batch_size": 500,
                "wal_mode_active": true, "checkpoint_optimized": true,
                "write_amplification": 1.0, "sequential_write_pct": 0.95},
            "compatible": true,
            "warnings": ["Microsoft Defender Antivirus (Microsoft) is active — add exclusions to prevent performance degradation"],
            "recommendations": ["Add recommended exclusion paths to your EDR/antivirus policy"]
        }"#;
        let report: EdrReport = serde_json::from_str(body).unwrap();
        assert!(report.compatible);
        assert_eq!(report.banner(), "2 product(s) detected / 1 warning(s)");
        assert_eq!(
            report.detected_products[1].notes.as_deref(),
            Some("Network-level only")
        );
        assert_eq!(report.io_optimization.batch_size, 500);
        assert_eq!(
            amplification_label(report.io_optimization.write_amplification),
            "1.0x"
        );
        assert_eq!(
            percent_label(report.io_optimization.sequential_write_pct),
            "95%"
        );
        assert_eq!(category_label("ot_security"), "OT Security");
        assert_eq!(category_label("sandbox"), "sandbox");
    }

    #[test]
    fn exclusions_and_io_status_decode() {
        let exclusions: Exclusions = serde_json::from_str(
            r#"{"exclusions": [{"path": "/var/lib/voltrus", "exclusion_type": "directory",
                "reason": "Data directory", "priority": "critical"}], "count": 1}"#,
        )
        .unwrap();
        assert_eq!(exclusions.count, exclusions.exclusions.len());
        let io: IoStatus = serde_json::from_str(
            r#"{"batch_writes_enabled": true, "batch_size": 500, "wal_mode_active": false,
                "checkpoint_optimized": false, "write_amplification": 1.0,
                "sequential_write_pct": 0.95}"#,
        )
        .unwrap();
        assert!(!io.wal_mode_active);
    }

    #[test]
    fn exclusions_filter_and_rank() {
        let list = vec![
            EdrExclusion {
                path: "/data".into(),
                exclusion_type: "directory".into(),
                reason: "SQLite".into(),
                priority: "critical".into(),
            },
            EdrExclusion {
                path: "voltrus".into(),
                exclusion_type: "process".into(),
                reason: "Main".into(),
                priority: "optional".into(),
            },
        ];
        assert_eq!(filter_exclusions(&list, "PROCESS").len(), 1);
        assert_eq!(filter_exclusions(&list, "sqlite")[0].path, "/data");
        assert_eq!(filter_exclusions(&list, " ").len(), 2);
        assert!(priority_rank("critical") < priority_rank("recommended"));
        assert_eq!(priority_rank("optional"), priority_rank("whatever"));
    }

    #[test]
    fn text_answers_split_into_lines() {
        assert_eq!(
            text_lines("# Script\r\n\tAdd-MpPreference\r\n"),
            ["# Script", "    Add-MpPreference"]
        );
        assert!(text_lines("").is_empty());
    }
}
