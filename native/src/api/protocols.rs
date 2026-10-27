//! Mirrors of the protocol device managers' REST answers (`src/api/
//! modbus_devices.rs`, `opcua_devices.rs`, `s7_devices.rs`,
//! `enip_devices.rs`, `dnp3_devices.rs`, `bacnet_devices/`,
//! `simulation.rs`), in the server's shapes.
//!
//! The console's device tables are generic over JSON rows: [`rows`] decodes
//! an answer through its typed mirror (so a wrong shape is an error and the
//! server's defaults apply) and hands back each row as a JSON object.
//!
//! Server facts worth knowing (all read from the handlers):
//! - Every OPC UA, S7, EtherNet/IP, DNP3, BACnet and Voltrus-PLC handler checks
//!   for the admin role itself; demo and integration users pass the router and
//!   get `403 {"error":"Admin access required"}`.
//! - Handler failures are `500 {"error": …}`, validation included.
//! - `GET /api/v1/devices/{id}/simulate` answers `{simulation_mode,
//!   sim_profile_id, sim_config}` for a row of the `devices` table and 404
//!   for any other id (a Modbus device has none).
use std::time::Duration;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Method, VoltrusApi};

impl VoltrusApi {
    /// `method path` with an optional JSON body, waiting up to `timeout`
    /// instead of the client's 10 s: network scans, discovery and browsing
    /// take as long as the devices they wait for.
    pub async fn send_slow(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value> {
        let request = self.req(method, path)?.timeout(timeout);
        let request = match body {
            Some(body) => request.json(body),
            None => request,
        };
        Self::decode(request.send().await?).await
    }
}

/// Whether a failed call means the server has no such route (404, 405):
/// an edition's route the core router does not serve.
pub fn not_routed(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<super::ServerError>()
        .is_some_and(|error| {
            matches!(
                error.status,
                reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
            )
        })
}

/// Decode `answer` as a list of `T` and give each back as a JSON object.
pub fn rows<T: DeserializeOwned + Serialize>(answer: Value) -> Result<Vec<Value>, String> {
    let typed: Vec<T> = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    typed
        .iter()
        .map(|row| serde_json::to_value(row).map_err(|error| error.to_string()))
        .collect()
}

/// Decode `answer[key]` (`{"device":…, "points": [...]}`) as a list of `T`.
pub fn rows_at<T: DeserializeOwned + Serialize>(
    mut answer: Value,
    key: &str,
) -> Result<Vec<Value>, String> {
    match answer.get_mut(key) {
        Some(list) => rows::<T>(list.take()),
        None => Err(format!("the answer has no `{key}`")),
    }
}

/// `201 {"ok": true, "id": …}`: a number for most devices, the given text
/// for a Modbus device.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CreatedId {
    #[serde(default)]
    pub ok: bool,
    pub id: Value,
}

impl CreatedId {
    /// The id as a path segment.
    pub fn text(&self) -> String {
        match &self.id {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        }
    }
}

// ── Modbus ───────────────────────────────────────────────────────────────

/// One row of `GET /api/v1/modbus-devices` (a hand-built object with
/// `register_count`), and the `device` of `GET /api/v1/modbus-devices/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModbusDeviceRow {
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
    /// In the list only.
    pub register_count: usize,
}

/// `voltrus::models::ModbusRegister`, a row of the device's `registers`.
/// A register created with an explicit `offset` comes back as
/// `address: <offset>, zero_based: true`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModbusRegisterRow {
    pub id: i64,
    pub device_id: String,
    pub name: String,
    pub address: u16,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
    pub sort_order: i64,
    pub zero_based: bool,
}

impl ModbusRegisterRow {
    /// The register as `ModbusRegisterInput` takes it back (`PUT
    /// …/registers` replaces the whole list).
    pub fn input(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "address": self.address,
            "zero_based": self.zero_based,
            "register_type": self.register_type,
            "register_area": self.register_area,
            "scale": self.scale,
        })
    }
}

/// `GET /api/v1/modbus-devices/status`: one entry per poller the server
/// runs (any protocol's); `running` is `status == "online"`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct PollerStatus {
    pub device_id: String,
    pub running: bool,
    pub status: String,
}

/// `POST /api/v1/modbus-devices/scan` answers one of these per address;
/// `address` is already a 5-digit reference (holding +40001, input
/// +30001, coil +1, discrete +10001).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ScannedRegister {
    pub address: u16,
    pub raw_value: Option<u16>,
    pub raw_value2: Option<u16>,
    pub changed: bool,
    pub readable: bool,
    pub likely_type: String,
    pub interpreted_value: Option<f64>,
}

