//! Data Manager › DNP3, after the web client's `Dnp3DeviceManager.tsx`:
//! outstations with their points (group and variation, index, type,
//! scale, deadband), a network scan that adds what answers, and the
//! actions the web client never offered: start, stop, integrity poll and
//! freeze. Points come with the device (`GET /dnp3/devices/{id}`; there is
//! no points list route). Every DNP3 handler checks for the admin role.

use std::time::Duration;

use gpui::Context;
use serde_json::{Map, Value, json};
use voltrus_scada_native::api::protocols::{
    DNP3_GROUP_VARIATIONS, DiscoveredDnp3, Dnp3DeviceRow, Dnp3PointRow, rows,
};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;

use super::device_form::{
    self, Cell, Child, ChildApi, Field, Kind, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Scope,
    Tool, ToolKind, Values, calls, choice, col, flag, int, mono, name_of, optional, row_id,
    section_hooks, text, value_text,
};
use crate::console::Console;
use crate::console::pages::Role;

/// The server's group/variation pairs, as `"30.4"` choices.
const GROUP_VARIATIONS: &[(&str, &str)] = &[
    ("1.2", "G1 V2 · Binary input"),
    ("10.2", "G10 V2 · Binary output status"),
    ("20.1", "G20 V1 · Counter"),
    ("21.1", "G21 V1 · Frozen counter"),
    ("30.4", "G30 V4 · Analog input (float32)"),
    ("30.1", "G30 V1 · Analog input (int32)"),
    ("32.3", "G32 V3 · Analog output status (float32)"),
    ("40.3", "G40 V3 · Analog output block (float32)"),
];

const TYPES: &[(&str, &str)] = &[
    ("float64", "float64"),
    ("float32", "float32"),
    ("int32", "int32"),
    ("int16", "int16"),
    ("uint32", "uint32"),
    ("uint16", "uint16"),
    ("bool", "bool"),
    ("string", "string"),
];

fn link(row: &Value) -> String {
    format!("dnp3_{}", row_id(row))
}

fn group(row: &Value) -> String {
    format!(
        "G{} V{}",
        value_text(row, "group_num"),
        value_text(row, "variation")
    )
}

/// `group_variation: "30.4"` → `group_num: 30, variation: 4`; a blank
/// display name is the name, as the web form sends it.
fn finish_point(body: &mut Map<String, Value>, _: bool) {
    if let Some(Value::String(pair)) = body.remove("group_variation")
        && let Some((group, variation)) = pair.split_once('.')
    {
        body.insert("group_num".into(), json!(group.parse::<u8>().unwrap_or(30)));
        body.insert(
            "variation".into(),
            json!(variation.parse::<u8>().unwrap_or(4)),
        );
    }
    if body.get("display_name").and_then(Value::as_str) == Some("") {
        let name = body.get("name").cloned().unwrap_or_default();
        body.insert("display_name".into(), name);
    }
}

fn scan_request(_: Option<&Value>, values: &Values) -> Result<(String, Option<Value>), String> {
    let range = values.get("range").map_or("", |range| range.trim());
    if !range.contains('/') {
        return Err("Give the range as a CIDR, e.g. 192.168.1.0/24 (at most a /24)".into());
    }
    let port: u16 = values
        .get("port")
        .and_then(|port| port.trim().parse().ok())
        .unwrap_or(20000);
    Ok((
        "/api/v1/dnp3/scan".into(),
        Some(json!({ "range": range, "port": port })),
    ))
}

fn scan_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let found: Vec<DiscoveredDnp3> =
        serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!(
            "{} outstations answered; Space selects, ↵ adds.",
            found.len()
        )),
        rows: found
            .into_iter()
            .map(|device| ProbeRow {
                label: format!("{}:{}", device.ip, device.port),
                detail: format!("outstation {}", device.outstation_address),
                payload: json!({
                    "name": format!("dnp3_{}", device.ip.replace('.', "_")),
                    "ip": device.ip,
                    "port": device.port,
                    "outstation_address": device.outstation_address,
                    "enabled": true,
                }),
            })
            .collect(),
    })
}

static SCAN: ProbeSpec = ProbeSpec {
    title: "Scan for outstations",
    intro: |_| {
        "Probes every host of the range for a DNP3 link layer (2 s each, 50 at a time).".to_owned()
    },
    fields: &[
        Field {
            default: "192.168.1.0/24",
            ..mono("range", "Range (CIDR, at most /24)", "192.168.1.0/24")
        },
        int("port", "Port", "20000", 1, 65535),
    ],
    run_label: "Scan",
    request: scan_request,
    timeout: |_| Duration::from_secs(90),
    decode: scan_decode,
    multi: true,
    pick_label: "Add",
    on_pick: device_form::pick_devices,
    empty: "No outstation answered in this range.",
    auto: false,
};

