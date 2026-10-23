#![allow(dead_code)]
use super::*;

// -- OEE / Downtime types --

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineState {
    Running,
    Idle,
    Faulted,
    Setup,
    Maintenance,
}

impl MachineState {
    pub fn as_str(&self) -> &'static str {
        match self {
            MachineState::Running => "running",
            MachineState::Idle => "idle",
            MachineState::Faulted => "faulted",
            MachineState::Setup => "setup",
            MachineState::Maintenance => "maintenance",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "running" => Some(MachineState::Running),
            "idle" => Some(MachineState::Idle),
            "faulted" => Some(MachineState::Faulted),
            "setup" => Some(MachineState::Setup),
            "maintenance" => Some(MachineState::Maintenance),
            _ => None,
        }
    }
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct DowntimeEvent {
    pub id: i64,
    pub device_id: String,
    pub state: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_secs: Option<i64>,
    pub reason: Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateDowntimeEventRequest {
    pub device_id: String,
    pub state: String,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CloseDowntimeEventRequest {
    pub reason: Option<String>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct OeeMetrics {
    pub availability: f64,
    pub performance: f64,
    pub quality: f64,
    pub oee: f64,
    pub total_running_secs: i64,
    pub total_planned_secs: i64,
    pub downtime_count: i64,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ShiftConfig {
    pub id: i64,
    pub name: String,
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_hours: u32,
    pub days_mask: u32,
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateShiftRequest {
    pub name: String,
    pub start_hour: u32,
    pub start_minute: u32,
    pub duration_hours: u32,
    pub days_mask: u32,
}

// ── Report types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Report {
    pub id: i64,
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
    pub device_ids: String,
    pub email_recipients: String,
    pub last_generated: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateReportRequest {
    pub name: String,
    pub schedule: String,
    pub device_ids: Option<String>,
    pub email_recipients: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateReportRequest {
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub device_ids: Option<String>,
    pub email_recipients: Option<String>,
    pub enabled: Option<bool>,
}

// ── Hub (Multi-Site) types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct HubSite {
    pub id: i64,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub enabled: bool,
    pub last_seen: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateHubSiteRequest {
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateHubSiteRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub api_key: Option<String>,
    pub enabled: Option<bool>,
}

// ── Recipe types ──

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    pub category: Option<String>,
    pub device_id: String,
    pub values: String,
    pub version: i32,
    pub description: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateRecipeRequest {
    pub name: String,
    pub category: Option<String>,
    pub device_id: String,
    pub description: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct LoadRecipeRequest {
    pub recipe_id: i64,
}

#[derive(Debug, Clone)]
pub struct LicenseActivationRow {
    pub license_key: String,
    pub fingerprint_hash: String,
    pub activation_blob: Vec<u8>,
    pub fingerprint_hostname: String,
    pub fingerprint_cpu: String,
    pub fingerprint_disk_serial: String,
    pub fingerprint_mac: String,
    pub fingerprint_machine_id: String,
    pub activated_at: String,
    pub expires_at: Option<String>,
    pub tier: Option<String>,
    pub device_limit: Option<u32>,
}

// ── Security Audit types (v0.22.0 IEC 62443) ──

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SecurityEvent {
    pub id: i64,
    pub event_type: String,
    pub severity: String,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub username: Option<String>,
    pub details: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateSecurityEvent {
    pub event_type: String,
    #[serde(default = "default_severity")]
    pub severity: String,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
    pub username: Option<String>,
    pub details: Option<String>,
}

fn default_severity() -> String {
    "info".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SecurityThresholds {
    pub failed_login_threshold: u32,
    pub failed_login_window_secs: u64,
    pub suspicious_401_threshold: u32,
    pub suspicious_401_window_secs: u64,
    pub rate_limit_threshold: u32,
    pub rate_limit_window_secs: u64,
}

impl Default for SecurityThresholds {
    fn default() -> Self {
        Self {
            failed_login_threshold: 5,
            failed_login_window_secs: 60,
            suspicious_401_threshold: 10,
            suspicious_401_window_secs: 60,
            rate_limit_threshold: 100,
            rate_limit_window_secs: 60,
        }
    }
}

// ── Annotation types (v0.23.0) ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Annotation {
    pub id: i64,
    pub tag_id: String,
    pub annotation_text: String,
    pub data_timestamp: i64,
    pub author_id: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateAnnotationRequest {
    pub tag_id: String,
    pub annotation_text: String,
    pub data_timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateAnnotationRequest {
    pub annotation_text: Option<String>,
}

// ── Shift Handover types (v0.23.0) ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ShiftHandover {
    pub id: i64,
    pub author_id: i64,
    pub shift_date: String,
    pub shift_period: String,
    pub what_happened: String,
    pub whats_pending: String,
    pub what_to_watch: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateShiftHandoverRequest {
    pub shift_date: String,
    pub shift_period: String,
    pub what_happened: Option<String>,
    pub whats_pending: Option<String>,
    pub what_to_watch: Option<String>,
}

// ── User Notification types (v0.23.0) ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UserNotification {
    pub id: i64,
    pub user_id: i64,
    pub from_user_id: i64,
    pub notification_type: String,
    pub message: String,
    pub reference_id: Option<i64>,
    pub read: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNotificationRequest {
    pub user_id: i64,
    pub from_user_id: i64,
    pub notification_type: String,
    pub message: String,
    pub reference_id: Option<i64>,
}

/// Extract `@username` patterns from text, returning a deduplicated list of usernames.
/// Ignores `@` when it is preceded by a word character (e.g. email addresses).
pub fn parse_mentions(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    while i < len {
        if bytes[i] == b'@' {
            // Skip if preceded by a word character (prevents matching email addresses)
            let prev_is_word =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            if !prev_is_word {
                let start = i + 1;
                let mut end = start;
                while end < len && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                    end += 1;
                }
                if end > start {
                    let name = text[start..end].to_string();
                    if seen.insert(name.clone()) {
                        result.push(name);
                    }
                }
                i = end;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    result
}

// ── Simulation types (v0.26.0) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimProfile {
    pub id: i64,
    pub name: String,
    pub data_type: String,
    pub config: String,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateSimProfileRequest {
    pub name: String,
    pub data_type: String,
    pub config: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSimProfileRequest {
    pub name: Option<String>,
    pub data_type: Option<String>,
    pub config: Option<String>,
}

// ── Script types (v0.26.0) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Script {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub syntax_error: Option<String>,
    pub cpu_limit_ms: i64,
    pub memory_limit_bytes: i64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateScriptRequest {
    pub name: String,
    pub source: String,
    #[serde(default = "default_cpu_limit")]
    pub cpu_limit_ms: Option<i64>,
    #[serde(default = "default_memory_limit")]
    pub memory_limit_bytes: Option<i64>,
}

fn default_cpu_limit() -> Option<i64> {
    Some(1000)
}
fn default_memory_limit() -> Option<i64> {
    Some(1048576)
}

#[derive(Debug, Deserialize)]
pub struct UpdateScriptRequest {
    pub name: Option<String>,
    pub source: Option<String>,
    pub cpu_limit_ms: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct TestScriptRequest {
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptExecution {
    pub id: i64,
    pub script_id: i64,
    pub flow_id: Option<String>,
    pub trigger_type: Option<String>,
    pub duration_ms: i64,
    pub status: String,
    pub error_message: Option<String>,
    pub output: Option<String>,
    pub created_at: i64,
}

// ── Config Snapshot types (v0.26.0) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub id: i64,
    pub label: Option<String>,
    pub description: Option<String>,
    pub snapshot_data: String,
    pub trigger: String,
    pub created_by: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct CreateSnapshotRequest {
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiffEntry {
    pub field: String,
    pub from: serde_json::Value,
    pub to: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigTableDiff {
    pub added: Vec<serde_json::Value>,
    pub removed: Vec<serde_json::Value>,
    pub changed: Vec<ConfigDiffEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiff {
    pub tables: std::collections::HashMap<String, ConfigTableDiff>,
}