/// The 0-based offset of a scanned register's 5-digit reference in
/// `area`, which is what a register should be created with (the web
/// client's `address` + `zero_based: false` misplaces coils and discrete
/// inputs).
pub fn scanned_offset(address: u16, area: &str) -> u16 {
    let base: u16 = match area {
        "input" => 30001,
        "coil" => 1,
        "discrete" => 10001,
        _ => 40001,
    };
    address.wrapping_sub(base)
}

/// `POST /api/v1/modbus-devices/scan-network` answers `{devices, count}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModbusNetworkScan {
    pub devices: Vec<DiscoveredModbus>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DiscoveredModbus {
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub responsive_registers: Vec<DiscoveredRegister>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DiscoveredRegister {
    pub address: u16,
    pub value: f64,
    pub raw: Vec<u16>,
}

/// `voltrus::models::SimProfile`: `config` is JSON in a string.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimProfile {
    pub id: i64,
    pub name: String,
    pub data_type: String,
    pub config: String,
    pub created_at: i64,
}

// ── OPC UA ───────────────────────────────────────────────────────────────

/// One row of `GET /api/v1/opcua/devices` (no password; `node_count` is the
/// monitored nodes).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpcUaDeviceRow {
    pub id: i64,
    pub name: String,
    pub url: String,
    /// `"None"` or `"basic256sha256"`: the server's enum names.
    pub security_policy: String,
    /// `"None"`, `"Sign"` or `"signandencrypt"`.
    pub security_mode: String,
    pub username: Option<String>,
    pub interval_ms: u64,
    pub enabled: bool,
    pub created_at: i64,
    pub node_count: usize,
}

/// `GET /api/v1/opcua/devices/{id}/nodes` rows; `id` is the row id the
/// DELETE takes, `node_id` the OPC UA node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpcUaNodeRow {
    pub id: i64,
    pub device_id: i64,
    pub node_id: String,
    pub name: String,
    pub display_name: String,
    pub data_type: String,
    pub created_at: i64,
}

/// `POST /api/v1/opcua/devices/{id}/browse`: one level under the Objects
/// folder.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct OpcUaBrowse {
    pub nodes: Vec<OpcUaBrowsedNode>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct OpcUaBrowsedNode {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    /// `Variable` or `Object`.
    pub node_class: String,
}

// ── S7 ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct S7DeviceRow {
    pub id: i64,
    pub name: String,
    /// `host:port`.
    pub url: String,
    pub rack: u8,
    pub slot: u8,
    pub interval_ms: u64,
    pub enabled: bool,
    pub created_at: i64,
    pub tag_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct S7TagRow {
    pub id: i64,
    pub device_id: i64,
    pub tag_address: String,
    pub name: String,
    pub display_name: String,
    pub data_type: String,
    pub created_at: i64,
}

/// `POST /api/v1/s7/devices/{id}/browse`: whether the PLC answered, and a
/// fixed catalogue of address areas with examples.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct S7Browse {
    pub connected: bool,
    pub url: String,
    pub rack: u8,
    pub slot: u8,
    pub areas: Vec<S7Area>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct S7Area {
    pub area: String,
    pub description: String,
    pub address_format: String,
    pub examples: Vec<String>,
}

// ── EtherNet/IP ──────────────────────────────────────────────────────────

/// The list leaves out `assembly_instance`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnipDeviceRow {
    pub id: i64,
    pub name: String,
    pub url: String,
    pub slot: u8,
    pub interval_ms: u64,
    pub enabled: bool,
    pub assembly_instance: u32,
    pub created_at: i64,
    pub tag_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EnipTagRow {
    pub id: i64,
    pub device_id: i64,
    pub tag_name: String,
    pub name: String,
    pub display_name: String,
    pub data_type: String,
    pub assembly_offset: u32,
    pub assembly_length: u32,
    pub created_at: i64,
}

/// `POST /api/v1/enip/devices/{id}/browse`: the PLC's symbols.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct EnipBrowse {
    pub tags: Vec<EnipBrowsedTag>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct EnipBrowsedTag {
    pub tag_name: String,
    /// Upper case: `BOOL`, `SINT`, `INT`, `DINT`, `REAL`, `STRING`,
    /// `DWORD`, `LINT`.
    pub data_type: String,
    pub element_count: u16,
}

