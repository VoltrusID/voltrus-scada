#![allow(dead_code)]
use serde::{Deserialize, Serialize};

// ── Config types ──

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub devices: Vec<DeviceConfig>,
    #[serde(default = "default_server_port")]
    pub server_port: u16,
    /// Interface to listen on. Default all interfaces; "127.0.0.1" keeps it behind a reverse proxy or tunnel.
    #[serde(default = "default_server_host")]
    pub server_host: String,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_persist_interval")]
    pub persist_interval_secs: u64,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub auth_secret: Option<String>,
    #[serde(default)]
    pub cors_origin: Option<String>,
    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub oidc_enabled: bool,
    #[serde(default)]
    pub oidc_issuer: String,
    #[serde(default)]
    pub oidc_audience: String,
    #[serde(default)]
    pub oidc_jwks_url: String,
    #[serde(default)]
    pub oidc_role_claim: String,
    #[serde(default)]
    pub oidc_admin_role: String,
    #[serde(default)]
    pub db_encryption_key: Option<String>,
    #[serde(default)]
    pub smtp: Option<super::notification_types::SmtpConfig>,
    #[serde(default)]
    pub mqtt_broker: MqttBrokerConfig,
    #[serde(default)]
    pub ha: HaConfig,
    #[serde(default)]
    pub watchdog: WatchdogConfig,
    #[serde(default)]
    pub networks: Vec<NetworkConfig>,
    #[serde(default)]
    pub device_templates: DeviceTemplatesConfig,
}

/// `device_templates:` in the config file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DeviceTemplatesConfig {
    /// Catalog index URL, written to the `device_templates.catalog_url` setting at startup.
    /// Default: the public VoltrusID catalog.
    #[serde(default)]
    pub catalog_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MqttBrokerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_mqtt_port")]
    pub port: u16,
    #[serde(default = "default_mqtt_max_connections")]
    pub max_connections: u32,
    #[serde(default)]
    pub auth: MqttAuthConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MqttAuthConfig {
    #[serde(default = "default_mqtt_auth_mode")]
    pub mode: String,
    #[serde(default)]
    pub users: Vec<MqttUser>,
}

impl Default for MqttAuthConfig {
    fn default() -> Self {
        Self {
            mode: default_mqtt_auth_mode(),
            users: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MqttUser {
    pub username: String,
    pub password: String,
}

impl Default for MqttBrokerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_mqtt_port(),
            max_connections: default_mqtt_max_connections(),
            auth: MqttAuthConfig::default(),
        }
    }
}

fn default_mqtt_port() -> u16 {
    1883
}
fn default_mqtt_max_connections() -> u32 {
    100
}
fn default_mqtt_auth_mode() -> String {
    "none".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_retention_enabled")]
    pub enabled: bool,
    #[serde(default = "default_run_interval")]
    pub run_interval_secs: u64,
    #[serde(default = "default_archive_dir")]
    pub archive_dir: String,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<RetentionTier>,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            enabled: default_retention_enabled(),
            run_interval_secs: default_run_interval(),
            archive_dir: default_archive_dir(),
            tiers: default_tiers(),
        }
    }
}

fn default_retention_enabled() -> bool {
    true
}
fn default_run_interval() -> u64 {
    21600
}
fn default_archive_dir() -> String {
    "./archive".to_string()
}

fn default_tiers() -> Vec<RetentionTier> {
    vec![
        RetentionTier {
            name: "raw".into(),
            keep: Some(604800),
            downsample_to: Some("hourly".into()),
            archive: true,
        },
        RetentionTier {
            name: "hourly".into(),
            keep: Some(7776000),
            downsample_to: Some("daily".into()),
            archive: false,
        },
        RetentionTier {
            name: "daily".into(),
            keep: None,
            downsample_to: None,
            archive: false,
        },
    ]
}

#[derive(Debug, Clone, Deserialize)]
pub struct RetentionTier {
    pub name: String,
    pub keep: Option<u64>,
    pub downsample_to: Option<String>,
    #[serde(default)]
    pub archive: bool,
}

fn default_cookie_secure() -> bool {
    true
}

