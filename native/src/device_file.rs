//! Device files from Modbus Poll and BACnet Explorer ("Export to SCADA" ›
//! Voltrus SCADA device), as pure functions: a port of the web client's
//! `client/src/deviceFileImport.ts`, message for message.
//!
//! - Modbus: the body of `POST /api/v1/modbus-devices` with its registers.
//!   A register carries `address` (legacy: 0-based below 30001, a 5-digit
//!   reference from 30001) or `offset` (an explicit 0-based offset within
//!   `register_area`, up to 65535); the explicit form is what Modbus Poll
//!   exports.
//! - BACnet: the body of `POST /api/v1/bacnet/devices` plus `objects`, each
//!   the body of `POST /api/v1/bacnet/devices/{id}/objects`.
//!
//! A file holds one device, an array of devices, or `{ "devices": [...] }`.

use serde::Serialize;
use serde_json::{Map, Value};

/// A register as the server's create body takes it: exactly one of
/// `address` and `offset`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModbusImportRegister {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u16>,
    pub register_type: String,
    pub register_area: String,
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModbusImportDevice {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u32,
    pub endianness: String,
    pub registers: Vec<ModbusImportRegister>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BacnetImportObject {
    pub object_type: String,
    pub object_instance: u32,
    pub property: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BacnetImportDevice {
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub device_instance: u32,
    pub interval_ms: u32,
    pub objects: Vec<BacnetImportObject>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFileKind {
    Modbus,
    Bacnet,
}

impl DeviceFileKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Modbus => "Modbus",
            Self::Bacnet => "BACnet",
        }
    }
}

/// The devices a file holds, and one message per rejected device or
/// register (a device with a bad register still imports, without it).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedDeviceFile<T> {
    pub devices: Vec<T>,
    pub errors: Vec<String>,
}

pub const MODBUS_TYPES: [&str; 5] = ["int16", "uint16", "int32", "float32", "float64"];
pub const MODBUS_AREAS: [&str; 4] = ["holding", "input", "coil", "discrete"];
pub const BACNET_TYPES: [&str; 9] = [
    "analogInput",
    "analogOutput",
    "analogValue",
    "binaryInput",
    "binaryOutput",
    "binaryValue",
    "multiStateInput",
    "multiStateOutput",
    "multiStateValue",
];

/// The highest BACnet instance number (22 bits).
const BACNET_INSTANCE_MAX: u64 = 4_194_303;
const INTERVAL_MAX: u64 = 3_600_000;

/// A whole number within `min..=max` (JSON's `2000.0` counts, as in JS).
fn int_in(value: &Value, min: i64, max: i64) -> Option<i64> {
    let number = value.as_number()?;
    let whole = number.as_i64().or_else(|| {
        let float = number.as_f64()?;
        #[allow(clippy::cast_possible_truncation)]
        (float.fract() == 0.0 && float.abs() < 9.0e15).then_some(float as i64)
    })?;
    (min..=max).contains(&whole).then_some(whole)
}

/// A string field, trimmed; anything else reads as empty.
fn text(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default()
}