// ── DNP3 ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dnp3DeviceRow {
    pub id: i64,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub master_address: u16,
    pub outstation_address: u16,
    pub enable_unsolicited: bool,
    pub integrity_poll_interval_secs: u64,
    pub event_scan_interval_ms: Option<u64>,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
    /// In the list only.
    pub point_count: usize,
}

/// The `points` of `GET /api/v1/dnp3/devices/{id}` (there is no points
/// list route).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dnp3PointRow {
    pub id: i64,
    pub device_id: i64,
    pub group_num: u8,
    pub variation: u8,
    pub point_index: u16,
    pub name: String,
    pub display_name: String,
    pub data_type: String,
    pub scale: f64,
    pub deadband: f64,
    pub created_at: i64,
}

/// `GET /api/v1/dnp3/group-variations`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct GroupVariation {
    pub group: u8,
    pub variation: u8,
    pub name: String,
}

/// The server's fixed `common_group_variations()`, in its order; the e2e
/// test checks the server still answers these.
pub const DNP3_GROUP_VARIATIONS: [(u8, u8, &str); 8] = [
    (1, 2, "Binary Input"),
    (10, 2, "Binary Output Status"),
    (20, 1, "Counter Input"),
    (21, 1, "Frozen Counter"),
    (30, 4, "Analog Input (float32)"),
    (30, 1, "Analog Input (int32)"),
    (32, 3, "Analog Output Status (float32)"),
    (40, 3, "Analog Output Block (float32)"),
];

/// `POST /api/v1/dnp3/scan` answers a bare list.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DiscoveredDnp3 {
    pub ip: String,
    pub port: u16,
    pub outstation_address: u16,
}

/// `POST /api/v1/dnp3/devices/{id}/integrity` and `/freeze`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ActionAnswer {
    pub ok: bool,
    pub message: Option<String>,
}

// ── BACnet ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BacnetDeviceRow {
    pub id: i64,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub device_instance: u32,
    pub interval_ms: u64,
    pub cov_enabled: bool,
    pub cov_lifetime: u32,
    pub enabled: bool,
    pub created_at: i64,
    pub object_count: usize,
}

/// `GET /api/v1/bacnet/devices/{id}/objects` rows (a bare list).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BacnetObjectRow {
    pub id: i64,
    pub device_id: i64,
    pub object_type: String,
    pub object_instance: u32,
    pub property: String,
    pub name: String,
    pub display_name: String,
    pub cov_increment: f64,
    pub created_at: i64,
}

/// `POST /api/v1/bacnet/devices/{id}/browse`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct BacnetBrowse {
    pub objects: Vec<BacnetBrowsedObject>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct BacnetBrowsedObject {
    pub object_type: String,
    pub object_instance: u32,
    pub object_name: Option<String>,
}