fn default_server_port() -> u16 {
    3000
}
fn default_server_host() -> String {
    "0.0.0.0".to_string()
}
fn default_db_path() -> String {
    "data.db".to_string()
}
fn default_persist_interval() -> u64 {
    5
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceConfig {
    pub id: String,
    pub ip: String,
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    #[serde(default)]
    pub endianness: Endianness,
    pub registers: Vec<RegisterConfig>,
    #[serde(default)]
    pub readback: Option<ReadbackConfig>,
    #[serde(default)]
    pub network: Option<String>,
}

pub fn default_modbus_port() -> u16 {
    502
}
pub fn default_unit_id() -> u8 {
    1
}
pub fn default_interval() -> u64 {
    1000
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    #[default]
    Big,
    Little,
    Swapped,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RegisterConfig {
    pub name: String,
    pub address: u16,
    #[serde(default = "default_reg_type", rename = "type")]
    pub reg_type: RegisterType,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_register_area")]
    pub register_area: String,
    /// `address` is the 0-based offset on the wire, whatever its value. When false (the default),
    /// an address of 30001 or more is a 5-digit reference (30001 input, 40001 holding) and is converted.
    #[serde(default)]
    pub zero_based: bool,
}

pub fn default_register_area() -> String {
    "holding".to_string()
}

pub fn default_reg_type() -> RegisterType {
    RegisterType::Float32
}
pub fn default_scale() -> f64 {
    1.0
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RegisterType {
    Int16,
    Uint16,
    Int32,
    Float32,
    Float64,
}

impl RegisterConfig {
    pub fn register_count(&self) -> u16 {
        match self.reg_type {
            RegisterType::Int16 | RegisterType::Uint16 => 1,
            RegisterType::Int32 | RegisterType::Float32 => 2,
            RegisterType::Float64 => 4,
        }
    }

    /// The 0-based address sent on the wire. A zero-based register is used as is; otherwise a Modbus
    /// 5-digit reference (e.g. 40001) is converted and anything below 30001 is already 0-based.
    pub fn protocol_address(&self) -> u16 {
        if self.zero_based {
            self.address
        } else if self.address >= 40001 {
            self.address - 40001
        } else if self.address >= 30001 {
            self.address - 30001
        } else {
            self.address
        }
    }
}

// ── HA / Reliability config (v0.33) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HaConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_ha_role")]
    pub role: String,
    #[serde(default = "default_node_id")]
    pub node_id: String,
    #[serde(default = "default_ha_bind_address")]
    pub bind_address: String,
    #[serde(default)]
    pub peer_address: String,
    #[serde(default = "default_heartbeat_ms")]
    pub heartbeat_interval_ms: u64,
    #[serde(default = "default_failover_missed")]
    pub failover_after_missed: u32,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
}

impl Default for HaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            role: default_ha_role(),
            node_id: default_node_id(),
            bind_address: default_ha_bind_address(),
            peer_address: String::new(),
            heartbeat_interval_ms: default_heartbeat_ms(),
            failover_after_missed: default_failover_missed(),
            tls: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub cert_path: String,
    #[serde(default)]
    pub key_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchdogConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_check_interval")]
    pub health_check_interval_secs: u64,
    #[serde(default = "default_restart_failures")]
    pub restart_after_failures: u32,
    #[serde(default = "default_cooldown")]
    pub restart_cooldown_secs: u64,
    #[serde(default = "default_max_restarts")]
    pub max_restart_attempts: u32,
}

impl Default for WatchdogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            health_check_interval_secs: default_check_interval(),
            restart_after_failures: default_restart_failures(),
            restart_cooldown_secs: default_cooldown(),
            max_restart_attempts: default_max_restarts(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub id: String,
    #[serde(default)]
    pub description: String,
    pub interface: String,
    pub bind_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadbackConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_readback_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_readback_retries")]
    pub retry_count: u32,
    #[serde(default = "default_readback_interval")]
    pub poll_interval_ms: u64,
}

impl Default for ReadbackConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: default_readback_timeout(),
            retry_count: default_readback_retries(),
            poll_interval_ms: default_readback_interval(),
        }
    }
}

fn default_ha_role() -> String {
    "auto".to_string()
}
fn default_node_id() -> String {
    "voltrus-1".to_string()
}
fn default_ha_bind_address() -> String {
    "0.0.0.0:8443".to_string()
}
fn default_heartbeat_ms() -> u64 {
    1000
}
fn default_failover_missed() -> u32 {
    3
}
fn default_check_interval() -> u64 {
    10
}
fn default_restart_failures() -> u32 {
    3
}
fn default_cooldown() -> u64 {
    30
}
fn default_max_restarts() -> u32 {
    5
}
fn default_readback_timeout() -> u64 {
    5000
}
fn default_readback_retries() -> u32 {
    2
}
fn default_readback_interval() -> u64 {
    500
}
