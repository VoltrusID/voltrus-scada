#![allow(dead_code)]
use serde::{Deserialize, Serialize};

fn default_bacnet_port() -> u16 {
    47808
}
fn default_interval() -> u64 {
    5000
}
fn default_cov_lifetime() -> u32 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BacnetDevice {
    pub id: i64,
    pub name: String,
    pub ip: String,
    #[serde(default = "default_bacnet_port")]
    pub port: u16,
    pub device_instance: u32,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    #[serde(default)]
    pub cov_enabled: bool,
    #[serde(default = "default_cov_lifetime")]
    pub cov_lifetime: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BacnetObject {
    pub id: i64,
    pub device_id: i64,
    pub object_type: String,
    pub object_instance: u32,
    pub property: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub cov_increment: f64,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct BacnetDeviceConfig {
    pub id: String,
    pub ip: String,
    pub port: u16,
    pub device_instance: u32,
    pub interval_ms: u64,
    pub cov_enabled: bool,
    pub cov_lifetime: u32,
    pub objects: Vec<BacnetObjectConfig>,
}

impl BacnetDeviceConfig {
    /// The poller config for a stored device and its objects. The poller id is
    /// `bacnet_<device id>`, the key its readings carry in the cache.
    pub fn from_rows(device: &BacnetDevice, objects: &[BacnetObject]) -> Self {
        Self {
            id: format!("bacnet_{}", device.id),
            ip: device.ip.clone(),
            port: device.port,
            device_instance: device.device_instance,
            interval_ms: device.interval_ms,
            cov_enabled: device.cov_enabled,
            cov_lifetime: device.cov_lifetime,
            objects: objects
                .iter()
                .map(|o| BacnetObjectConfig {
                    object_type: o.object_type.clone(),
                    object_instance: o.object_instance,
                    property: o.property.clone(),
                    name: o.name.clone(),
                    cov_increment: o.cov_increment,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BacnetObjectConfig {
    pub object_type: String,
    pub object_instance: u32,
    pub property: String,
    pub name: String,
    pub cov_increment: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BacnetDiscoveredDevice {
    pub device_instance: u32,
    pub ip: String,
    pub port: u16,
    pub vendor_name: Option<String>,
    pub model_name: Option<String>,
    pub firmware_version: Option<String>,
    pub object_list_count: Option<u32>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateBacnetDeviceRequest {
    pub name: String,
    pub ip: String,
    #[serde(default = "default_bacnet_port")]
    pub port: u16,
    #[serde(default)]
    pub device_instance: u32,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    #[serde(default)]
    pub cov_enabled: bool,
    #[serde(default = "default_cov_lifetime")]
    pub cov_lifetime: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateBacnetDeviceRequest {
    pub name: Option<String>,
    pub ip: Option<String>,
    pub port: Option<u16>,
    pub device_instance: Option<u32>,
    pub interval_ms: Option<u64>,
    pub cov_enabled: Option<bool>,
    pub cov_lifetime: Option<u32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateBacnetObjectRequest {
    pub object_type: String,
    pub object_instance: u32,
    pub property: Option<String>,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub cov_increment: f64,
}

fn default_true() -> bool {
    true
}
