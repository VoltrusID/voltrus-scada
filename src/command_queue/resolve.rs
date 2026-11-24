//! Which device point a queued write goes to, and in what type.
//!
//! A tag (data source) names its point either in `source_config` (an address,
//! node id, object, ...) or by its key matching a point configured on the
//! device. Both resolve to the same configuration the poller reads with, so a
//! value is written in the type, scale and area it is read in. A point that
//! cannot be resolved fails the command: guessing a type writes a wrong value.
#![cfg_attr(
    not(any(
        feature = "modbus",
        feature = "s7",
        feature = "opcua",
        feature = "ethernet-ip",
        feature = "bacnet"
    )),
    allow(dead_code)
)]

use serde_json::Value;

fn config(source_config: &str) -> Value {
    serde_json::from_str(source_config).unwrap_or(Value::Null)
}

fn str_field<'a>(cfg: &'a Value, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|n| cfg.get(*n).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn u64_field(cfg: &Value, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|n| cfg.get(*n).and_then(Value::as_u64))
}

/// The names a tag may give its point by: the key itself, the key without a
/// `<device>.` prefix (how the cache names metrics) and the part after a
/// `<device>:` prefix (how protocol tags auto-create their data sources).
fn point_names<'a>(key: &'a str, device_ref: &str) -> Vec<&'a str> {
    let mut names = vec![key];
    if let Some(rest) = key
        .strip_prefix(device_ref)
        .and_then(|r| r.strip_prefix('.'))
    {
        names.push(rest);
    }
    if let Some((_, rest)) = key.split_once(':') {
        names.push(rest);
    }
    names
}

fn find_by_name<'a, T>(
    items: &'a [T],
    key: &str,
    device_ref: &str,
    name: impl Fn(&T) -> &str,
) -> Option<&'a T> {
    point_names(key, device_ref)
        .into_iter()
        .find_map(|n| items.iter().find(|item| name(item) == n))
}

// ── Modbus ──

#[cfg(feature = "modbus")]
pub(super) fn modbus_register(
    key: &str,
    device_ref: &str,
    source_config: &str,
    registers: &[crate::models::RegisterConfig],
) -> Result<crate::models::RegisterConfig, String> {
    use crate::models::{RegisterConfig, RegisterType};

    let cfg = config(source_config);
    let Some(address) = u64_field(&cfg, &["address", "register"]) else {
        return find_by_name(registers, key, device_ref, |r| &r.name)
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Tag '{key}' matches no register of Modbus device '{device_ref}' and its \
                     source_config has no address"
                )
            });
    };
    let address = u16::try_from(address)
        .map_err(|_| format!("Modbus address {address} in source_config is above 65535"))?;
    let zero_based = cfg
        .get("zero_based")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let area = str_field(&cfg, &["register_area", "area"]).map(str::to_string);
    let wanted = RegisterConfig {
        name: String::new(),
        address,
        reg_type: RegisterType::Uint16,
        scale: 1.0,
        register_area: String::new(),
        zero_based,
    };
    // A 5-digit reference names its area: 3xxxx input, 4xxxx holding.
    let area = area.or_else(|| match address {
        _ if zero_based => None,
        40001.. => Some("holding".into()),
        30001.. => Some("input".into()),
        _ => None,
    });

    let at_address: Vec<&RegisterConfig> = registers
        .iter()
        .filter(|r| r.protocol_address() == wanted.protocol_address())
        .filter(|r| area.as_ref().is_none_or(|a| &r.register_area == a))
        .collect();
    match at_address.as_slice() {
        [one] => return Ok((*one).clone()),
        [] => {}
        several => {
            let names = point_names(key, device_ref);
            return several
                .iter()
                .find(|r| names.contains(&r.name.as_str()))
                .map(|r| (*r).clone())
                .ok_or_else(|| {
                    format!(
                        "Modbus device '{device_ref}' has {} registers at address {address}; \
                         give the tag the register's name as key or its register_area",
                        several.len()
                    )
                });
        }
    }

    // Not configured on the device: source_config must say what it is.
    let type_name = str_field(&cfg, &["type", "register_type"]).ok_or_else(|| {
        format!(
            "No register at address {address} is configured on Modbus device '{device_ref}', \
             and the tag's source_config gives no \"type\""
        )
    })?;
    let reg_type = match type_name.to_lowercase().as_str() {
        "int16" => RegisterType::Int16,
        "uint16" => RegisterType::Uint16,
        "int32" => RegisterType::Int32,
        "float32" => RegisterType::Float32,
        "float64" => RegisterType::Float64,
        other => return Err(format!("Unknown Modbus register type '{other}'")),
    };
    Ok(RegisterConfig {
        name: key.to_string(),
        address,
        reg_type,
        scale: cfg.get("scale").and_then(Value::as_f64).unwrap_or(1.0),
        register_area: area.unwrap_or_else(|| "holding".into()),
        zero_based,
    })
}