/// `POST /api/v1/bacnet/discover` answers `{devices, count}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct BacnetDiscovery {
    pub devices: Vec<DiscoveredBacnet>,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct DiscoveredBacnet {
    pub device_instance: u32,
    pub ip: String,
    pub port: u16,
    pub vendor_name: Option<String>,
    pub model_name: Option<String>,
    pub firmware_version: Option<String>,
    pub object_list_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn modbus_rows_and_registers_decode_in_the_servers_shape() {
        let list = json!([{"id": "plts_1", "name": "PLTS 1", "ip": "10.0.0.5", "port": 502,
            "unit_id": 1, "interval_ms": 1000, "endianness": "big", "enabled": true,
            "created_at": 1_700_000_000, "updated_at": 1_700_000_001, "register_count": 3}]);
        let devices = rows::<ModbusDeviceRow>(list).unwrap();
        assert_eq!(devices[0]["id"], "plts_1");
        assert_eq!(devices[0]["register_count"], 3);
        let detail = json!({"device": {"id": "plts_1", "name": "PLTS 1", "ip": "10.0.0.5", "port": 502,
            "unit_id": 1, "interval_ms": 1000, "endianness": "big", "enabled": true,
            "created_at": 1, "updated_at": 2},
            "registers": [{"id": 9, "device_id": "plts_1", "name": "voltage", "address": 0,
                "register_type": "float32", "register_area": "holding", "scale": 1.0,
                "sort_order": 0, "zero_based": true}]});
        let registers = rows_at::<ModbusRegisterRow>(detail, "registers").unwrap();
        let register: ModbusRegisterRow = serde_json::from_value(registers[0].clone()).unwrap();
        assert!(register.zero_based);
        assert_eq!(
            register.input(),
            json!({"name": "voltage", "address": 0, "zero_based": true,
                "register_type": "float32", "register_area": "holding", "scale": 1.0})
        );
        assert!(rows_at::<ModbusRegisterRow>(json!({}), "registers").is_err());
        assert!(rows::<ModbusDeviceRow>(json!({"devices": []})).is_err());
    }

    #[test]
    fn modbus_status_scans_and_profiles_decode() {
        let status: Vec<PollerStatus> = serde_json::from_value(
            json!([{"device_id": "plts_1", "running": false, "status": "offline"}]),
        )
        .unwrap();
        assert_eq!(status[0].status, "offline");
        let scan: Vec<ScannedRegister> = serde_json::from_value(json!([
            {"address": 40001, "raw_value": 17000, "raw_value2": 17001, "changed": true,
             "readable": true, "likely_type": "float32", "interpreted_value": 12.5},
            {"address": 40003, "raw_value": null, "raw_value2": null, "changed": false,
             "readable": false, "likely_type": "uint16", "interpreted_value": null}
        ]))
        .unwrap();
        assert_eq!(scan[0].interpreted_value, Some(12.5));
        assert!(!scan[1].readable);
        assert_eq!(scanned_offset(40001, "holding"), 0);
        assert_eq!(scanned_offset(30011, "input"), 10);
        assert_eq!(scanned_offset(1, "coil"), 0);
        assert_eq!(scanned_offset(10006, "discrete"), 5);
        let network: ModbusNetworkScan = serde_json::from_value(json!({"devices": [
            {"ip": "10.0.0.7", "port": 502, "unit_id": 1,
             "responsive_registers": [{"address": 0, "value": 12.0, "raw": [12]}]}], "count": 1}))
        .unwrap();
        assert_eq!(network.devices[0].responsive_registers[0].raw, [12]);
        let profiles =
            rows::<SimProfile>(json!([{"id": 1, "name": "Sine", "data_type": "sinusoidal",
            "config": "{\"amplitude\":10}", "created_at": 5}]))
            .unwrap();
        assert_eq!(profiles[0]["config"], "{\"amplitude\":10}");
    }

    #[test]
    fn opcua_nodes_and_browse_decode() {
        let nodes =
            rows::<OpcUaNodeRow>(json!([{"id": 4, "device_id": 2, "node_id": "ns=2;s=Temp",
            "name": "Temp", "display_name": "Temperature", "data_type": "", "created_at": 1}]))
            .unwrap();
        assert_eq!(nodes[0]["node_id"], "ns=2;s=Temp");
        let browse: OpcUaBrowse =
            serde_json::from_value(json!({"nodes": [{"node_id": "ns=2;s=Foo",
            "browse_name": "2:Foo", "display_name": "Foo", "node_class": "Variable"}], "count": 1}))
            .unwrap();
        assert_eq!(browse.nodes[0].browse_name, "2:Foo");
        let created: CreatedId = serde_json::from_value(json!({"ok": true, "id": 7})).unwrap();
        assert_eq!(created.text(), "7");
        let created: CreatedId =
            serde_json::from_value(json!({"ok": true, "id": "plts_9"})).unwrap();
        assert_eq!(created.text(), "plts_9");
    }

    #[test]
    fn s7_and_enip_rows_and_browse_decode() {
        let devices = rows::<S7DeviceRow>(json!([{"id": 1, "name": "CPU", "url": "10.0.0.9:102",
            "rack": 0, "slot": 1, "interval_ms": 1000, "enabled": true, "created_at": 1, "tag_count": 2}]))
        .unwrap();
        assert_eq!(devices[0]["slot"], 1);
        let tags = rows::<S7TagRow>(json!([{"id": 3, "device_id": 1, "tag_address": "DB1.DBD0",
            "name": "flow", "display_name": "", "data_type": "float32", "created_at": 1}]))
        .unwrap();
        assert_eq!(tags[0]["tag_address"], "DB1.DBD0");
        let browse: S7Browse =
            serde_json::from_value(json!({"connected": false, "url": "10.0.0.9:102",
            "rack": 0, "slot": 1, "areas": [{"area": "DB", "description": "Data Block",
            "address_format": "DB<n>.DBX<byte>.<bit>", "examples": ["DB1.DBX0.0", "DB1.DBW100"]}],
            "count": 6}))
            .unwrap();
        assert_eq!(browse.areas[0].examples.len(), 2);
        // The list leaves out assembly_instance.
        let devices =
            rows::<EnipDeviceRow>(json!([{"id": 1, "name": "L85", "url": "10.0.0.8:44818",
            "slot": 0, "interval_ms": 1000, "enabled": true, "created_at": 1, "tag_count": 0}]))
            .unwrap();
        assert_eq!(devices[0]["assembly_instance"], 0);
        let tags = rows::<EnipTagRow>(json!([{"id": 2, "device_id": 1, "tag_name": "Program.Flow",
            "name": "flow", "display_name": "", "data_type": "real32", "assembly_offset": 0,
            "assembly_length": 0, "created_at": 1}]))
        .unwrap();
        assert_eq!(tags[0]["tag_name"], "Program.Flow");
        let browse: EnipBrowse = serde_json::from_value(json!({"tags": [{"tag_name": "Flow",
            "data_type": "REAL", "element_count": 1}], "count": 1}))
        .unwrap();
        assert_eq!(browse.tags[0].data_type, "REAL");
    }

    #[test]
    fn dnp3_rows_points_and_scans_decode() {
        let devices = rows::<Dnp3DeviceRow>(json!([{"id": 1, "name": "RTU", "ip": "10.0.0.3",
            "port": 20000, "master_address": 1, "outstation_address": 10,
            "enable_unsolicited": true, "integrity_poll_interval_secs": 300,
            "event_scan_interval_ms": null, "enabled": true, "created_at": 1,
            "updated_at": 1, "point_count": 0}]))
        .unwrap();
        assert_eq!(devices[0]["event_scan_interval_ms"], Value::Null);
        let detail = json!({"device": {"id": 1, "name": "RTU", "ip": "10.0.0.3", "port": 20000,
            "master_address": 1, "outstation_address": 10, "enable_unsolicited": true,
            "integrity_poll_interval_secs": 300, "event_scan_interval_ms": 500,
            "enabled": true, "created_at": 1, "updated_at": 1},
            "points": [{"id": 5, "device_id": 1, "group_num": 30, "variation": 4,
                "point_index": 0, "name": "kw", "display_name": "kW", "data_type": "float64",
                "scale": 1.0, "deadband": 0.0, "created_at": 1}]});
        let points = rows_at::<Dnp3PointRow>(detail, "points").unwrap();
        assert_eq!(points[0]["group_num"], 30);
        let variations: Vec<GroupVariation> =
            serde_json::from_value(json!([{"group": 1, "variation": 2, "name": "Binary Input"}]))
                .unwrap();
        assert_eq!(variations[0].name, DNP3_GROUP_VARIATIONS[0].2);
        let found: Vec<DiscoveredDnp3> = serde_json::from_value(
            json!([{"ip": "10.0.0.4", "port": 20000, "outstation_address": 10}]),
        )
        .unwrap();
        assert_eq!(found[0].outstation_address, 10);
        let answer: ActionAnswer =
            serde_json::from_value(json!({"ok": true, "message": "Freeze triggered"})).unwrap();
        assert_eq!(answer.message.as_deref(), Some("Freeze triggered"));
    }

    #[test]
    fn bacnet_rows_objects_browse_and_discovery_decode() {
        let devices = rows::<BacnetDeviceRow>(json!([{"id": 1, "name": "AHU", "ip": "10.0.0.2",
            "port": 47808, "device_instance": 7, "interval_ms": 5000, "cov_enabled": false,
            "cov_lifetime": 3600, "enabled": true, "created_at": 1, "object_count": 1}]))
        .unwrap();
        assert_eq!(devices[0]["device_instance"], 7);
        let objects = rows::<BacnetObjectRow>(json!([{"id": 3, "device_id": 1,
            "object_type": "analogInput", "object_instance": 1, "property": "presentValue",
            "name": "T", "display_name": "", "cov_increment": 0.0, "created_at": 1}]))
        .unwrap();
        assert_eq!(objects[0]["property"], "presentValue");
        let browse: BacnetBrowse = serde_json::from_value(json!({"objects": [{"object_type":
            "analogInput", "object_instance": 1, "object_name": null}], "count": 1}))
        .unwrap();
        assert_eq!(browse.objects[0].object_name, None);
        let found: BacnetDiscovery =
            serde_json::from_value(json!({"devices": [{"device_instance": 123,
            "ip": "10.0.0.5", "port": 47808, "vendor_name": "Acme", "model_name": null,
            "firmware_version": null, "object_list_count": null}], "count": 1}))
            .unwrap();
        assert_eq!(found.devices[0].vendor_name.as_deref(), Some("Acme"));
    }
}
