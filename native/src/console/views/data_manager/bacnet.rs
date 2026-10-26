//! Data Manager › BACnet, after the web client's `BacnetDeviceManager.tsx`:
//! devices (IP, port 47808, instance, interval, COV) with their objects,
//! a Who-Is discovery that adds what answers, a browse of a device's
//! object list, start and stop, and BACnet Explorer device files imported
//! from the Import… prompt or dropped on the section. Objects are read as
//! the bare list the server answers (the web client expects
//! `{objects: [...]}` and shows none). Every BACnet handler checks for the
//! admin role.

use std::time::Duration;

use gpui::Context;
use serde_json::{Map, Value, json};
use voltrus_scada_native::api::protocols::{
    BacnetBrowse, BacnetDeviceRow, BacnetDiscovery, BacnetObjectRow, rows,
};
use voltrus_scada_native::device_file::{BACNET_TYPES, DeviceFileKind};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;

use super::device_form::{
    self, Cell, Child, ChildApi, Field, Kind, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Scope,
    Tool, ToolKind, Values, calls, choice, col, device_path, flag, int, mono, name_of, optional,
    row_id, section_hooks, text, value_text,
};
use crate::console::Console;
use crate::console::pages::Role;

const TYPES: &[(&str, &str)] = &[
    ("analogInput", "analogInput"),
    ("analogOutput", "analogOutput"),
    ("analogValue", "analogValue"),
    ("binaryInput", "binaryInput"),
    ("binaryOutput", "binaryOutput"),
    ("binaryValue", "binaryValue"),
    ("multiStateInput", "multiStateInput"),
    ("multiStateOutput", "multiStateOutput"),
    ("multiStateValue", "multiStateValue"),
];

fn link(row: &Value) -> String {
    format!("bacnet_{}", row_id(row))
}

/// A blank display name is the name, as the web form sends it.
fn finish_object(body: &mut Map<String, Value>, _: bool) {
    if body.get("display_name").and_then(Value::as_str) == Some("") {
        let name = body.get("name").cloned().unwrap_or_default();
        body.insert("display_name".into(), name);
    }
}

fn discover_request(_: Option<&Value>, values: &Values) -> Result<(String, Option<Value>), String> {
    Ok((
        "/api/v1/bacnet/discover".into(),
        Some(json!({ "timeout_secs": timeout_secs(values) })),
    ))
}

fn timeout_secs(values: &Values) -> u64 {
    values
        .get("timeout_secs")
        .and_then(|secs| secs.trim().parse().ok())
        .unwrap_or(5)
        .clamp(1, 30)
}

fn discover_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let found: BacnetDiscovery =
        serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!(
            "{} devices answered Who-Is; Space selects, ↵ adds.",
            found.count
        )),
        rows: found
            .devices
            .into_iter()
            .map(|device| ProbeRow {
                label: format!("{}:{}", device.ip, device.port),
                detail: format!(
                    "instance {} · {}",
                    device.device_instance,
                    device.vendor_name.as_deref().unwrap_or("unknown vendor")
                ),
                payload: json!({
                    "name": format!("bacnet_{}", device.ip.replace('.', "_")),
                    "ip": device.ip,
                    "port": device.port,
                    "device_instance": device.device_instance,
                    "enabled": true,
                }),
            })
            .collect(),
    })
}

static DISCOVER: ProbeSpec = ProbeSpec {
    title: "Discover BACnet devices",
    intro: |_| "Broadcasts Who-Is and lists the devices that answer within the timeout.".to_owned(),
    fields: &[int("timeout_secs", "Timeout (s)", "5", 1, 30)],
    run_label: "Discover",
    request: discover_request,
    timeout: |values| Duration::from_secs(timeout_secs(values) + 15),
    decode: discover_decode,
    multi: true,
    pick_label: "Add",
    on_pick: device_form::pick_devices,
    empty: "No BACnet device answered on this network.",
    auto: false,
};

