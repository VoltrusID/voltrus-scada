#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

// ── API response types ──

#[derive(Debug, Serialize, ToSchema)]
pub struct LiveResponse {
    pub devices: Vec<super::DeviceSnapshot>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub devices: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DataPoint {
    pub timestamp: i64,
    pub value: f64,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub device_id: String,
    pub metric: String,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

// ── System response types ──

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct SystemResponse {
    pub hostname: String,
    pub os: String,
    pub uptime_secs: u64,
    pub cpu_usage: f32,
    pub memory_total_mb: f64,
    pub memory_used_mb: f64,
    pub memory_percent: f64,
    pub disk_total_gb: f64,
    pub disk_used_gb: f64,
    pub disk_percent: f64,
    pub db_size_mb: f64,
    pub process: ProcessInfo,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ProcessInfo {
    pub pid: u32,
    pub memory_mb: f64,
    pub cpu_percent: f32,
    pub uptime_secs: u64,
    pub binary_size_mb: f64,
}

// ── Request/Response types ──

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct LoginResponse {
    pub ok: bool,
    pub username: String,
    pub role: String,
    #[serde(default)]
    pub must_change_password: bool,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    #[serde(default = "default_user_role")]
    pub role: String,
}

pub fn default_user_role() -> String {
    "user".to_string()
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateUserRequest {
    pub role: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TokenClaims {
    pub username: String,
    pub exp: u64,
    #[serde(default = "default_user_role")]
    pub role: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshToken {
    pub id: i64,
    pub token_hash: String,
    pub username: String,
    pub expires_at: i64,
    pub created_at: i64,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ToSchema)]
pub struct BrandingSettings {
    pub title: String,
    pub subtitle: String,
    pub logo: String,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DataSourceData {
    pub id: i64,
    pub key: String,
    pub value: f64,
    pub warn: f64,
    pub crit: f64,
    pub source_type: String,
    pub status: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct HistoryResponse {
    pub sensors: Vec<DataSourceSummary>,
    #[schema(value_type = Object)]
    pub data: serde_json::Value,
    pub range: String,
    pub interval: u64,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DataSourceSummary {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub unit: String,
    pub color: String,
    pub warn_val: f64,
    pub crit_val: f64,
    pub decimals: i64,
    pub source_type: String,
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct DataQuery {
    pub screen: Option<String>,
    pub device: Option<String>,
    pub range: Option<String>,
    pub sensor: Option<String>,
    pub device_id: Option<String>,
    pub metric: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub detail: Option<String>,
    pub agg: Option<String>,
}

/// Query parameters for the /api/v1/data/export endpoint.
#[derive(Debug, serde::Deserialize)]
pub struct ExportQuery {
    /// Start time: ISO 8601 string or unix timestamp (seconds).
    /// Parsed to i64 unix timestamp.
    pub from: Option<String>,
    /// End time: ISO 8601 string or unix timestamp (seconds).
    pub to: Option<String>,
    /// Filter by device ID.
    pub device: Option<String>,
    /// Filter by metric name.
    pub metric: Option<String>,
    /// Aggregation function: avg, min, max, count, sum. Default: raw (no aggregation).
    pub aggregation: Option<String>,
    /// Aggregation window: 1m, 5m, 15m, 1h, 1d.
    pub interval: Option<String>,
    /// Pagination: max rows to return (default 1000, max 10000).
    pub limit: Option<usize>,
    /// Pagination: rows to skip (default 0).
    pub offset: Option<usize>,
    /// Response format: json (default) or csv.
    pub format: Option<String>,
}

/// A single telemetry row for export responses.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TelemetryExportRow {
    pub timestamp: i64,
    pub device: String,
    pub metric: String,
    pub value: f64,
}

/// Paginated meta information for export responses.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ExportMeta {
    pub total: i64,
    pub limit: usize,
    pub offset: usize,
    pub aggregation: String,
    pub interval: String,
}

/// The standard envelope for JSON export responses.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct ExportResponse {
    pub data: Vec<TelemetryExportRow>,
    pub meta: ExportMeta,
}
