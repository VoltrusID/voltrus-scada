#![allow(dead_code)]
use serde::{Deserialize, Serialize};

fn default_dnp3_port() -> u16 {
    20000
}
fn default_master_address() -> u16 {
    1
}
fn default_outstation_address() -> u16 {
    10
}
fn default_integrity_interval() -> u64 {
    300
}
fn default_float64() -> String {
    "float64".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Dnp3Device {
    pub id: i64,
    pub name: String,
    pub ip: String,
    #[serde(default = "default_dnp3_port")]
    pub port: u16,
    #[serde(default = "default_master_address")]
    pub master_address: u16,
    #[serde(default = "default_outstation_address")]
    pub outstation_address: u16,
    #[serde(default = "default_true")]
    pub enable_unsolicited: bool,
    #[serde(default = "default_integrity_interval")]
    pub integrity_poll_interval_secs: u64,
    pub event_scan_interval_ms: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Dnp3Point {
    pub id: i64,
    pub device_id: i64,
    pub group_num: u8,
    pub variation: u8,
    pub point_index: u16,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_float64")]
    pub data_type: String,
    #[serde(default)]
    pub scale: f64,
    #[serde(default)]
    pub deadband: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct Dnp3DeviceConfig {
    pub id: String,
    pub ip: String,
    pub port: u16,
    pub master_address: u16,
    pub outstation_address: u16,
    pub event_scan_interval_ms: Option<u64>,
    pub points: Vec<Dnp3PointConfig>,
}

#[derive(Debug, Clone)]
pub struct Dnp3PointConfig {
    pub group_num: u8,
    pub variation: u8,
    pub point_index: u16,
    pub name: String,
    pub scale: f64,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDnp3DeviceRequest {
    pub name: String,
    pub ip: String,
    #[serde(default = "default_dnp3_port")]
    pub port: u16,
    #[serde(default = "default_master_address")]
    pub master_address: u16,
    #[serde(default = "default_outstation_address")]
    pub outstation_address: u16,
    #[serde(default = "default_true")]
    pub enable_unsolicited: bool,
    #[serde(default = "default_integrity_interval")]
    pub integrity_poll_interval_secs: u64,
    pub event_scan_interval_ms: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateDnp3DeviceRequest {
    pub name: Option<String>,
    pub ip: Option<String>,
    pub port: Option<u16>,
    pub master_address: Option<u16>,
    pub outstation_address: Option<u16>,
    pub enable_unsolicited: Option<bool>,
    pub integrity_poll_interval_secs: Option<u64>,
    pub event_scan_interval_ms: Option<Option<u64>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateDnp3PointRequest {
    pub group_num: u8,
    pub variation: u8,
    pub point_index: u16,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default = "default_float64")]
    pub data_type: String,
    #[serde(default)]
    pub scale: f64,
    #[serde(default)]
    pub deadband: f64,
}

fn default_true() -> bool {
    true
}