fn browse_request(device: Option<&Value>, _: &Values) -> Result<(String, Option<Value>), String> {
    let device = device.ok_or("no device")?;
    Ok((format!("{}/browse", device_path(&PROTOCOL, device)), None))
}

fn browse_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let browse: BacnetBrowse = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!("{} objects; ↵ adds one to poll.", browse.count)),
        rows: browse
            .objects
            .into_iter()
            .map(|object| {
                let name = object.object_name.clone().unwrap_or_else(|| {
                    format!("{}_{}", object.object_type, object.object_instance)
                });
                ProbeRow {
                    label: format!("{} {}", object.object_type, object.object_instance),
                    detail: if BACNET_TYPES.contains(&object.object_type.as_str()) {
                        object.object_name.clone().unwrap_or_default()
                    } else {
                        "not a type the server polls".to_owned()
                    },
                    payload: json!({
                        "object_type": object.object_type,
                        "object_instance": object.object_instance,
                        "name": name,
                    }),
                }
            })
            .collect(),
    })
}

static BROWSE: ProbeSpec = ProbeSpec {
    title: "Browse the device's objects",
    intro: |device| {
        device.map_or_else(String::new, |device| {
            format!(
                "{} · {}:{} · instance {}",
                name_of(device),
                value_text(device, "ip"),
                value_text(device, "port"),
                value_text(device, "device_instance")
            )
        })
    },
    fields: &[],
    run_label: "Browse again",
    request: browse_request,
    timeout: |_| Duration::from_secs(45),
    decode: browse_decode,
    multi: false,
    pick_label: "Add object",
    on_pick: device_form::pick_child,
    empty: "The device lists no objects.",
    auto: true,
};

static OBJECTS: Child = Child {
    title: "Objects",
    noun: "object",
    api: ChildApi::Rest {
        segment: "objects",
        within: None,
    },
    decode: rows::<BacnetObjectRow>,
    columns: &[
        col("Name", 0.0, Cell::Key("name"), true, 0),
        col("Display name", 140.0, Cell::Key("display_name"), false, 3),
        col("Type", 120.0, Cell::Key("object_type"), true, 0),
        col("Instance", 64.0, Cell::Key("object_instance"), true, 0),
        col("Property", 96.0, Cell::Key("property"), true, 2),
        col("COV inc.", 64.0, Cell::Key("cov_increment"), true, 1),
    ],
    fields: &[
        mono("name", "Name", "object_name"),
        optional("display_name", "Display name", "Supply air temperature"),
        choice("object_type", "Object type", TYPES, "analogInput"),
        int("object_instance", "Instance", "0", 0, 4_194_303),
        Field {
            kind: Kind::Text {
                required: true,
                mono: true,
            },
            default: "presentValue",
            ..optional("property", "Property", "presentValue")
        },
        Field {
            kind: Kind::Float,
            default: "0",
            ..optional("cov_increment", "COV increment", "0")
        },
    ],
    finish: Some(finish_object),
    makes_tags: false,
};

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "bacnet",
    title: "BACnet",
    noun: "BACnet device",
    icon: Icon::Factory,
    list_id: "data.bacnet.list",
    calls: calls!("bacnet"),
    base: "/api/v1/bacnet/devices",
    decode: rows::<BacnetDeviceRow>,
    write_role: Role::Admin,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("IP", 120.0, Cell::Key("ip"), true, 0),
        col("Port", 56.0, Cell::Key("port"), true, 4),
        col("Instance", 72.0, Cell::Key("device_instance"), true, 0),
        col("Interval", 72.0, Cell::Ms("interval_ms"), true, 5),
        col("COV", 44.0, Cell::OnOff("cov_enabled"), false, 3),
        col("Objects", 60.0, Cell::Key("object_count"), true, 2),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 0),
    ],
    search: &["name", "id", "ip"],
    fields: &[
        text("name", "Name", "BACnet controller A"),
        mono("ip", "IP address", "192.168.1.100"),
        int("port", "Port", "47808", 1, 65535),
        int("device_instance", "Device instance", "0", 0, 4_194_303),
        int("interval_ms", "Interval (ms)", "5000", 100, 3_600_000),
        int("cov_lifetime", "COV lifetime (s)", "3600", 0, 4_294_967_295),
        flag("cov_enabled", "COV subscriptions", "false"),
        flag("enabled", "Enabled", "true"),
    ],
    finish: None,
    child: Some(&OBJECTS),
    tools: &[
        Tool {
            label: "Start polling",
            icon: Icon::Play,
            scope: Scope::Device,
            kind: ToolKind::Post {
                segment: "start",
                done: "polling started",
            },
            when: None,
        },
        Tool {
            label: "Stop polling",
            icon: Icon::Pause,
            scope: Scope::Device,
            kind: ToolKind::Post {
                segment: "stop",
                done: "polling stopped",
            },
            when: None,
        },
        Tool {
            label: "Browse objects",
            icon: Icon::Search,
            scope: Scope::Device,
            kind: ToolKind::Probe(&BROWSE),
            when: None,
        },
        Tool {
            label: "Discover",
            icon: Icon::Radio,
            scope: Scope::Section,
            kind: ToolKind::Probe(&DISCOVER),
            when: None,
        },
    ],
    import: Some(DeviceFileKind::Bacnet),
    duplicate: false,
    about: "A BACnet/IP device the server polls for the objects you pick. Add one with N, discover the network, or drop a BACnet Explorer device file here.",
    unrouted: None,
};

