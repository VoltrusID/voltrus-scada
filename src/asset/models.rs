use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Asset ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Asset {
    pub id: String,
    pub device_ref: Option<String>,
    pub name: String,
    pub asset_type: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub ip_address: Option<String>,
    pub mac_address: Option<String>,
    pub location: Option<String>,
    pub criticality: String,
    pub tags: Vec<String>,
    pub risk_score: Option<f64>,
    pub last_maintenance: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateAssetRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub asset_type: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub firmware_version: Option<String>,
    pub serial_number: Option<String>,
    pub ip_address: Option<String>,
    pub mac_address: Option<String>,
    pub location: Option<String>,
    #[serde(default = "default_criticality")]
    pub criticality: String,
    #[serde(default = "default_tags")]
    pub tags: Vec<String>,
    pub notes: Option<String>,
}

fn default_criticality() -> String {
    "medium".to_string()
}

fn default_tags() -> Vec<String> {
    Vec::new()
}

// ── Maintenance Log ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MaintenanceLog {
    pub id: String,
    pub asset_id: String,
    pub description: String,
    pub technician: Option<String>,
    pub performed_at: String,
    pub created_by: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMaintenanceRequest {
    pub description: String,
    pub technician: Option<String>,
    pub performed_at: String,
}

// ── Firmware Catalog ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FirmwareCatalogEntry {
    pub id: String,
    pub vendor: String,
    pub model: String,
    pub latest_version: String,
    pub release_date: Option<String>,
    pub notes: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateFirmwareCatalogRequest {
    pub vendor: String,
    pub model: String,
    pub latest_version: String,
    pub release_date: Option<String>,
    pub notes: Option<String>,
}

// ── SNMP Scan ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnmpScanResult {
    pub ip: String,
    pub sys_descr: Option<String>,
    pub sys_name: Option<String>,
    pub sys_uptime: Option<String>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub mac_address: Option<String>,
    pub scanned_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SnmpScanRequest {
    pub subnet: String,
    #[serde(default = "default_community")]
    pub community: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,
}

fn default_community() -> String {
    "public".to_string()
}

fn default_timeout_ms() -> u32 {
    2000
}

// ── Import ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportPreview {
    pub import_id: String,
    pub rows: Vec<serde_json::Value>,
    pub total: usize,
    pub valid: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportConfirmRequest {
    pub import_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImportResult {
    pub total: usize,
    pub created: usize,
    pub skipped_duplicates: usize,
    pub errors: Vec<String>,
}

// ── IP Conflict Detection ──

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IpConflict {
    pub ip: String,
    pub assets: Vec<serde_json::Value>,
    pub devices: Vec<serde_json::Value>,
}
