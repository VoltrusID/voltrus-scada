#![allow(dead_code)]

#[cfg(feature = "modbus")]
use super::{
    DeviceConfig, Endianness, RegisterConfig, RegisterType, default_interval, default_modbus_port,
    default_register_area, default_scale, default_unit_id,
};

#[cfg(any(
    feature = "modbus",
    feature = "opcua",
    feature = "s7",
    feature = "ethernet-ip"
))]
use super::{default_interval_ms, default_true};

#[cfg(feature = "modbus")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct ModbusDevice {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u64,
    pub endianness: String,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(feature = "modbus")]
impl ModbusDevice {
    pub fn to_device_config(&self, registers: &[ModbusRegister]) -> DeviceConfig {
        DeviceConfig {
            id: self.id.clone(),
            ip: self.ip.clone(),
            port: self.port,
            unit_id: self.unit_id,
            interval_ms: self.interval_ms,
            endianness: match self.endianness.as_str() {
                "little" => Endianness::Little,
                "swapped" => Endianness::Swapped,
                _ => Endianness::Big,
            },
            registers: registers
                .iter()
                .map(|r| RegisterConfig {
                    name: r.name.clone(),
                    address: r.address,
                    reg_type: match r.register_type.as_str() {
                        "int16" => RegisterType::Int16,
                        "uint16" => RegisterType::Uint16,
                        "int32" => RegisterType::Int32,
                        "float64" => RegisterType::Float64,
                        _ => RegisterType::Float32,
                    },
                    scale: r.scale,
                    register_area: r.register_area.clone(),
                    zero_based: r.zero_based,
                })
                .collect(),
            readback: None,
            network: None,
        }
    }
}

#[cfg(feature = "modbus")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModbusRegister {
    pub id: i64,
    pub device_id: String,
    pub name: String,
    pub address: u16,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
    pub sort_order: i64,
    /// `address` is a 0-based offset (see `RegisterConfig::zero_based`).
    #[serde(default)]
    pub zero_based: bool,
}

#[cfg(feature = "modbus")]
#[derive(Debug, serde::Deserialize)]
pub struct CreateModbusDeviceRequest {
    pub id: String,
    pub name: String,
    pub ip: String,
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    #[serde(default)]
    pub endianness: String,
    pub registers: Vec<ModbusRegisterInput>,
}

#[cfg(feature = "modbus")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ModbusRegisterInput {
    pub name: String,
    /// Legacy address: a 0-based offset below 30001, or a 5-digit reference (30001 and up, 40001 and up).
    /// Give either `address` or `offset`.
    #[serde(default)]
    pub address: Option<u16>,
    /// Explicit 0-based offset in `register_area`, valid for the whole 0-65535 range. Replaces `address`.
    #[serde(default)]
    pub offset: Option<u16>,
    /// With `address`: the value is a 0-based offset as is (what the API returns for a register stored with `offset`).
    #[serde(default)]
    pub zero_based: bool,
    #[serde(default = "default_reg_type_str")]
    pub register_type: String,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_register_area")]
    pub register_area: String,
}

#[cfg(feature = "modbus")]
impl ModbusRegisterInput {
    /// The address to store and whether it is a 0-based offset.
    pub fn resolve_address(&self) -> Result<(u16, bool), String> {
        match (self.address, self.offset) {
            (Some(_), Some(_)) => Err(format!(
                "register '{}': give address or offset, not both",
                self.name
            )),
            (None, Some(offset)) => Ok((offset, true)),
            (Some(address), None) => Ok((address, self.zero_based)),
            (None, None) => Err(format!(
                "register '{}': address or offset is required",
                self.name
            )),
        }
    }

    /// Parse the `registers` array of a request body; a register that does not parse is an error, not skipped.
    pub fn parse_all(body: &serde_json::Value) -> Result<Vec<ModbusRegisterInput>, String> {
        let Some(items) = body.get("registers").and_then(|v| v.as_array()) else {
            return Ok(Vec::new());
        };
        items
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let input: ModbusRegisterInput = serde_json::from_value(r.clone())
                    .map_err(|e| format!("register {}: {e}", i + 1))?;
                input.resolve_address()?;
                Ok(input)
            })
            .collect()
    }
}

#[cfg(feature = "modbus")]
fn default_reg_type_str() -> String {
    "float32".to_string()
}

#[cfg(feature = "modbus")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ScanRegisterRequest {
    pub ip: String,
    #[serde(default = "default_modbus_port")]
    pub port: u16,
    #[serde(default = "default_unit_id")]
    pub unit_id: u8,
    pub start_addr: u16,
    pub end_addr: u16,
    #[serde(default)]
    pub endianness: String,
    #[serde(default = "default_scan_register_area")]
    pub register_area: String,
    #[serde(default = "default_scan_delay")]
    pub scan_delay_ms: u64,
}

#[cfg(feature = "modbus")]
fn default_scan_delay() -> u64 {
    500
}

#[cfg(feature = "modbus")]
fn default_scan_register_area() -> String {
    "holding".to_string()
}

#[cfg(feature = "modbus")]
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ScannedRegister {
    pub address: u16,
    pub raw_value: Option<u16>,
    pub raw_value2: Option<u16>,
    pub changed: bool,
    pub readable: bool,
    pub likely_type: String,
    pub interpreted_value: Option<f64>,
}

#[cfg(feature = "modbus")]
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct PollerStatus {
    pub device_id: String,
    pub running: bool,
    pub status: String,
}

// ── OPC-UA config types ──

#[cfg(feature = "opcua")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, utoipa::ToSchema)]
pub enum OpcUaSecurityPolicy {
    #[default]
    None,
    #[serde(rename = "basic256sha256")]
    Basic256Sha256,
}