static POINTS: Child = Child {
    title: "Points",
    noun: "point",
    api: ChildApi::Rest {
        segment: "points",
        within: Some("points"),
    },
    decode: rows::<Dnp3PointRow>,
    columns: &[
        col("Name", 0.0, Cell::Key("name"), true, 0),
        col("Display name", 140.0, Cell::Key("display_name"), false, 3),
        col("Group", 72.0, Cell::Fn(group), true, 0),
        col("Index", 52.0, Cell::Key("point_index"), true, 0),
        col("Type", 64.0, Cell::Key("data_type"), true, 2),
        col("Scale", 56.0, Cell::Key("scale"), true, 1),
        col("Deadband", 64.0, Cell::Key("deadband"), true, 1),
    ],
    fields: &[
        mono("name", "Name", "point_name"),
        optional("display_name", "Display name", "Feeder current"),
        choice(
            "group_variation",
            "Group and variation",
            GROUP_VARIATIONS,
            "30.4",
        ),
        int("point_index", "Index", "0", 0, 65535),
        choice("data_type", "Data type", TYPES, "float64"),
        Field {
            kind: Kind::Float,
            default: "1",
            ..optional("scale", "Scale", "1")
        },
        Field {
            kind: Kind::Float,
            default: "0",
            ..optional("deadband", "Deadband", "0")
        },
    ],
    finish: Some(finish_point),
    makes_tags: false,
};

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "dnp3",
    title: "DNP3",
    noun: "DNP3 outstation",
    icon: Icon::RadioTower,
    list_id: "data.dnp3.list",
    calls: calls!("dnp3"),
    base: "/api/v1/dnp3/devices",
    decode: rows::<Dnp3DeviceRow>,
    write_role: Role::Admin,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("IP", 120.0, Cell::Key("ip"), true, 0),
        col("Port", 56.0, Cell::Key("port"), true, 4),
        col("Master", 56.0, Cell::Key("master_address"), true, 3),
        col("Outstation", 72.0, Cell::Key("outstation_address"), true, 0),
        col("Points", 52.0, Cell::Key("point_count"), true, 2),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 0),
    ],
    search: &["name", "id", "ip"],
    fields: &[
        text("name", "Name", "RTU substation A"),
        mono("ip", "IP address", "192.168.1.100"),
        int("port", "Port", "20000", 1, 65535),
        int("master_address", "Master address", "1", 0, 65519),
        int("outstation_address", "Outstation address", "10", 0, 65519),
        int(
            "integrity_poll_interval_secs",
            "Integrity poll (s)",
            "300",
            1,
            86_400,
        ),
        Field {
            kind: Kind::OptionalInt {
                min: 1,
                max: 3_600_000,
            },
            ..optional("event_scan_interval_ms", "Event scan (ms)", "Optional")
        },
        flag("enable_unsolicited", "Unsolicited responses", "true"),
        flag("enabled", "Enabled", "true"),
    ],
    finish: None,
    child: Some(&POINTS),
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
            label: "Integrity poll",
            icon: Icon::RefreshCw,
            scope: Scope::Device,
            kind: ToolKind::Post {
                segment: "integrity",
                done: "integrity poll asked",
            },
            when: None,
        },
        Tool {
            label: "Freeze counters",
            icon: Icon::Clock,
            scope: Scope::Device,
            kind: ToolKind::Post {
                segment: "freeze",
                done: "freeze asked",
            },
            when: None,
        },
        Tool {
            label: "Scan network",
            icon: Icon::Radio,
            scope: Scope::Section,
            kind: ToolKind::Probe(&SCAN),
            when: None,
        },
    ],
    import: None,
    duplicate: false,
    about: "A DNP3 outstation (an RTU or IED) the server polls over TCP. Add one with N, or scan the network.",
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
    fn the_choices_are_the_servers_group_variations() {
        let ours: Vec<String> = GROUP_VARIATIONS
            .iter()
            .map(|(value, _)| (*value).to_owned())
            .collect();
        let servers: Vec<String> = DNP3_GROUP_VARIATIONS
            .iter()
            .map(|(group, variation, _)| format!("{group}.{variation}"))
            .collect();
        assert_eq!(ours, servers);
    }

    #[test]
    fn a_point_splits_its_group_and_variation() {
        let mut values = values_of(POINTS.fields, None);
        values.insert("name", "kw".into());
        let mut sent = body(POINTS.fields, &values, false).unwrap();
        finish_point(&mut sent, false);
        assert_eq!(
            Value::Object(sent),
            json!({"name": "kw", "display_name": "kw", "group_num": 30, "variation": 4,
                "point_index": 0, "data_type": "float64", "scale": 1.0, "deadband": 0.0})
        );
    }

    #[test]
    fn an_outstation_sends_null_for_no_event_scan() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("name", "RTU".into());
        values.insert("ip", "10.0.0.3".into());
        let sent = body(PROTOCOL.fields, &values, false).unwrap();
        assert_eq!(sent["event_scan_interval_ms"], Value::Null);
        assert_eq!(sent["port"], 20000);
        assert_eq!(sent["outstation_address"], 10);
        values.insert("event_scan_interval_ms", "500".into());
        assert_eq!(
            body(PROTOCOL.fields, &values, false).unwrap()["event_scan_interval_ms"],
            500
        );
    }

    #[test]
    fn a_scan_needs_a_cidr_and_adds_what_answered() {
        let mut values = values_of(SCAN.fields, None);
        assert_eq!(
            scan_request(None, &values).unwrap(),
            (
                "/api/v1/dnp3/scan".to_owned(),
                Some(json!({"range": "192.168.1.0/24", "port": 20000}))
            )
        );
        values.insert("range", "10.0.0.1".into());
        assert!(scan_request(None, &values).is_err());
        let answer = scan_decode(
            json!([{"ip": "10.0.0.4", "port": 20000, "outstation_address": 10}]),
            &values,
        )
        .unwrap();
        assert_eq!(
            answer.rows[0].payload,
            json!({"name": "dnp3_10_0_0_4", "ip": "10.0.0.4", "port": 20000,
                "outstation_address": 10, "enabled": true})
        );
    }
}
