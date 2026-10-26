//! Data Manager › OPC UA, after the web client's `OpcUaDeviceManager.tsx`:
//! servers with their monitored nodes, start and stop, and a browse of
//! the server's Objects folder to pick nodes from.
//!
//! The core router serves no `GET /api/v1/opcua/devices` (only POST), so on
//! the open-core server the section says so instead of listing; an
//! edition whose router adds the route gets the full section. Every OPC UA
//! handler checks for the admin role itself, so the section is admin's.

use std::time::Duration;

use gpui::Context;
use serde_json::{Map, Value, json};
use voltrus_scada_native::api::protocols::{OpcUaBrowse, OpcUaDeviceRow, OpcUaNodeRow, rows};
use voltrus_scada_native::runtime::{CallKey, Reply};
use vt_ui::Icon;

use super::device_form::{
    self, Cell, Child, ChildApi, Field, Kind, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Scope,
    Tool, ToolKind, Values, calls, col, device_path, int, mono, name_of, optional, row_id,
    section_hooks, text, value_text,
};
use crate::console::Console;
use crate::console::pages::Role;

const POLICIES: &[(&str, &str)] = &[("None", "None"), ("basic256sha256", "Basic256Sha256")];
const MODES: &[(&str, &str)] = &[
    ("None", "None"),
    ("Sign", "Sign"),
    ("signandencrypt", "Sign & encrypt"),
];

fn security(row: &Value) -> String {
    format!(
        "{}/{}",
        value_text(row, "security_policy"),
        value_text(row, "security_mode")
    )
}

fn link(row: &Value) -> String {
    format!("opcua_{}", row_id(row))
}

/// A blank user name is left out (the server cannot clear one).
fn finish(body: &mut Map<String, Value>, _: bool) {
    if body.get("username").and_then(Value::as_str) == Some("") {
        body.remove("username");
    }
}

fn browse_request(device: Option<&Value>, _: &Values) -> Result<(String, Option<Value>), String> {
    let device = device.ok_or("no server")?;
    Ok((format!("{}/browse", device_path(&PROTOCOL, device)), None))
}

fn browse_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let browse: OpcUaBrowse = serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!(
            "{} nodes under Objects; ↵ adds one to monitor.",
            browse.count
        )),
        rows: browse
            .nodes
            .into_iter()
            .map(|node| ProbeRow {
                label: node.node_id.clone(),
                detail: format!("{} · {}", node.display_name, node.node_class),
                payload: json!({
                    "node_id": node.node_id,
                    "name": node.browse_name,
                    "display_name": node.display_name,
                }),
            })
            .collect(),
    })
}

static BROWSE: ProbeSpec = ProbeSpec {
    title: "Browse the server",
    intro: |device| {
        device.map_or_else(String::new, |device| {
            format!("{} · {}", name_of(device), value_text(device, "url"))
        })
    },
    fields: &[],
    run_label: "Browse again",
    request: browse_request,
    timeout: |_| Duration::from_secs(60),
    decode: browse_decode,
    multi: false,
    pick_label: "Add node",
    on_pick: device_form::pick_child,
    empty: "The server has no nodes under Objects.",
    auto: true,
};

static NODES: Child = Child {
    title: "Nodes",
    noun: "node",
    api: ChildApi::Rest {
        segment: "nodes",
        within: None,
    },
    decode: rows::<OpcUaNodeRow>,
    columns: &[
        col("Node ID", 0.0, Cell::Key("node_id"), true, 0),
        col("Name", 160.0, Cell::Key("name"), false, 0),
        col("Display name", 160.0, Cell::Key("display_name"), false, 2),
        col("Data type", 96.0, Cell::Key("data_type"), true, 1),
    ],
    fields: &[
        mono("node_id", "Node ID", "ns=2;s=Temperature"),
        mono("name", "Name", "temperature"),
        optional("display_name", "Display name", "Temperature"),
        Field {
            kind: Kind::Text {
                required: false,
                mono: true,
            },
            ..optional("data_type", "Data type", "Double")
        },
    ],
    finish: None,
    makes_tags: true,
};

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "opcua",
    title: "OPC UA",
    noun: "OPC UA server",
    icon: Icon::Network,
    list_id: "data.opcua.list",
    calls: calls!("opcua"),
    base: "/api/v1/opcua/devices",
    decode: rows::<OpcUaDeviceRow>,
    write_role: Role::Admin,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("URL", 220.0, Cell::Key("url"), true, 0),
        col("Security", 150.0, Cell::Fn(security), true, 3),
        col("Interval", 72.0, Cell::Ms("interval_ms"), true, 4),
        col("Nodes", 56.0, Cell::Key("node_count"), true, 2),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 0),
    ],
    search: &["name", "id", "url"],
    fields: &[
        text("name", "Name", "My OPC UA server"),
        mono("url", "URL", "opc.tcp://192.168.1.100:4840"),
        device_form::choice("security_policy", "Security policy", POLICIES, "None"),
        device_form::choice("security_mode", "Security mode", MODES, "None"),
        optional("username", "User name", "operator"),
        Field {
            kind: Kind::Secret,
            ..optional("password", "Password", "")
        },
        int("interval_ms", "Interval (ms)", "1000", 100, 3_600_000),
        device_form::flag("enabled", "Enabled", "true"),
    ],
    finish: Some(finish),
    child: Some(&NODES),
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
            label: "Browse nodes",
            icon: Icon::Search,
            scope: Scope::Device,
            kind: ToolKind::Probe(&BROWSE),
            when: None,
        },
    ],
    import: None,
    duplicate: false,
    about: "An OPC UA server the console polls for the nodes you pick. Add one with N.",
    unrouted: Some(
        "Its router cannot list OPC UA devices (it serves no GET for them; the open core leaves that to an edition). An edition that routes the list gets this section in full.",
    ),
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
    fn a_server_is_created_with_the_servers_enum_names() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("name", "Plant".into());
        values.insert("url", "opc.tcp://10.0.0.5:4840".into());
        let mut sent = body(PROTOCOL.fields, &values, false).unwrap();
        finish(&mut sent, false);
        assert_eq!(
            Value::Object(sent),
            json!({"name": "Plant", "url": "opc.tcp://10.0.0.5:4840", "security_policy": "None",
                "security_mode": "None", "interval_ms": 1000, "enabled": true}),
            "\"None\", not the web client's \"none\"; no blank user or password"
        );
    }

    #[test]
    fn browsed_nodes_fill_the_node_form() {
        let answer = browse_decode(
            json!({"nodes": [{"node_id": "ns=2;s=Foo", "browse_name": "2:Foo",
                "display_name": "Foo", "node_class": "Variable"}], "count": 1}),
            &Values::new(),
        )
        .unwrap();
        assert_eq!(answer.rows[0].label, "ns=2;s=Foo");
        let values = device_form::prefill(NODES.fields, &answer.rows[0].payload);
        assert_eq!(values["node_id"], "ns=2;s=Foo");
        assert_eq!(values["name"], "2:Foo");
        assert_eq!(values["display_name"], "Foo");
    }
}
