//! Data Manager › Siemens S7, after the web client's `S7DeviceManager.tsx`
//! (both send the server's shapes): PLCs at `host:port` with
//! rack and slot, their tags by S7 address (`DB1.DBD0`, `M0.0`), start and
//! stop, and a browse that says whether the PLC answers and lists the
//! address forms to pick from. Every S7 handler checks for the admin role.

use std::time::Duration;

use gpui::Context;
use serde_json::{Value, json};
use voltrus_scada_native::api::protocols::{S7Browse, S7DeviceRow, S7TagRow, rows};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;

use super::device_form::{
    self, Cell, Child, ChildApi, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Scope, Tool, ToolKind,
    Values, calls, choice, col, device_path, flag, int, mono, name_of, optional, row_id,
    section_hooks, text, value_text,
};
use crate::console::Console;
use crate::console::pages::Role;

pub(crate) const TYPES: &[(&str, &str)] = &[
    ("bool", "bool"),
    ("int16", "int16"),
    ("uint16", "uint16"),
    ("int32", "int32"),
    ("uint32", "uint32"),
    ("float32", "float32"),
    ("float64", "float64"),
    ("string", "string"),
];

fn link(row: &Value) -> String {
    format!("s7_{}", row_id(row))
}

pub(crate) fn browse_request(
    device: Option<&Value>,
    _: &Values,
) -> Result<(String, Option<Value>), String> {
    let device = device.ok_or("no PLC")?;
    Ok((format!("{}/browse", device_path(&PROTOCOL, device)), None))
}

/// The address forms, without timers and counters: the server lists them
/// but cannot add a tag at one.
pub(crate) fn browse_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let browse: S7Browse = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    let note = if browse.connected {
        format!(
            "The PLC at {} answered. ↵ adds a tag at an address.",
            browse.url
        )
    } else {
        format!(
            "The PLC at {} did not answer; these are the address forms it takes.",
            browse.url
        )
    };
    Ok(ProbeAnswer {
        note: Some(note),
        rows: browse
            .areas
            .into_iter()
            .filter(|area| !matches!(area.area.as_str(), "T" | "C"))
            .flat_map(|area| {
                let detail = format!("{} · {}", area.description, area.address_format);
                area.examples.into_iter().map(move |example| ProbeRow {
                    label: example.clone(),
                    detail: detail.clone(),
                    payload: json!({ "tag_address": example }),
                })
            })
            .collect(),
    })
}

static BROWSE: ProbeSpec = ProbeSpec {
    title: "Browse the PLC",
    intro: |device| {
        device.map_or_else(String::new, |device| {
            format!(
                "{} · {} · rack {} slot {}",
                name_of(device),
                value_text(device, "url"),
                value_text(device, "rack"),
                value_text(device, "slot")
            )
        })
    },
    fields: &[],
    run_label: "Browse again",
    request: browse_request,
    timeout: |_| Duration::from_secs(45),
    decode: browse_decode,
    multi: false,
    pick_label: "Add tag",
    on_pick: device_form::pick_child,
    empty: "No address forms came back.",
    auto: true,
};

static TAGS: Child = Child {
    title: "Tags",
    noun: "tag",
    api: ChildApi::Rest {
        segment: "tags",
        within: None,
    },
    decode: rows::<S7TagRow>,
    columns: &[
        col("Address", 0.0, Cell::Key("tag_address"), true, 0),
        col("Name", 160.0, Cell::Key("name"), true, 0),
        col("Display name", 160.0, Cell::Key("display_name"), false, 2),
        col("Type", 80.0, Cell::Key("data_type"), true, 1),
    ],
    fields: &[
        mono("tag_address", "Address", "DB1.DBD0"),
        mono("name", "Name", "flow"),
        optional("display_name", "Display name", "Flow"),
        choice("data_type", "Data type", TYPES, "float32"),
    ],
    finish: None,
    makes_tags: true,
};

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "s7",
    title: "Siemens S7",
    noun: "S7 PLC",
    icon: Icon::Cpu,
    list_id: "data.s7.list",
    calls: calls!("s7"),
    base: "/api/v1/s7/devices",
    decode: rows::<S7DeviceRow>,
    write_role: Role::Admin,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("Address", 150.0, Cell::Key("url"), true, 0),
        col("Rack", 44.0, Cell::Key("rack"), true, 3),
        col("Slot", 44.0, Cell::Key("slot"), true, 3),
        col("Interval", 72.0, Cell::Ms("interval_ms"), true, 4),
        col("Tags", 48.0, Cell::Key("tag_count"), true, 2),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 0),
    ],
    search: &["name", "id", "url"],
    fields: &[
        text("name", "Name", "S7-1200 CPU"),
        mono("url", "Address (host:port)", "192.168.0.1:102"),
        int("rack", "Rack", "0", 0, 255),
        int("slot", "Slot", "1", 0, 255),
        int("interval_ms", "Interval (ms)", "1000", 100, 3_600_000),
        flag("enabled", "Enabled", "true"),
    ],
    finish: None,
    child: Some(&TAGS),
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
            label: "Browse addresses",
            icon: Icon::Search,
            scope: Scope::Device,
            kind: ToolKind::Probe(&BROWSE),
            when: None,
        },
    ],
    import: None,
    duplicate: false,
    about: "A Siemens S7 PLC the server polls over ISO-on-TCP. Add one with N.",
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
    fn a_plc_and_a_tag_go_in_the_servers_shape() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("name", "CPU".into());
        values.insert("url", "10.0.0.9:102".into());
        assert_eq!(
            Value::Object(body(PROTOCOL.fields, &values, false).unwrap()),
            json!({"name": "CPU", "url": "10.0.0.9:102", "rack": 0, "slot": 1,
                "interval_ms": 1000, "enabled": true})
        );
        let mut tag = values_of(TAGS.fields, None);
        tag.insert("tag_address", "DB1.DBD0".into());
        tag.insert("name", "flow".into());
        assert_eq!(
            Value::Object(body(TAGS.fields, &tag, false).unwrap()),
            json!({"tag_address": "DB1.DBD0", "name": "flow", "display_name": "",
                "data_type": "float32"})
        );
    }

    #[test]
    fn browsing_offers_the_addresses_a_tag_can_take() {
        let answer = browse_decode(
            json!({"connected": false, "url": "10.0.0.9:102", "rack": 0, "slot": 1, "count": 2,
                "areas": [
                    {"area": "DB", "description": "Data Block", "address_format": "DB<n>.DBX<byte>.<bit>",
                     "examples": ["DB1.DBX0.0", "DB1.DBW100"]},
                    {"area": "T", "description": "Timers", "address_format": "T<n>", "examples": ["T0"]}
                ]}),
            &Values::new(),
        )
        .unwrap();
        assert!(answer.note.unwrap().contains("did not answer"));
        let labels: Vec<_> = answer.rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["DB1.DBX0.0", "DB1.DBW100"], "no timers");
        assert_eq!(
            device_form::prefill(TAGS.fields, &answer.rows[1].payload)["tag_address"],
            "DB1.DBW100"
        );
    }
}