// ── S7 ──

#[cfg(feature = "s7")]
pub(super) fn s7_tag(
    key: &str,
    device_ref: &str,
    source_config: &str,
    tags: &[crate::models::S7TagRow],
) -> Result<crate::s7::S7Tag, String> {
    use crate::s7::S7Tag;

    let cfg = config(source_config);
    let configured_type = str_field(&cfg, &["data_type", "type"]);
    let row_type = |tag: &S7Tag| {
        tags.iter()
            .find(|row| {
                S7Tag::resolve(&row.tag_address, &row.data_type).is_ok_and(|t| {
                    t.area == tag.area
                        && t.db_number == tag.db_number
                        && t.offset == tag.offset
                        && t.bit_offset == tag.bit_offset
                })
            })
            .map(|row| row.data_type.as_str())
    };

    if let Some(address) = str_field(&cfg, &["address", "tag_address"]) {
        let addressed = crate::s7::S7Client::parse_tag_address(address)
            .map_err(|e| format!("S7 address in source_config: {e}"))?;
        let data_type = configured_type
            .or_else(|| row_type(&addressed))
            .unwrap_or("");
        return S7Tag::resolve(address, data_type).map_err(|e| e.to_string());
    }

    if let Some(offset) = u64_field(&cfg, &["offset"]) {
        let offset =
            u16::try_from(offset).map_err(|_| format!("S7 offset {offset} is above 65535"))?;
        let area = str_field(&cfg, &["area"]).unwrap_or("DB");
        let db_number = u64_field(&cfg, &["db_number", "db"]).unwrap_or(0);
        let db_number = u16::try_from(db_number)
            .map_err(|_| format!("S7 DB number {db_number} is above 65535"))?;
        let bit = u64_field(&cfg, &["bit", "bit_offset"])
            .unwrap_or(0)
            .min(255) as u8;
        let data_type = match configured_type {
            Some(t) => t.to_string(),
            None => {
                // The type of a configured tag at the same place, else nothing to go on.
                let probe = S7Tag::from_parts(area, db_number, offset, bit, "byte")
                    .map_err(|e| e.to_string())?;
                let bool_probe = S7Tag::from_parts(area, db_number, offset, bit, "bool")
                    .map_err(|e| e.to_string())?;
                row_type(&probe)
                    .or_else(|| row_type(&bool_probe))
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        format!(
                            "S7 tag '{key}' gives no data_type in source_config and no tag of \
                             the device is at {area} {db_number} offset {offset}"
                        )
                    })?
            }
        };
        return S7Tag::from_parts(area, db_number, offset, bit, &data_type)
            .map_err(|e| e.to_string());
    }

    let row = find_by_name(tags, key, device_ref, |t| &t.name).ok_or_else(|| {
        format!("Tag '{key}' matches no tag of S7 device {device_ref} and has no address")
    })?;
    S7Tag::resolve(&row.tag_address, &row.data_type).map_err(|e| e.to_string())
}

// ── OPC UA ──

#[cfg(feature = "opcua")]
pub(super) fn opcua_node_id(
    key: &str,
    device_ref: &str,
    source_config: &str,
    nodes: &[crate::models::OpcUaNode],
) -> Result<String, String> {
    let cfg = config(source_config);
    if let Some(id) = str_field(&cfg, &["node_id", "nodeId"]) {
        return Ok(id.to_string());
    }
    find_by_name(nodes, key, device_ref, |n| &n.name)
        .map(|n| n.node_id.clone())
        .ok_or_else(|| {
            format!("Tag '{key}' matches no node of OPC UA device {device_ref} and has no node_id")
        })
}

