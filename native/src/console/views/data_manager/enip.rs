//! Data Manager › EtherNet/IP, after the web client's
//! `EnipDeviceManager.tsx` but in the server's shapes (the web client sends
//! `ip` and `port`, and tags without the `tag_name` the server needs):
//! PLCs at `host:port`, their tags by CIP symbol, start and stop, and a
//! browse of the PLC's symbols to pick from. Every EtherNet/IP handler
//! checks for the admin role.

use std::time::Duration;

use gpui::Context;
use serde_json::{Value, json};
use voltrus_scada_native::api::protocols::{EnipBrowse, EnipDeviceRow, EnipTagRow, rows};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;

use super::device_form::{
    self, Cell, Child, ChildApi, Field, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Scope, Tool,
    ToolKind, Values, calls, choice, col, device_path, flag, int, mono, name_of, optional, row_id,
    section_hooks, text, value_text,
};
use crate::console::Console;
use crate::console::pages::Role;

const TYPES: &[(&str, &str)] = &[
    ("bool", "bool"),
    ("sint8", "sint8"),
    ("int16", "int16"),
    ("dint32", "dint32"),
    ("lint64", "lint64"),
    ("real32", "real32"),
    ("string", "string"),
];

fn link(row: &Value) -> String {
    format!("enip_{}", row_id(row))
}

/// A browsed CIP type as the tag form's data type.
fn data_type(cip: &str) -> &'static str {
    match cip {
        "BOOL" => "bool",
        "SINT" => "sint8",
        "INT" => "int16",
        "LINT" => "lint64",
        "REAL" => "real32",
        "STRING" => "string",
        _ => "dint32",
    }
}

fn browse_request(device: Option<&Value>, _: &Values) -> Result<(String, Option<Value>), String> {
    let device = device.ok_or("no PLC")?;
    Ok((format!("{}/browse", device_path(&PROTOCOL, device)), None))
}

fn browse_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let browse: EnipBrowse = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!("{} symbols; ↵ adds one as a tag.", browse.count)),
        rows: browse
            .tags
            .into_iter()
            .map(|tag| ProbeRow {
                detail: format!("{} × {}", tag.data_type, tag.element_count),
                payload: json!({
                    "tag_name": tag.tag_name,
                    "name": tag.tag_name.replace(['.', '[', ']'], "_").to_lowercase(),
                    "data_type": data_type(&tag.data_type),
                }),
                label: tag.tag_name,
            })
            .collect(),
    })
}

static BROWSE: ProbeSpec = ProbeSpec {
    title: "Browse the PLC",
    intro: |device| {
        device.map_or_else(String::new, |device| {
            format!("{} · {}", name_of(device), value_text(device, "url"))
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
    empty: "The PLC lists no symbols.",
    auto: true,
};

static TAGS: Child = Child {
    title: "Tags",
    noun: "tag",
    api: ChildApi::Rest {
        segment: "tags",
        within: None,
    },
    decode: rows::<EnipTagRow>,
    columns: &[
        col("Tag", 0.0, Cell::Key("tag_name"), true, 0),
        col("Name", 160.0, Cell::Key("name"), true, 0),
        col("Display name", 160.0, Cell::Key("display_name"), false, 2),
        col("Type", 80.0, Cell::Key("data_type"), true, 1),
    ],
    fields: &[
        mono("tag_name", "Tag (CIP symbol)", "Program.Flow"),
        mono("name", "Name", "flow"),
        optional("display_name", "Display name", "Flow"),
        choice("data_type", "Data type", TYPES, "dint32"),
    ],
    finish: None,
    makes_tags: true,
};

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "enip",
    title: "EtherNet/IP",
    noun: "EtherNet/IP PLC",
    icon: Icon::Cable,
    list_id: "data.enip.list",
    calls: calls!("enip"),
    base: "/api/v1/enip/devices",
    decode: rows::<EnipDeviceRow>,
    write_role: Role::Admin,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("Address", 160.0, Cell::Key("url"), true, 0),
        col("Slot", 44.0, Cell::Key("slot"), true, 3),
        col("Interval", 72.0, Cell::Ms("interval_ms"), true, 4),
        col("Tags", 48.0, Cell::Key("tag_count"), true, 2),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 0),
    ],
    search: &["name", "id", "url"],
    fields: &[
        text("name", "Name", "ControlLogix L85E"),
        mono("url", "Address (host:port)", "192.168.1.10:44818"),
        int("slot", "Slot", "0", 0, 255),
        int("interval_ms", "Interval (ms)", "1000", 100, 3_600_000),
        // The list leaves it out, so an edit could only reset it.
        Field {
            create_only: true,
            ..int(
                "assembly_instance",
                "Assembly instance (0 polls tags)",
                "0",
                0,
                4_294_967_295,
            )
        },
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
            label: "Browse symbols",
            icon: Icon::Search,
            scope: Scope::Device,
            kind: ToolKind::Probe(&BROWSE),
            when: None,
        },
    ],
    import: None,
    duplicate: false,
    about: "An EtherNet/IP (CIP) PLC the server reads tags from. Add one with N.",
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
    fn the_assembly_instance_is_set_once_and_tags_carry_their_symbol() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("name", "L85".into());
        values.insert("url", "10.0.0.8:44818".into());
        assert_eq!(
            Value::Object(body(PROTOCOL.fields, &values, false).unwrap()),
            json!({"name": "L85", "url": "10.0.0.8:44818", "slot": 0, "interval_ms": 1000,
                "assembly_instance": 0, "enabled": true})
        );
        let editing = body(PROTOCOL.fields, &values, true).unwrap();
        assert!(!editing.contains_key("assembly_instance"));
    }

    #[test]
    fn browsed_symbols_fill_the_tag_form() {
        let answer = browse_decode(
            json!({"tags": [{"tag_name": "Program.Flow[2]", "data_type": "REAL", "element_count": 1}],
                "count": 1}),
            &Values::new(),
        )
        .unwrap();
        let values = device_form::prefill(TAGS.fields, &answer.rows[0].payload);
        assert_eq!(values["tag_name"], "Program.Flow[2]");
        assert_eq!(values["name"], "program_flow_2_");
        assert_eq!(values["data_type"], "real32");
        assert_eq!(data_type("DWORD"), "dint32");
    }
}