#[cfg(feature = "opcua")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, utoipa::ToSchema)]
pub enum OpcUaSecurityMode {
    #[default]
    None,
    Sign,
    #[serde(rename = "signandencrypt")]
    SignAndEncrypt,
}

#[cfg(feature = "opcua")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OpcUaNodeConfig {
    pub node_id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
}

/// Runtime config for an OPC-UA device, used by the poller.
#[cfg(feature = "opcua")]
#[derive(Debug, Clone)]
pub struct OpcUaDeviceConfig {
    pub id: String,
    pub url: String,
    pub security_policy: OpcUaSecurityPolicy,
    pub security_mode: OpcUaSecurityMode,
    pub username: Option<String>,
    pub password: Option<String>,
    pub interval_ms: u64,
    pub nodes: Vec<OpcUaNodeConfig>,
}

/// Database row for opcua_devices table.
#[cfg(feature = "opcua")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OpcUaDevice {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub security_policy: OpcUaSecurityPolicy,
    pub security_mode: OpcUaSecurityMode,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: i64,
}

/// Database row for opcua_nodes table.
#[cfg(feature = "opcua")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct OpcUaNode {
    pub id: i64,
    pub device_id: i64,
    pub node_id: String,
    pub name: String,
    pub display_name: String,
    pub data_type: String,
    pub created_at: i64,
}

#[cfg(feature = "opcua")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateOpcUaDeviceRequest {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub security_policy: OpcUaSecurityPolicy,
    #[serde(default)]
    pub security_mode: OpcUaSecurityMode,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[cfg(feature = "opcua")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateOpcUaDeviceRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub security_policy: Option<OpcUaSecurityPolicy>,
    pub security_mode: Option<OpcUaSecurityMode>,
    pub username: Option<Option<String>>,
    pub password: Option<Option<String>>,
    pub interval_ms: Option<u64>,
    pub enabled: Option<bool>,
}

#[cfg(feature = "opcua")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateOpcUaNodeRequest {
    pub node_id: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
}

// ── S7 (Siemens) device types ──

/// Database row for s7_devices table.
#[cfg(feature = "s7")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S7Device {
    pub id: i64,
    pub name: String,
    pub url: String,
    #[serde(default = "default_s7_rack")]
    pub rack: u8,
    #[serde(default = "default_s7_slot")]
    pub slot: u8,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: i64,
}

#[cfg(feature = "s7")]
fn default_s7_rack() -> u8 {
    0
}
#[cfg(feature = "s7")]
fn default_s7_slot() -> u8 {
    1
}

/// Database row for s7_tags table.
#[cfg(feature = "s7")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct S7TagRow {
    pub id: i64,
    pub device_id: i64,
    pub tag_address: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
    pub created_at: i64,
}

#[cfg(feature = "s7")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateS7DeviceRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_s7_rack")]
    pub rack: u8,
    #[serde(default = "default_s7_slot")]
    pub slot: u8,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[cfg(feature = "s7")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateS7DeviceRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub rack: Option<u8>,
    pub slot: Option<u8>,
    pub interval_ms: Option<u64>,
    pub enabled: Option<bool>,
}

#[cfg(feature = "s7")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateS7TagRequest {
    pub tag_address: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
}

// ── EtherNet/IP (Allen-Bradley) device types ──

/// Database row for enip_devices table.
#[cfg(feature = "ethernet-ip")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EnipDevice {
    pub id: i64,
    pub name: String,
    pub url: String,
    #[serde(default = "default_enip_slot")]
    pub slot: u8,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub assembly_instance: u32,
    pub created_at: i64,
}

#[cfg(feature = "ethernet-ip")]
fn default_enip_slot() -> u8 {
    0
}

/// Database row for enip_tags table.
#[cfg(feature = "ethernet-ip")]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct EnipTag {
    pub id: i64,
    pub device_id: i64,
    pub tag_name: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
    #[serde(default)]
    pub assembly_offset: u32,
    #[serde(default)]
    pub assembly_length: u32,
    pub created_at: i64,
}

#[cfg(feature = "ethernet-ip")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateEnipDeviceRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_enip_slot")]
    pub slot: u8,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub assembly_instance: u32,
}

#[cfg(feature = "ethernet-ip")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateEnipDeviceRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub slot: Option<u8>,
    pub interval_ms: Option<u64>,
    pub enabled: Option<bool>,
    pub assembly_instance: Option<u32>,
}

#[cfg(feature = "ethernet-ip")]
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateEnipTagRequest {
    pub tag_name: String,
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
    #[serde(default)]
    pub assembly_offset: u32,
    #[serde(default)]
    pub assembly_length: u32,
}

/// Runtime config for an EtherNet/IP device, used by the poller.
#[cfg(feature = "ethernet-ip")]
#[derive(Debug, Clone)]
pub struct EnipDeviceConfig {
    pub id: String,
    pub url: String,
    pub slot: u8,
    pub interval_ms: u64,
    pub assembly_instance: u32,
    pub tags: Vec<crate::ethernet_ip::EnipTagConfig>,
}

// ── Voltrus PLC Runtime device types ──

fn default_voltrus_transport() -> String {
    "grpc".to_string()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VoltrusPlcDevice {
    pub id: i64,
    pub name: String,
    pub url: String,
    #[serde(default = "default_voltrus_transport")]
    pub transport: String,
    #[serde(default = "default_interval_ms")]
    pub interval_ms: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VoltrusPlcTag {
    pub id: i64,
    pub device_id: i64,
    pub variable_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub data_type: String,
    pub created_at: i64,
}
