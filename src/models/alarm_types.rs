#![allow(dead_code)]

// ── Alarm types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Alarm {
    pub id: i64,
    pub source_key: String,
    pub source_name: String,
    pub severity: String,
    pub priority: String,
    pub value: f64,
    pub threshold: f64,
    pub message: String,
    pub triggered_at: i64,
    pub resolved_at: Option<i64>,
    pub acknowledged_at: Option<i64>,
    pub acknowledged_by: Option<String>,
    pub cooldown_until: Option<i64>,
    pub shelved_until: Option<i64>,
    pub shelved_by: Option<String>,
    pub shelved_reason: Option<String>,
    pub deadband: f64,
    pub state: String,
    /// "high" (fires when value ≥ threshold) or "low" (fires when value ≤
    /// threshold). Drives direction-aware auto-resolution.
    #[serde(default = "default_alarm_type")]
    pub alarm_type: String,
}

/// An alarm that just changed state (resolved, acknowledged, shelved or
/// unshelved), as the storage reports it for the WebSocket push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlarmTransition {
    pub alarm_id: i64,
    pub source_key: String,
    /// The alarm's state after the change.
    pub state: String,
}

#[derive(Debug, serde::Deserialize)]
pub struct AlarmQuery {
    pub status: Option<String>,
    pub limit: Option<i64>,
}

// ── Alarm Management Overhaul types (v0.23.0) ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmAuditEntry {
    pub id: i64,
    pub alarm_id: i64,
    pub action: String,
    pub performed_by: String,
    pub details: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ShelfAlarmRequest {
    pub duration_secs: u64,
    pub reason: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmSummary {
    pub active_critical: i64,
    pub active_warning: i64,
    pub active_info: i64,
    pub shelved: i64,
    pub recent_24h: i64,
    pub avg_ack_time_secs: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmAnalyticsEntry {
    pub source_key: String,
    pub source_name: String,
    pub trigger_count: i64,
    pub avg_time_to_ack_secs: f64,
    pub last_triggered_at: i64,
}

/// Append-only journal row for one alarm lifecycle event (ISA-18.2):
/// triggered, acknowledged, shelved, unshelved, resolved.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct AlarmJournalEntry {
    pub id: i64,
    pub alarm_id: i64,
    pub event: String,
    pub at: i64,
    pub actor: Option<String>,
}

// ── Notification settings ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct NotificationSetting {
    pub id: i64,
    pub source_key: String,
    pub channel_type: String,
    pub config: String,
    pub enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateNotificationSettingRequest {
    pub source_key: String,
    pub channel_type: String,
    pub config: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

pub fn default_interval_ms() -> u64 {
    1000
}

pub fn default_true() -> bool {
    true
}

pub fn default_alarm_type() -> String {
    "high".into()
}

fn default_retention_hours() -> u32 {
    72
}

// ── RTSP Video / Camera types ──

/// Database row for cameras table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Camera {
    pub id: i64,
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub snapshot_on_alarm: bool,
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u32,
    pub created_at: i64,
}

/// Database row for camera_snapshots table.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct CameraSnapshot {
    pub id: i64,
    pub camera_id: i64,
    pub timestamp: i64,
    pub alarm_source: Option<String>,
    pub file_path: String,
    pub file_size: u64,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateCameraRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub snapshot_on_alarm: bool,
    #[serde(default = "default_retention_hours")]
    pub retention_hours: u32,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateCameraRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub enabled: Option<bool>,
    pub snapshot_on_alarm: Option<bool>,
    pub retention_hours: Option<u32>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateNotificationSettingRequest {
    pub source_key: Option<String>,
    pub channel_type: Option<String>,
    pub config: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct TestNotificationRequest {
    pub channel_type: String,
    pub config: String,
    pub source_key: Option<String>,
}

// ── Push subscription (groundwork for Web Push) ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PushSubscription {
    pub id: i64,
    pub user: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth_key: String,
    pub created_at: i64,
}

#[derive(Debug, serde::Deserialize)]
pub struct PushSubscribeRequest {
    pub endpoint: String,
    pub p256dh: String,
    pub auth_key: String,
}