section_hooks!();

pub fn on_open(console: &mut Console, _: &mut Context<Console>) {
    device_form::on_open(&PROTOCOL, console);
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    device_form::on_reply(&PROTOCOL, console, key, reply, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::views::data_manager::device_form::{body, values_of};

    #[test]
    fn the_object_types_are_the_servers() {
        let ours: Vec<&str> = TYPES.iter().map(|(value, _)| *value).collect();
        assert_eq!(ours, BACNET_TYPES);
    }

    #[test]
    fn a_device_and_an_object_go_in_the_servers_shape() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("name", "AHU".into());
        values.insert("ip", "10.0.0.2".into());
        assert_eq!(
            Value::Object(body(PROTOCOL.fields, &values, false).unwrap()),
            json!({"name": "AHU", "ip": "10.0.0.2", "port": 47808, "device_instance": 0,
                "interval_ms": 5000, "cov_lifetime": 3600, "cov_enabled": false, "enabled": true})
        );
        let mut object = values_of(OBJECTS.fields, None);
        object.insert("name", "sat".into());
        let mut sent = body(OBJECTS.fields, &object, false).unwrap();
        finish_object(&mut sent, false);
        assert_eq!(
            Value::Object(sent),
            json!({"name": "sat", "display_name": "sat", "object_type": "analogInput",
                "object_instance": 0, "property": "presentValue", "cov_increment": 0.0})
        );
    }

    #[test]
    fn discovery_and_browse_answer_rows_to_add() {
        let values = values_of(DISCOVER.fields, None);
        assert_eq!(
            discover_request(None, &values).unwrap(),
            (
                "/api/v1/bacnet/discover".to_owned(),
                Some(json!({"timeout_secs": 5}))
            )
        );
        let found = discover_decode(
            json!({"devices": [{"device_instance": 123, "ip": "10.0.0.5", "port": 47808,
                "vendor_name": "Acme", "model_name": null, "firmware_version": null,
                "object_list_count": null}], "count": 1}),
            &values,
        )
        .unwrap();
        assert_eq!(found.rows[0].detail, "instance 123 · Acme");
        assert_eq!(found.rows[0].payload["name"], "bacnet_10_0_0_5");
        let objects = browse_decode(
            json!({"objects": [{"object_type": "analogInput", "object_instance": 1, "object_name": null}],
                "count": 1}),
            &values,
        )
        .unwrap();
        let prefill = device_form::prefill(OBJECTS.fields, &objects.rows[0].payload);
        assert_eq!(prefill["name"], "analogInput_1");
        assert_eq!(prefill["object_instance"], "1");
    }
}