// ── EtherNet/IP ──

/// The CIP tag name and configured data type (empty when none is set).
#[cfg(feature = "ethernet-ip")]
pub(super) fn enip_tag(
    key: &str,
    device_ref: &str,
    source_config: &str,
    tags: &[crate::models::EnipTag],
) -> Result<(String, String), String> {
    let cfg = config(source_config);
    let configured_type = str_field(&cfg, &["data_type", "type"]);
    if let Some(name) = str_field(&cfg, &["tag_name", "tagName"]) {
        let row_type = tags
            .iter()
            .find(|t| t.tag_name == name)
            .map(|t| t.data_type.as_str());
        let data_type = configured_type.or(row_type).unwrap_or("");
        return Ok((name.to_string(), data_type.to_string()));
    }
    let row = find_by_name(tags, key, device_ref, |t| &t.name).ok_or_else(|| {
        format!("Tag '{key}' matches no tag of EtherNet/IP device {device_ref} and has no tag_name")
    })?;
    let data_type = configured_type.unwrap_or(&row.data_type);
    Ok((row.tag_name.clone(), data_type.to_string()))
}

// ── BACnet ──

#[cfg(feature = "bacnet")]
#[derive(Debug, PartialEq)]
pub(super) struct BacnetPoint {
    pub object_type: String,
    pub object_instance: u32,
    pub property: String,
    pub priority: u8,
}