/// A value as JavaScript's `String(v)` writes it, for the messages.
fn js(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => match (number.as_i64(), number.as_f64()) {
            (Some(whole), _) => whole.to_string(),
            (None, Some(float)) if float.fract() == 0.0 && float.abs() < 1e21 => {
                format!("{float:.0}")
            }
            _ => number.to_string(),
        },
        Value::Null => "null".to_owned(),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(items) => items.iter().map(js).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `raw[key] ?? default`: a missing or null field takes the default.
fn or<'a>(raw: &'a Map<String, Value>, key: &str, default: &'a Value) -> &'a Value {
    match raw.get(key) {
        None | Some(Value::Null) => default,
        Some(value) => value,
    }
}

fn device_list(text: &str) -> (Vec<Value>, Option<String>) {
    let doc: Value = match serde_json::from_str(text) {
        Ok(doc) => doc,
        Err(error) => return (Vec::new(), Some(format!("Not valid JSON: {error}"))),
    };
    match doc {
        Value::Array(list) => (list, None),
        Value::Object(mut object) => match object.remove("devices") {
            Some(Value::Array(list)) => (list, None),
            Some(other) => {
                object.insert("devices".to_owned(), other);
                (vec![Value::Object(object)], None)
            }
            None => (vec![Value::Object(object)], None),
        },
        _ => (
            Vec::new(),
            Some(
                "Expected a device object, an array of devices or { \"devices\": [...] }"
                    .to_owned(),
            ),
        ),
    }
}

/// The device's name for messages: its name, or its place in the file.
fn label_of(raw: &Value, index: usize) -> String {
    let name = text(raw.get("name"));
    if raw.is_object() && !name.is_empty() {
        name
    } else {
        format!("device {}", index + 1)
    }
}

/// Which kind of device file `text` is, from the fields only that kind has:
/// Modbus devices list `registers`, BACnet devices `objects` or a
/// `device_instance`.
pub fn device_file_kind(text: &str) -> Option<DeviceFileKind> {
    let (list, _) = device_list(text);
    let first = list.iter().find_map(Value::as_object)?;
    if first.get("registers").is_some_and(Value::is_array) {
        return Some(DeviceFileKind::Modbus);
    }
    if first.get("objects").is_some_and(Value::is_array) || first.contains_key("device_instance") {
        return Some(DeviceFileKind::Bacnet);
    }
    None
}

#[allow(clippy::too_many_lines)]
pub fn parse_modbus_device_file(text_in: &str) -> ParsedDeviceFile<ModbusImportDevice> {
    let (list, error) = device_list(text_in);
    let mut out = ParsedDeviceFile {
        devices: Vec::new(),
        errors: error.into_iter().collect(),
    };
    let port_default = Value::from(502);
    let unit_default = Value::from(1);
    let interval_default = Value::from(1000);
    let scale_default = Value::from(1);
    for (index, raw) in list.iter().enumerate() {
        let label = label_of(raw, index);
        let Some(raw) = raw.as_object() else {
            out.errors.push(format!("{label}: not an object"));
            continue;
        };
        let id = text(raw.get("id"));
        let ip = text(raw.get("ip"));
        if id.is_empty() {
            out.errors.push(format!("{label}: id is missing"));
            continue;
        }
        if ip.is_empty() {
            out.errors.push(format!(
                "{label}: ip is missing (a serial device needs the address of a Modbus TCP gateway)"
            ));
            continue;
        }
        let port = or(raw, "port", &port_default);
        let unit = or(raw, "unit_id", &unit_default);
        let interval = or(raw, "interval_ms", &interval_default);
        let Some(port) = int_in(port, 1, 65535) else {
            out.errors
                .push(format!("{label}: port {} is not 1-65535", js(port)));
            continue;
        };
        let Some(unit) = int_in(unit, 0, 255) else {
            out.errors
                .push(format!("{label}: unit_id {} is not 0-255", js(unit)));
            continue;
        };
        let Some(interval) = int_in(interval, 1, INTERVAL_MAX as i64) else {
            out.errors.push(format!(
                "{label}: interval_ms {} is not a positive number",
                js(interval)
            ));
            continue;
        };
        let endianness = match text(raw.get("endianness")) {
            empty if empty.is_empty() => "big".to_owned(),
            given => given,
        };
        if !["big", "little", "swapped"].contains(&endianness.as_str()) {
            out.errors.push(format!(
                "{label}: endianness '{endianness}' is not big, little or swapped"
            ));
            continue;
        }
        let Some(raw_registers) = raw.get("registers").and_then(Value::as_array) else {
            out.errors.push(format!("{label}: registers is missing"));
            continue;
        };
        let mut registers = Vec::new();
        for (at, register) in raw_registers.iter().enumerate() {
            let Some(register) = register.as_object() else {
                out.errors
                    .push(format!("{label}: register {} is not an object", at + 1));
                continue;
            };
            let name = text(register.get("name"));
            let kind = match text(register.get("register_type")) {
                empty if empty.is_empty() => "float32".to_owned(),
                given => given,
            };
            let area = match text(register.get("register_area")) {
                empty if empty.is_empty() => "holding".to_owned(),
                given => given,
            };
            let scale = or(register, "scale", &scale_default);
            let address = register.get("address");
            let offset = register.get("offset");
            let address_ok = address.and_then(|value| int_in(value, 0, 65535));
            let offset_ok = offset.and_then(|value| int_in(value, 0, 65535));
            let problem = if name.is_empty() {
                Some(format!("{label}: register {} has no name", at + 1))
            } else if address.is_some() && offset.is_some() {
                Some(format!("{label}: {name} has both address and offset"))
            } else if address.is_none() && offset.is_none() {
                Some(format!("{label}: {name} has no address or offset"))
            } else if let (Some(value), None) = (address, address_ok) {
                Some(format!(
                    "{label}: {name} has address {}, not 0-65535",
                    js(value)
                ))
            } else if let (Some(value), None) = (offset, offset_ok) {
                Some(format!(
                    "{label}: {name} has offset {}, not 0-65535",
                    js(value)
                ))
            } else if !MODBUS_TYPES.contains(&kind.as_str()) {
                Some(format!(
                    "{label}: {name} has type '{kind}' (use {})",
                    MODBUS_TYPES.join(", ")
                ))
            } else if !MODBUS_AREAS.contains(&area.as_str()) {
                Some(format!(
                    "{label}: {name} has area '{area}' (use {})",
                    MODBUS_AREAS.join(", ")
                ))
            } else if !scale.is_number() {
                Some(format!("{label}: {name} has a scale that is not a number"))
            } else {
                None
            };
            if let Some(problem) = problem {
                out.errors.push(problem);
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let to_u16 = |value: Option<i64>| value.map(|value| value as u16);
            registers.push(ModbusImportRegister {
                name,
                address: if offset.is_some() {
                    None
                } else {
                    to_u16(address_ok)
                },
                offset: to_u16(offset_ok),
                register_type: kind,
                register_area: area,
                scale: scale.as_f64().unwrap_or(1.0),
            });
        }
        let name = match text(raw.get("name")) {
            empty if empty.is_empty() => id.clone(),
            given => given,
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        out.devices.push(ModbusImportDevice {
            id,
            name,
            ip,
            port: port as u16,
            unit_id: unit as u8,
            interval_ms: interval as u32,
            endianness,
            registers,
        });
    }
    out
}

pub fn parse_bacnet_device_file(text_in: &str) -> ParsedDeviceFile<BacnetImportDevice> {
    let (list, error) = device_list(text_in);
    let mut out = ParsedDeviceFile {
        devices: Vec::new(),
        errors: error.into_iter().collect(),
    };
    let port_default = Value::from(47808);
    let instance_default = Value::from(0);
    let interval_default = Value::from(1000);
    let no_objects = Vec::new();
    for (index, raw) in list.iter().enumerate() {
        let label = label_of(raw, index);
        let Some(raw) = raw.as_object() else {
            out.errors.push(format!("{label}: not an object"));
            continue;
        };
        let name = text(raw.get("name"));
        let ip = text(raw.get("ip"));
        if name.is_empty() {
            out.errors.push(format!("{label}: name is missing"));
            continue;
        }
        if ip.is_empty() {
            out.errors.push(format!("{label}: ip is missing"));
            continue;
        }
        let port = or(raw, "port", &port_default);
        let instance = or(raw, "device_instance", &instance_default);
        let interval = or(raw, "interval_ms", &interval_default);
        let Some(port) = int_in(port, 1, 65535) else {
            out.errors
                .push(format!("{label}: port {} is not 1-65535", js(port)));
            continue;
        };
        let Some(instance) = int_in(instance, 0, BACNET_INSTANCE_MAX as i64) else {
            out.errors.push(format!(
                "{label}: device_instance {} is not 0-4194303",
                js(instance)
            ));
            continue;
        };
        let Some(interval) = int_in(interval, 1, INTERVAL_MAX as i64) else {
            out.errors.push(format!(
                "{label}: interval_ms {} is not a positive number",
                js(interval)
            ));
            continue;
        };
        let mut objects = Vec::new();
        let raw_objects = raw
            .get("objects")
            .and_then(Value::as_array)
            .unwrap_or(&no_objects);
        for (at, object) in raw_objects.iter().enumerate() {
            let Some(object) = object.as_object() else {
                out.errors
                    .push(format!("{label}: object {} is not an object", at + 1));
                continue;
            };
            let kind = text(object.get("object_type"));
            let object_name = text(object.get("name"));
            let raw_instance = object.get("object_instance").unwrap_or(&Value::Null);
            let instance = int_in(raw_instance, 0, BACNET_INSTANCE_MAX as i64);
            if !BACNET_TYPES.contains(&kind.as_str()) {
                out.errors.push(format!(
                    "{label}: object {} has type '{kind}' (use {})",
                    at + 1,
                    BACNET_TYPES.join(", ")
                ));
            } else if instance.is_none() {
                let shown = if object.contains_key("object_instance") {
                    js(raw_instance)
                } else {
                    "undefined".to_owned()
                };
                out.errors.push(format!(
                    "{label}: object {} has instance {shown}, not 0-4194303",
                    at + 1
                ));
            } else if object_name.is_empty() {
                out.errors
                    .push(format!("{label}: object {} has no name", at + 1));
            } else {
                let property = match text(object.get("property")) {
                    empty if empty.is_empty() => "presentValue".to_owned(),
                    given => given,
                };
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                objects.push(BacnetImportObject {
                    object_type: kind,
                    object_instance: instance.unwrap_or(0) as u32,
                    property,
                    name: object_name,
                });
            }
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        out.devices.push(BacnetImportDevice {
            name,
            ip,
            port: port as u16,
            device_instance: instance as u32,
            interval_ms: interval as u32,
            objects,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The web client's `tests/fixtures/modbus-poll-big-map.json`: Modbus
    /// Poll's export of a tag list with addresses above 30000.
    const BIG_MAP: &str = r#"{
      "id": "big_map", "name": "Big map", "ip": "10.1.2.3", "port": 1502,
      "unit_id": 1, "interval_ms": 2000, "endianness": "swapped",
      "registers": [
        { "name": "Edge_30000", "register_area": "holding", "offset": 30000, "register_type": "uint16", "scale": 1 },
        { "name": "Float_35000", "register_area": "holding", "offset": 35000, "register_type": "float32", "scale": 0.5 },
        { "name": "Count_40000", "register_area": "holding", "offset": 40000, "register_type": "int32", "scale": 1 },
        { "name": "Input_30001", "register_area": "input", "offset": 30001, "register_type": "int16", "scale": 1 },
        { "name": "Input_65534", "register_area": "input", "offset": 65534, "register_type": "uint16", "scale": 1 },
        { "name": "Coil_40000", "register_area": "coil", "offset": 40000, "register_type": "uint16", "scale": 1 },
        { "name": "Discrete_50000", "register_area": "discrete", "offset": 50000, "register_type": "uint16", "scale": 1 },
        { "name": "Low_5", "register_area": "holding", "offset": 5, "register_type": "float64", "scale": 1 }
      ]
    }"#;

    fn register(
        name: &str,
        area: &str,
        address: Option<u16>,
        offset: Option<u16>,
        kind: &str,
        scale: f64,
    ) -> ModbusImportRegister {
        ModbusImportRegister {
            name: name.into(),
            address,
            offset,
            register_type: kind.into(),
            register_area: area.into(),
            scale,
        }
    }

    #[test]
    fn a_modbus_poll_export_imports_every_tag_with_its_explicit_offset() {
        assert_eq!(device_file_kind(BIG_MAP), Some(DeviceFileKind::Modbus));
        let parsed = parse_modbus_device_file(BIG_MAP);
        assert_eq!(parsed.errors, Vec::<String>::new());
        assert_eq!(parsed.devices.len(), 1);
        let device = &parsed.devices[0];
        assert_eq!(
            (
                device.id.as_str(),
                device.name.as_str(),
                device.ip.as_str(),
                device.port,
                device.unit_id,
                device.interval_ms,
                device.endianness.as_str()
            ),
            ("big_map", "Big map", "10.1.2.3", 1502, 1, 2000, "swapped")
        );
        let tags = [
            ("Edge_30000", "holding", 30000, "uint16", 1.0),
            ("Float_35000", "holding", 35000, "float32", 0.5),
            ("Count_40000", "holding", 40000, "int32", 1.0),
            ("Input_30001", "input", 30001, "int16", 1.0),
            ("Input_65534", "input", 65534, "uint16", 1.0),
            ("Coil_40000", "coil", 40000, "uint16", 1.0),
            ("Discrete_50000", "discrete", 50000, "uint16", 1.0),
            ("Low_5", "holding", 5, "float64", 1.0),
        ];
        let expected: Vec<_> = tags
            .iter()
            .map(|(name, area, offset, kind, scale)| {
                register(name, area, None, Some(*offset), kind, *scale)
            })
            .collect();
        assert_eq!(device.registers, expected);
        // The create body names the offset, never an address.
        let body = serde_json::to_value(&device.registers[0]).unwrap();
        assert_eq!(
            body,
            json!({"name": "Edge_30000", "offset": 30000, "register_type": "uint16", "register_area": "holding", "scale": 1.0})
        );
    }

    #[test]
    fn the_legacy_address_form_still_imports() {
        let file = json!({
            "id": "old", "name": "Old", "ip": "10.0.0.1",
            "registers": [
                {"name": "v", "address": 40001, "register_type": "float32", "register_area": "holding", "scale": 1},
                {"name": "w", "address": 12}
            ]
        })
        .to_string();
        let parsed = parse_modbus_device_file(&file);
        assert_eq!(parsed.errors, Vec::<String>::new());
        assert_eq!(
            parsed.devices[0].registers,
            vec![
                register("v", "holding", Some(40001), None, "float32", 1.0),
                register("w", "holding", Some(12), None, "float32", 1.0),
            ]
        );
        // The defaults of the web form.
        let device = &parsed.devices[0];
        assert_eq!(
            (device.port, device.unit_id, device.interval_ms),
            (502, 1, 1000)
        );
        assert_eq!(device.endianness, "big");
    }

    fn register_errors(register: Value) -> Vec<String> {
        let file = json!({"id": "d", "name": "D", "ip": "1.1.1.1", "registers": [register]});
        parse_modbus_device_file(&file.to_string()).errors
    }

    #[test]
    fn a_register_needs_exactly_one_of_address_and_offset_within_range() {
        assert!(register_errors(json!({"name": "a"}))[0].contains("no address or offset"));
        assert!(
            register_errors(json!({"name": "a", "address": 1, "offset": 1}))[0]
                .contains("both address and offset")
        );
        assert!(
            register_errors(json!({"name": "a", "offset": 65536}))[0]
                .contains("offset 65536, not 0-65535")
        );
        assert!(register_errors(json!({"name": "a", "offset": -1}))[0].contains("offset -1"));
        assert!(register_errors(json!({"name": "a", "offset": 65535})).is_empty());
    }

    #[test]
    fn bacnet_files_still_import() {
        let file = json!({
            "name": "AHU", "ip": "10.0.0.2", "device_instance": 7,
            "objects": [{"name": "T", "object_type": "analogInput", "object_instance": 1}]
        })
        .to_string();
        assert_eq!(device_file_kind(&file), Some(DeviceFileKind::Bacnet));
        let parsed = parse_bacnet_device_file(&file);
        assert_eq!(parsed.errors, Vec::<String>::new());
        let device = &parsed.devices[0];
        assert_eq!(device.objects[0].property, "presentValue");
        assert_eq!((device.port, device.interval_ms), (47808, 1000));
    }

    #[test]
    fn a_file_holds_one_device_a_list_or_a_devices_object() {
        let one = json!({"id": "a", "ip": "1.1.1.1", "registers": []});
        let list = json!([one, {"id": "b", "ip": "1.1.1.2", "registers": []}]);
        let wrapped = json!({ "devices": list });
        assert_eq!(parse_modbus_device_file(&one.to_string()).devices.len(), 1);
        assert_eq!(parse_modbus_device_file(&list.to_string()).devices.len(), 2);
        assert_eq!(
            parse_modbus_device_file(&wrapped.to_string()).devices.len(),
            2
        );
        // A nameless device is named after its id.
        assert_eq!(
            parse_modbus_device_file(&one.to_string()).devices[0].name,
            "a"
        );
        let broken = parse_modbus_device_file("{ nope");
        assert!(broken.errors[0].starts_with("Not valid JSON: "));
        assert!(broken.devices.is_empty());
        assert_eq!(
            parse_modbus_device_file("42").errors,
            ["Expected a device object, an array of devices or { \"devices\": [...] }"]
        );
    }

    #[test]
    fn the_kind_comes_from_the_fields_only_that_kind_has() {
        assert_eq!(device_file_kind("{ nope"), None);
        assert_eq!(device_file_kind("[1, 2]"), None);
        assert_eq!(device_file_kind(r#"{"name": "x"}"#), None);
        assert_eq!(
            device_file_kind(r#"[1, {"objects": []}]"#),
            Some(DeviceFileKind::Bacnet),
            "the first object decides"
        );
        assert_eq!(
            device_file_kind(r#"{"device_instance": null}"#),
            Some(DeviceFileKind::Bacnet)
        );
    }

    #[test]
    fn a_device_is_rejected_with_the_reason_and_the_rest_import() {
        let file = json!([
            "text",
            {"name": "NoId", "ip": "1.1.1.1", "registers": []},
            {"id": "noip", "registers": []},
            {"id": "p", "ip": "1.1.1.1", "port": 70000, "registers": []},
            {"id": "u", "ip": "1.1.1.1", "unit_id": 256, "registers": []},
            {"id": "i", "ip": "1.1.1.1", "interval_ms": 0, "registers": []},
            {"id": "e", "ip": "1.1.1.1", "endianness": "middle", "registers": []},
            {"id": "r", "ip": "1.1.1.1"},
            {"id": "ok", "ip": "1.1.1.1", "port": 1502.0, "registers": [
                "x",
                {"address": 1},
                {"name": "t", "address": 1, "register_type": "bool"},
                {"name": "a", "address": 1, "register_area": "memory"},
                {"name": "s", "address": 1, "scale": "2"},
                {"name": "n", "address": null},
                {"name": "good", "address": 7}
            ]}
        ])
        .to_string();
        let parsed = parse_modbus_device_file(&file);
        assert_eq!(
            parsed.errors,
            [
                "device 1: not an object",
                "NoId: id is missing",
                "device 3: ip is missing (a serial device needs the address of a Modbus TCP gateway)",
                "device 4: port 70000 is not 1-65535",
                "device 5: unit_id 256 is not 0-255",
                "device 6: interval_ms 0 is not a positive number",
                "device 7: endianness 'middle' is not big, little or swapped",
                "device 8: registers is missing",
                "device 9: register 1 is not an object",
                "device 9: register 2 has no name",
                "device 9: t has type 'bool' (use int16, uint16, int32, float32, float64)",
                "device 9: a has area 'memory' (use holding, input, coil, discrete)",
                "device 9: s has a scale that is not a number",
                "device 9: n has address null, not 0-65535",
            ]
        );
        assert_eq!(parsed.devices.len(), 1);
        assert_eq!(parsed.devices[0].port, 1502);
        assert_eq!(parsed.devices[0].registers.len(), 1);
    }

    #[test]
    fn bacnet_devices_and_objects_are_checked_like_the_web() {
        let file = json!({"devices": [
            {"ip": "1.1.1.1"},
            {"name": "NoIp"},
            {"name": "Inst", "ip": "1.1.1.1", "device_instance": 4_194_304},
            {"name": "AHU", "ip": "1.1.1.1", "objects": [
                3,
                {"object_type": "lightingOutput", "object_instance": 1, "name": "L"},
                {"object_type": "analogValue", "name": "N"},
                {"object_type": "analogValue", "object_instance": 2},
                {"object_type": "binaryOutput", "object_instance": 9, "name": " Fan ", "property": "statusFlags"}
            ]}
        ]})
        .to_string();
        let parsed = parse_bacnet_device_file(&file);
        assert_eq!(
            parsed.errors,
            [
                "device 1: name is missing",
                "NoIp: ip is missing",
                "Inst: device_instance 4194304 is not 0-4194303",
                "AHU: object 1 is not an object",
                "AHU: object 2 has type 'lightingOutput' (use analogInput, analogOutput, analogValue, binaryInput, binaryOutput, binaryValue, multiStateInput, multiStateOutput, multiStateValue)",
                "AHU: object 3 has instance undefined, not 0-4194303",
                "AHU: object 4 has no name",
            ]
        );
        assert_eq!(
            parsed.devices,
            vec![BacnetImportDevice {
                name: "AHU".into(),
                ip: "1.1.1.1".into(),
                port: 47808,
                device_instance: 0,
                interval_ms: 1000,
                objects: vec![BacnetImportObject {
                    object_type: "binaryOutput".into(),
                    object_instance: 9,
                    property: "statusFlags".into(),
                    name: "Fan".into(),
                }],
            }]
        );
    }
}
