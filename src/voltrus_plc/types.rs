use crate::models::VoltrusPlcTag;

/// Runtime state of a Voltrus PLC (mirrors the gRPC RuntimeState).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PlcRuntimeState {
    Stopped,
    Running,
    Paused,
    Error,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateVoltrusPlcDeviceRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, serde::Deserialize)]
pub struct UpdateVoltrusPlcDeviceRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub transport: Option<String>,
    pub interval_ms: Option<u64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, serde::Deserialize)]
pub struct CreateVoltrusPlcTagRequest {
    pub variable_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
}

/// Runtime config for a Voltrus PLC device, used by the poller.
#[derive(Debug, Clone)]
pub struct VoltrusPlcDeviceConfig {
    pub id: String,
    pub url: String,
    pub transport: String,
    pub interval_ms: u64,
    pub tags: Vec<VoltrusPlcTag>,
}

fn default_transport() -> String {
    "grpc".to_string()
}

fn default_interval_ms() -> u64 {
    1000
}

fn default_true() -> bool {
    true
}