#[cfg(feature = "bacnet")]
pub(super) fn bacnet_point(
    key: &str,
    device_ref: &str,
    source_config: &str,
    objects: &[crate::models::BacnetObject],
) -> Result<BacnetPoint, String> {
    let cfg = config(source_config);
    let priority = match u64_field(&cfg, &["priority"]) {
        Some(p) => crate::bacnet::check_priority(u8::try_from(p).unwrap_or(0))?,
        None => crate::bacnet::DEFAULT_WRITE_PRIORITY,
    };
    let property = str_field(&cfg, &["property"]);

    let object_type = str_field(&cfg, &["object_type"]);
    let object_instance = u64_field(&cfg, &["object_instance"]);
    if let (Some(object_type), Some(instance)) = (object_type, object_instance) {
        return Ok(BacnetPoint {
            object_type: object_type.to_string(),
            object_instance: u32::try_from(instance)
                .map_err(|_| format!("BACnet object instance {instance} is out of range"))?,
            property: property.unwrap_or("presentValue").to_string(),
            priority,
        });
    }
    let object = find_by_name(objects, key, device_ref, |o| &o.name).ok_or_else(|| {
        format!(
            "Tag '{key}' matches no object of BACnet device {device_ref} and its source_config \
             has no object_type and object_instance"
        )
    })?;
    Ok(BacnetPoint {
        object_type: object.object_type.clone(),
        object_instance: object.object_instance,
        property: property.unwrap_or(&object.property).to_string(),
        priority,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_names_strip_device_prefixes() {
        assert_eq!(point_names("power", "m1"), vec!["power"]);
        assert_eq!(point_names("m1.power", "m1"), vec!["m1.power", "power"]);
        assert_eq!(point_names("PLC1:valve", "3"), vec!["PLC1:valve", "valve"]);
    }

    #[cfg(feature = "modbus")]
    mod modbus {
        use super::*;
        use crate::models::{RegisterConfig, RegisterType};

        fn reg(name: &str, address: u16, reg_type: RegisterType, area: &str) -> RegisterConfig {
            RegisterConfig {
                name: name.into(),
                address,
                reg_type,
                scale: 0.1,
                register_area: area.into(),
                zero_based: false,
            }
        }

        fn device() -> Vec<RegisterConfig> {
            vec![
                reg("setpoint", 40011, RegisterType::Float32, "holding"),
                reg("pump", 5, RegisterType::Uint16, "coil"),
                reg("level", 30011, RegisterType::Int16, "input"),
            ]
        }

        #[test]
        fn by_key() {
            let r = modbus_register("setpoint", "m1", "{}", &device()).unwrap();
            assert_eq!(r.reg_type, RegisterType::Float32);
            assert_eq!(r.scale, 0.1);
            let r = modbus_register("m1.pump", "m1", "", &device()).unwrap();
            assert_eq!(r.register_area, "coil");
        }

        #[test]
        fn by_address_uses_configured_register() {
            let r = modbus_register("x", "m1", r#"{"address": 40011}"#, &device()).unwrap();
            assert_eq!(r.name, "setpoint");
            assert_eq!(r.reg_type, RegisterType::Float32);
            // Same offset 10, but the 3xxxx reference is the input register.
            let r = modbus_register("x", "m1", r#"{"register": 30011}"#, &device()).unwrap();
            assert_eq!(r.name, "level");
            let r = modbus_register(
                "x",
                "m1",
                r#"{"address": 5, "register_area": "coil"}"#,
                &device(),
            )
            .unwrap();
            assert_eq!(r.name, "pump");
        }

        #[test]
        fn same_offset_in_two_areas_needs_a_hint() {
            let err = modbus_register("x", "m1", r#"{"address": 10}"#, &device()).unwrap_err();
            assert!(err.contains("2 registers"), "{err}");
            let r = modbus_register("level", "m1", r#"{"address": 10}"#, &device()).unwrap();
            assert_eq!(r.name, "level");
        }

        #[test]
        fn unconfigured_address_needs_a_type() {
            let err = modbus_register("x", "m1", r#"{"address": 40100}"#, &device()).unwrap_err();
            assert!(err.contains("\"type\""), "{err}");
            let r = modbus_register(
                "x",
                "m1",
                r#"{"address": 40100, "type": "int32", "scale": 0.5}"#,
                &device(),
            )
            .unwrap();
            assert_eq!(r.reg_type, RegisterType::Int32);
            assert_eq!(r.scale, 0.5);
            assert_eq!(r.register_area, "holding");
            assert_eq!(r.protocol_address(), 99);
        }

        #[test]
        fn unknown_key_fails() {
            assert!(modbus_register("nope", "m1", "{}", &device()).is_err());
        }
    }

    #[cfg(feature = "s7")]
    mod s7 {
        use super::*;
        use crate::models::S7TagRow;
        use crate::s7::{S7Area, S7DataType};

        fn row(name: &str, address: &str, data_type: &str) -> S7TagRow {
            S7TagRow {
                id: 1,
                device_id: 3,
                tag_address: address.into(),
                name: name.into(),
                display_name: String::new(),
                data_type: data_type.into(),
                created_at: 0,
            }
        }

        fn device() -> Vec<S7TagRow> {
            vec![
                row("temp_sp", "DB1.DBD4", "real"),
                row("count", "DB1.DBW8", "int"),
                row("run", "DB1.DBX10.3", ""),
            ]
        }

        #[test]
        fn by_key_uses_configured_type() {
            let t = s7_tag("PLC1:temp_sp", "3", "{}", &device()).unwrap();
            assert_eq!(t.data_type, S7DataType::Real);
            assert_eq!(t.offset, 4);
            let t = s7_tag("count", "3", "{}", &device()).unwrap();
            assert_eq!(t.data_type, S7DataType::Int);
            let t = s7_tag("run", "3", "{}", &device()).unwrap();
            assert_eq!(t.data_type, S7DataType::Bool);
            assert_eq!(t.bit_offset, 3);
        }

        #[test]
        fn address_in_source_config() {
            let t = s7_tag("x", "3", r#"{"address": "DB1.DBD4"}"#, &device()).unwrap();
            assert_eq!(
                t.data_type,
                S7DataType::Real,
                "type of the tag at that address"
            );
            let t = s7_tag(
                "x",
                "3",
                r#"{"address": "MD20", "data_type": "dint"}"#,
                &device(),
            )
            .unwrap();
            assert_eq!(t.area, S7Area::Marker);
            assert_eq!(t.data_type, S7DataType::DInt);
            let t = s7_tag("x", "3", r#"{"address": "MD20"}"#, &device()).unwrap();
            assert_eq!(t.data_type, S7DataType::DWord);
        }

        #[test]
        fn legacy_offset_form_needs_a_type() {
            let t = s7_tag(
                "x",
                "3",
                r#"{"area": "DB", "db_number": 1, "offset": 4}"#,
                &device(),
            )
            .unwrap();
            assert_eq!(t.data_type, S7DataType::Real);
            let err = s7_tag(
                "x",
                "3",
                r#"{"area": "DB", "db_number": 1, "offset": 40}"#,
                &device(),
            )
            .unwrap_err();
            assert!(err.contains("data_type"), "{err}");
            let t = s7_tag(
                "x",
                "3",
                r#"{"area": "M", "offset": 2, "bit": 5, "data_type": "bool"}"#,
                &device(),
            )
            .unwrap();
            assert_eq!(t.area, S7Area::Marker);
            assert_eq!(t.bit_offset, 5);
        }

        #[test]
        fn type_must_fit_address() {
            assert!(
                s7_tag(
                    "x",
                    "3",
                    r#"{"address": "DB1.DBW0", "data_type": "real"}"#,
                    &device()
                )
                .is_err()
            );
        }

        #[test]
        fn unknown_key_fails() {
            assert!(s7_tag("nope", "3", "{}", &device()).is_err());
        }
    }

    #[cfg(feature = "opcua")]
    #[test]
    fn opcua_node_by_config_or_name() {
        let nodes = vec![crate::models::OpcUaNode {
            id: 1,
            device_id: 2,
            node_id: "ns=2;s=Setpoint".into(),
            name: "setpoint".into(),
            display_name: String::new(),
            data_type: String::new(),
            created_at: 0,
        }];
        assert_eq!(
            opcua_node_id("x", "2", r#"{"node_id": "ns=2;i=7"}"#, &nodes).unwrap(),
            "ns=2;i=7"
        );
        assert_eq!(
            opcua_node_id("dev:setpoint", "2", "{}", &nodes).unwrap(),
            "ns=2;s=Setpoint"
        );
        assert!(opcua_node_id("other", "2", "{}", &nodes).is_err());
    }

    #[cfg(feature = "ethernet-ip")]
    #[test]
    fn enip_tag_and_type() {
        let tags = vec![crate::models::EnipTag {
            id: 1,
            device_id: 4,
            tag_name: "Line1.Speed".into(),
            name: "speed".into(),
            display_name: String::new(),
            data_type: "REAL".into(),
            assembly_offset: 0,
            assembly_length: 0,
            created_at: 0,
        }];
        assert_eq!(
            enip_tag("speed", "4", "{}", &tags).unwrap(),
            ("Line1.Speed".to_string(), "REAL".to_string())
        );
        assert_eq!(
            enip_tag("x", "4", r#"{"tag_name": "Line1.Speed"}"#, &tags).unwrap(),
            ("Line1.Speed".to_string(), "REAL".to_string())
        );
        assert_eq!(
            enip_tag("x", "4", r#"{"tagName": "Other"}"#, &tags).unwrap(),
            ("Other".to_string(), String::new())
        );
        assert!(enip_tag("x", "4", "{}", &tags).is_err());
    }

    #[cfg(feature = "bacnet")]
    #[test]
    fn bacnet_point_priority_and_lookup() {
        let objects = vec![crate::models::BacnetObject {
            id: 1,
            device_id: 5,
            object_type: "binaryOutput".into(),
            object_instance: 3,
            property: "presentValue".into(),
            name: "fan".into(),
            display_name: String::new(),
            cov_increment: 0.0,
            created_at: 0,
        }];
        let p = bacnet_point("fan", "5", "{}", &objects).unwrap();
        assert_eq!(p.object_type, "binaryOutput");
        assert_eq!(p.object_instance, 3);
        assert_eq!(p.priority, crate::bacnet::DEFAULT_WRITE_PRIORITY);

        let p = bacnet_point(
            "x",
            "5",
            r#"{"object_type": "analogValue", "object_instance": 9, "priority": 12}"#,
            &objects,
        )
        .unwrap();
        assert_eq!(p.object_instance, 9);
        assert_eq!(p.property, "presentValue");
        assert_eq!(p.priority, 12);

        assert!(bacnet_point("fan", "5", r#"{"priority": 6}"#, &objects).is_err());
        assert!(bacnet_point("x", "5", r#"{"object_type": "analogValue"}"#, &objects).is_err());
    }
}
