//! Data Manager › Modbus, after the web client's `ModbusDeviceManager.tsx`,
//! `ModbusDeviceForm.tsx`, `SimControls.tsx` and `SimProfileManager.tsx`:
//! Modbus TCP devices with their registers; the pollers' states, fetched
//! every 5 s while the section is on screen; restart; a register scan whose
//! readable registers can be added; a network scan (the server's route the
//! web client never offered); duplicate; Modbus Poll device files from the
//! Import… prompt or dropped on the section; simulation profiles and the
//! simulation switch.
//!
//! Reads are open to every signed-in user, changes to operators (the
//! server's `demo_protected` routes). Registers are added and removed by
//! replacing the device's list (`PUT …/registers`), as the server stores
//! them; added registers carry an explicit `offset`.

use std::collections::HashMap;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{AnyElement, Context, Entity, FontWeight, Subscription, Window, div, px};
use serde::de::IgnoredAny;
use serde_json::{Map, Value, json};
use voltrus_scada_native::api::Method;
use voltrus_scada_native::api::ops::encode;
use voltrus_scada_native::api::protocols::{
    ModbusDeviceRow, ModbusNetworkScan, ModbusRegisterRow, PollerStatus, ScannedRegister,
    SimProfile, rows, scanned_offset,
};
use voltrus_scada_native::device_file::DeviceFileKind;
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, pill};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::device_form::{
    self, Cell, Child, ChildApi, Field, Kind, ProbeAnswer, ProbeRow, ProbeSpec, Protocol, Saved,
    Scope, Tool, ToolKind, Values, calls, choice, col, device_path, int, mono, name_of, optional,
    row_id, section, section_hooks, value_text,
};
use crate::console::Console;
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::{ListKeys, focus_ring, keyboard_list};

const ENDIANNESS: &[(&str, &str)] = &[("big", "big"), ("little", "little"), ("swapped", "swapped")];
const TYPES: &[(&str, &str)] = &[
    ("float32", "float32"),
    ("int32", "int32"),
    ("uint16", "uint16"),
    ("int16", "int16"),
    ("float64", "float64"),
];
const AREAS: &[(&str, &str)] = &[
    ("holding", "holding"),
    ("input", "input"),
    ("coil", "coil"),
    ("discrete", "discrete"),
];
const ADDRESSING: &[(&str, &str)] = &[
    ("offset", "0-based offset"),
    ("address", "5-digit reference (40001)"),
];

const STATUS: &str = "data.modbus.status";
const PROFILES: &str = "data.modbus.profiles";
const PROFILE_SAVE: &str = "data.modbus.profile_save";
const PROFILE_DELETE: &str = "data.modbus.profile_delete";
const SIMULATE: &str = "data.modbus.simulate";
const PROFILES_DIALOG: &str = "data-modbus-profiles";
const SIMULATE_DIALOG: &str = "data-modbus-simulate";
const PROFILE_LIST: &str = "data.modbus.profile_list";

fn link(row: &Value) -> String {
    row_id(row)
}

fn register_address(row: &Value) -> String {
    let address = value_text(row, "address");
    if row
        .get("zero_based")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        format!("+{address}")
    } else {
        address
    }
}

/// A nameless device is named after its id, as the web form does.
fn finish_device(body: &mut Map<String, Value>, _: bool) {
    if body.get("name").and_then(Value::as_str) == Some("")
        && let Some(id) = body.get("id").cloned()
    {
        body.insert("name".into(), id);
    }
}

/// `addressing` + `at` → `offset` or `address`, as `ModbusRegisterInput`
/// takes them.
fn finish_register(body: &mut Map<String, Value>, _: bool) {
    let addressing = body.remove("addressing");
    let at = body.remove("at").unwrap_or(json!(0));
    let key = match addressing.as_ref().and_then(Value::as_str) {
        Some("address") => "address",
        _ => "offset",
    };
    body.insert(key.into(), at);
}

// ── Register scan ────────────────────────────────────────────────────────

fn number(values: &Values, key: &str, default: u64) -> u64 {
    values
        .get(key)
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(default)
}

fn scan_request(
    device: Option<&Value>,
    values: &Values,
) -> Result<(String, Option<Value>), String> {
    let device = device.ok_or("no device")?;
    let (start, end) = (
        number(values, "start_addr", 0),
        number(values, "end_addr", 100),
    );
    if end < start {
        return Err("The end address comes before the start".into());
    }
    if end > 65535 {
        return Err("Addresses go up to 65535".into());
    }
    if end - start >= 1000 {
        return Err("Scan at most 1000 registers at a time".into());
    }
    let area = values
        .get("register_area")
        .map_or("holding", String::as_str);
    Ok((
        "/api/v1/modbus-devices/scan".into(),
        Some(json!({
            "ip": value_text(device, "ip"),
            "port": device.get("port").cloned().unwrap_or(json!(502)),
            "unit_id": device.get("unit_id").cloned().unwrap_or(json!(1)),
            "start_addr": start,
            "end_addr": end,
            "endianness": value_text(device, "endianness"),
            "register_area": area,
            "scan_delay_ms": number(values, "scan_delay_ms", 500),
        })),
    ))
}

/// Two reads per batch of 20, each up to 5 s, plus the delay.
fn scan_timeout(values: &Values) -> Duration {
    let span = number(values, "end_addr", 100).saturating_sub(number(values, "start_addr", 0)) + 1;
    let delay = number(values, "scan_delay_ms", 500) / 1000;
    Duration::from_secs((30 + span.div_ceil(20) * 10 + delay).min(900))
}

fn scan_decode(answer: Value, values: &Values) -> Result<ProbeAnswer, String> {
    let scanned: Vec<ScannedRegister> =
        serde_json::from_value(answer).map_err(|error| error.to_string())?;
    let area = values
        .get("register_area")
        .cloned()
        .unwrap_or_else(|| "holding".to_owned());
    let total = scanned.len();
    let rows: Vec<ProbeRow> = scanned
        .into_iter()
        .filter(|register| register.readable)
        .map(|register| {
            let bits = matches!(area.as_str(), "coil" | "discrete");
            let kind = if bits {
                "uint16".to_owned()
            } else {
                register.likely_type.clone()
            };
            ProbeRow {
                label: format!("reg_{:05}", register.address),
                detail: format!(
                    "{} · {} · {}",
                    register.likely_type,
                    register
                        .interpreted_value
                        .map_or_else(|| "—".to_owned(), |value| format!("{value:.2}")),
                    if register.changed {
                        "changing"
                    } else {
                        "steady"
                    }
                ),
                payload: json!({
                    "name": format!("reg_{:05}", register.address),
                    "offset": scanned_offset(register.address, &area),
                    "register_type": kind,
                    "register_area": area,
                    "scale": 1.0,
                }),
            }
        })
        .collect();
    Ok(ProbeAnswer {
        note: Some(format!(
            "{} of {total} addresses read; Space selects, ↵ adds the selected registers.",
            rows.len()
        )),
        rows,
    })
}

/// Add the picked registers to the device (by offset, so coils and
/// discrete inputs land where they were read); names it has are skipped.
fn add_scanned(
    console: &mut Console,
    protocol: &'static Protocol,
    device: Option<Value>,
    registers: Vec<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(device) = device else {
        return;
    };
    console.close_overlay(window, cx);
    let path = device_path(protocol, &device);
    let id = row_id(&device);
    let name = name_of(&device);
    console.call(
        CallKey::new(protocol.calls.save),
        Call::new(move |api| async move {
            let mut current = device_form::modbus_registers(&api, &path).await?;
            let mut added = 0;
            for register in registers {
                let fresh = !current
                    .iter()
                    .any(|have| value_text(have, "name") == value_text(&register, "name"));
                if fresh {
                    current.push(register);
                    added += 1;
                }
            }
            device_form::replace_registers(&api, &path, current).await?;
            anyhow::Ok(Saved::child(
                id,
                format!("Added {added} registers to {name}"),
            ))
        }),
    );
}

static SCAN: ProbeSpec = ProbeSpec {
    title: "Scan registers",
    intro: |device| {
        device.map_or_else(String::new, |device| {
            format!(
                "{}:{} · unit {} · {}",
                value_text(device, "ip"),
                value_text(device, "port"),
                value_text(device, "unit_id"),
                value_text(device, "endianness")
            )
        })
    },
    fields: &[
        int("start_addr", "Start offset", "0", 0, 65535),
        int("end_addr", "End offset", "100", 0, 65535),
        choice("register_area", "Area", AREAS, "holding"),
        int(
            "scan_delay_ms",
            "Delay between reads (ms)",
            "500",
            0,
            10_000,
        ),
    ],
    run_label: "Scan",
    request: scan_request,
    timeout: scan_timeout,
    decode: scan_decode,
    multi: true,
    pick_label: "Add",
    on_pick: add_scanned,
    empty: "No address in this range answered.",
    auto: false,
};

// ── Network scan ─────────────────────────────────────────────────────────

fn network_request(_: Option<&Value>, values: &Values) -> Result<(String, Option<Value>), String> {
    let range = values.get("range").map_or("", |range| range.trim());
    if !range.contains('/') {
        return Err("Give the range as a CIDR, e.g. 192.168.1.0/24 (at most a /24)".into());
    }
    Ok((
        "/api/v1/modbus-devices/scan-network".into(),
        Some(json!({ "range": range, "port": number(values, "port", 502) })),
    ))
}

fn network_decode(answer: Value, _: &Values) -> Result<ProbeAnswer, String> {
    let found: ModbusNetworkScan =
        serde_json::from_value(answer).map_err(|error| error.to_string())?;
    Ok(ProbeAnswer {
        note: Some(format!(
            "{} devices answered; Space selects, ↵ adds them (without registers).",
            found.count
        )),
        rows: found
            .devices
            .into_iter()
            .map(|device| {
                let id = format!("mb_{}_u{}", device.ip.replace('.', "_"), device.unit_id);
                ProbeRow {
                    label: format!("{}:{}", device.ip, device.port),
                    detail: format!(
                        "unit {} · {} registers answered",
                        device.unit_id,
                        device.responsive_registers.len()
                    ),
                    payload: json!({
                        "id": id,
                        "name": id,
                        "ip": device.ip,
                        "port": device.port,
                        "unit_id": device.unit_id,
                    }),
                }
            })
            .collect(),
    })
}

static NETWORK: ProbeSpec = ProbeSpec {
    title: "Scan the network",
    intro: |_| {
        "Asks every host of the range for holding registers 0–9 on unit 1, 2 and 3.".to_owned()
    },
    fields: &[
        Field {
            default: "192.168.1.0/24",
            ..mono("range", "Range (CIDR, at most /24)", "192.168.1.0/24")
        },
        int("port", "Port", "502", 1, 65535),
    ],
    run_label: "Scan",
    request: network_request,
    timeout: |_| Duration::from_secs(90),
    decode: network_decode,
    multi: true,
    pick_label: "Add",
    on_pick: device_form::pick_devices,
    empty: "No Modbus device answered in this range.",
    auto: false,
};

// ── The tables ───────────────────────────────────────────────────────────

static REGISTERS: Child = Child {
    title: "Registers",
    noun: "register",
    api: ChildApi::ModbusRegisters,
    decode: rows::<ModbusRegisterRow>,
    columns: &[
        col("Name", 0.0, Cell::Key("name"), true, 0),
        col("Address", 80.0, Cell::Fn(register_address), true, 0),
        col("Type", 72.0, Cell::Key("register_type"), true, 0),
        col("Area", 72.0, Cell::Key("register_area"), false, 1),
        col("Scale", 56.0, Cell::Key("scale"), true, 2),
    ],
    fields: &[
        mono("name", "Name", "voltage"),
        choice("addressing", "Addressing", ADDRESSING, "offset"),
        int("at", "Offset or reference", "0", 0, 65535),
        choice("register_type", "Type", TYPES, "float32"),
        choice("register_area", "Area", AREAS, "holding"),
        Field {
            kind: Kind::Float,
            default: "1",
            ..optional("scale", "Scale", "1")
        },
    ],
    finish: Some(finish_register),
    makes_tags: false,
};

fn running(row: &Value) -> bool {
    row.get("enabled").and_then(Value::as_bool).unwrap_or(true)
}

pub(crate) static PROTOCOL: Protocol = Protocol {
    id: "modbus",
    title: "Modbus",
    noun: "Modbus device",
    icon: Icon::Cable,
    list_id: "data.modbus.list",
    calls: calls!("modbus"),
    base: "/api/v1/modbus-devices",
    decode: rows::<ModbusDeviceRow>,
    write_role: Role::Operator,
    link,
    columns: &[
        col("Name", 0.0, Cell::Fn(name_of), false, 0),
        col("IP", 120.0, Cell::Key("ip"), true, 0),
        col("Port", 52.0, Cell::Key("port"), true, 5),
        col("Unit", 40.0, Cell::Key("unit_id"), true, 4),
        col("Interval", 72.0, Cell::Ms("interval_ms"), true, 6),
        col("Endian", 64.0, Cell::Key("endianness"), false, 7),
        col("Registers", 72.0, Cell::Key("register_count"), true, 3),
        col("Status", 92.0, Cell::Link, false, 1),
        col("Enabled", 64.0, Cell::Enabled, false, 2),
    ],
    search: &["name", "id", "ip"],
    fields: &[
        Field {
            create_only: true,
            ..mono("id", "Device ID", "plts_1")
        },
        optional("name", "Name", "PLTS 1"),
        mono("ip", "IP address", "192.168.1.10"),
        int("port", "Port", "502", 1, 65535),
        int("unit_id", "Unit ID", "1", 0, 255),
        int("interval_ms", "Interval (ms)", "1000", 50, 3_600_000),
        choice("endianness", "Word order", ENDIANNESS, "big"),
    ],
    finish: Some(finish_device),
    child: Some(&REGISTERS),
    tools: &[
        Tool {
            label: "Restart poller",
            icon: Icon::RefreshCw,
            scope: Scope::Device,
            kind: ToolKind::Post {
                segment: "restart",
                done: "poller restarted",
            },
            when: Some(running),
        },
        Tool {
            label: "Scan registers",
            icon: Icon::Search,
            scope: Scope::Device,
            kind: ToolKind::Probe(&SCAN),
            when: None,
        },
        Tool {
            label: "Simulation",
            icon: Icon::Activity,
            scope: Scope::Device,
            kind: ToolKind::Open(open_simulate),
            when: None,
        },
        Tool {
            label: "Scan network",
            icon: Icon::Radio,
            scope: Scope::Section,
            kind: ToolKind::Probe(&NETWORK),
            when: None,
        },
        Tool {
            label: "Sim profiles",
            icon: Icon::Activity,
            scope: Scope::Section,
            kind: ToolKind::Open(open_profiles),
            when: None,
        },
    ],
    import: Some(DeviceFileKind::Modbus),
    duplicate: true,
    about: "A Modbus TCP device (a PLC, meter or inverter) the server polls. Add one with N, scan the network, or drop a Modbus Poll device file here.",
    unrouted: None,
};

section_hooks!();

/// Fetch the list once, and the pollers' states every 5 s while shown.
pub fn on_open(console: &mut Console, cx: &mut Context<Console>) {
    device_form::on_open(&PROTOCOL, console);
    if console.stress.is_some() {
        return;
    }
    fetch_status(console, cx);
    device_form::poll_section(
        console,
        PROTOCOL.id,
        Duration::from_secs(5),
        fetch_status,
        cx,
    );
}

/// The newest status request's number: an older, slower answer is dropped.
#[derive(Default)]
struct StatusSeq(i64);

fn fetch_status(console: &mut Console, _: &mut Context<Console>) {
    let seq = console.state::<StatusSeq>();
    seq.0 += 1;
    let key = CallKey::new(STATUS).with(seq.0);
    console.call(
        key,
        Call::get::<Vec<PollerStatus>>("/api/v1/modbus-devices/status"),
    );
}

pub fn on_reply(console: &mut Console, key: CallKey, reply: Reply, cx: &mut Context<Console>) {
    match key.name {
        STATUS => {
            let newest = console.state::<StatusSeq>().0 == key.arg;
            if let (true, Ok(statuses)) = (newest, reply.take::<Vec<PollerStatus>>()) {
                section(console, &PROTOCOL).status = statuses
                    .into_iter()
                    .map(|status| {
                        let text = if status.status.is_empty() {
                            (if status.running { "online" } else { "offline" }).to_owned()
                        } else {
                            status.status
                        };
                        (status.device_id, text)
                    })
                    .collect();
            }
        }
        PROFILES => console.state::<Sim>().profiles = Some(reply.take::<Vec<SimProfile>>()),
        PROFILE_SAVE => match reply.take::<SimProfile>() {
            Ok(profile) => {
                console.show_toast(format!("Saved profile {}", profile.name), false, cx);
                let sim = console.state::<Sim>();
                sim.form = None;
                fetch_profiles(console);
            }
            Err(error) => match console.state::<Sim>().form.as_mut() {
                Some(form) => {
                    form.saving = false;
                    form.error = Some(server_text(&error));
                }
                None => console.show_toast(error, true, cx),
            },
        },
        PROFILE_DELETE => {
            match reply.take::<String>() {
                Ok(name) => console.show_toast(format!("Deleted profile {name}"), false, cx),
                Err(error) => {
                    console.show_toast(format!("Delete failed: {}", server_text(&error)), true, cx)
                }
            }
            fetch_profiles(console);
        }
        SIMULATE => {
            let answer = reply
                .take::<String>()
                .map_err(|error| simulate_error(&error));
            if let Some(simulate) = console.state::<Sim>().simulate.as_mut() {
                simulate.busy = false;
                simulate.result = Some(answer);
            }
        }
        _ => device_form::on_reply(&PROTOCOL, console, key, reply, cx),
    }
    cx.notify();
}

/// The simulation routes answer bare status codes, without a message.
fn server_text(error: &str) -> String {
    match error.trim_end_matches(": ") {
        text if text.starts_with("HTTP 500") => {
            "The server refused it (a profile name must be unique)".to_owned()
        }
        text => text.to_owned(),
    }
}

/// The server simulates rows of its `devices` table: a Modbus device has
/// none, so it answers 404.
fn simulate_error(error: &str) -> String {
    if error.starts_with("HTTP 404") {
        "The server has no simulated device with this id: it switches simulation for /api/v1/devices rows, not Modbus devices (404)".to_owned()
    } else {
        server_text(error)
    }
}

// ── Simulation profiles ──────────────────────────────────────────────────

const SIM_TYPES: &[(&str, &str)] = &[
    ("sinusoidal", "Sinusoidal"),
    ("random_walk", "Random walk"),
    ("step", "Step"),
    ("constant", "Constant"),
    ("square_wave", "Square wave"),
];

const fn number_field(key: &'static str, label: &'static str, placeholder: &'static str) -> Field {
    Field {
        key,
        label,
        placeholder,
        kind: Kind::Float,
        default: "0",
        create_only: false,
    }
}

const SINUSOIDAL: &[Field] = &[
    number_field("amplitude", "Amplitude", "100"),
    number_field("center", "Center", "0"),
    number_field("period_sec", "Period (s)", "60"),
    number_field("noise_pct", "Noise (%)", "5"),
];
const RANDOM_WALK: &[Field] = &[
    number_field("start_value", "Start value", "50"),
    number_field("step_size", "Step size", "0.5"),
    number_field("min", "Min", "0"),
    number_field("max", "Max", "100"),
];
const STEP: &[Field] = &[
    number_field("low_value", "Low value", "0"),
    number_field("high_value", "High value", "100"),
    number_field("step_period_sec", "Step period (s)", "30"),
];
const CONSTANT: &[Field] = &[number_field("value", "Value", "50")];
const SQUARE_WAVE: &[Field] = &[
    number_field("high_value", "High value", "100"),
    number_field("low_value", "Low value", "0"),
    number_field("period_sec", "Period (s)", "60"),
    number_field("duty_pct", "Duty (%)", "50"),
];
/// Every config field once, for the text fields.
const ALL_CONFIG: &[Field] = &[
    number_field("amplitude", "Amplitude", "100"),
    number_field("center", "Center", "0"),
    number_field("period_sec", "Period (s)", "60"),
    number_field("noise_pct", "Noise (%)", "5"),
    number_field("start_value", "Start value", "50"),
    number_field("step_size", "Step size", "0.5"),
    number_field("min", "Min", "0"),
    number_field("max", "Max", "100"),
    number_field("low_value", "Low value", "0"),
    number_field("high_value", "High value", "100"),
    number_field("step_period_sec", "Step period (s)", "30"),
    number_field("value", "Value", "50"),
    number_field("duty_pct", "Duty (%)", "50"),
];
const PROFILE_FIELDS: &[Field] = &[
    device_form::text("name", "Name", "Profile name"),
    choice("data_type", "Waveform", SIM_TYPES, "sinusoidal"),
];

fn config_fields(data_type: &str) -> &'static [Field] {
    match data_type {
        "random_walk" => RANDOM_WALK,
        "step" => STEP,
        "constant" => CONSTANT,
        "square_wave" => SQUARE_WAVE,
        _ => SINUSOIDAL,
    }
}

/// The body of a profile: `config` is JSON in a string, of the waveform's
/// numbers only.
fn profile_body(values: &Values) -> Result<Value, String> {
    let head = device_form::body(PROFILE_FIELDS, values, false)?;
    let data_type = head["data_type"]
        .as_str()
        .unwrap_or("sinusoidal")
        .to_owned();
    let config = device_form::body(config_fields(&data_type), values, false)?;
    for (key, limit) in [("noise_pct", 100.0), ("duty_pct", 100.0)] {
        if config
            .get(key)
            .and_then(Value::as_f64)
            .is_some_and(|value| !(0.0..=limit).contains(&value))
        {
            return Err(format!("{key} must be from 0 to {limit}"));
        }
    }
    Ok(json!({
        "name": head["name"],
        "data_type": data_type,
        "config": Value::Object(config).to_string(),
    }))
}

/// A profile's values for the form, its config spread over the fields.
fn profile_values(profile: Option<&SimProfile>) -> Values {
    let mut values = device_form::values_of(PROFILE_FIELDS, None);
    values.extend(device_form::values_of(ALL_CONFIG, None));
    if let Some(profile) = profile {
        values.insert("name", profile.name.clone());
        if SIM_TYPES
            .iter()
            .any(|(value, _)| *value == profile.data_type)
        {
            values.insert("data_type", profile.data_type.clone());
        }
        if let Ok(Value::Object(config)) = serde_json::from_str::<Value>(&profile.config) {
            for field in ALL_CONFIG {
                if let Some(value) = config.get(field.key) {
                    values.insert(field.key, value_text(&json!({ "v": value }), "v"));
                }
            }
        }
    }
    values
}

fn config_summary(profile: &SimProfile) -> String {
    match serde_json::from_str::<Value>(&profile.config) {
        Ok(Value::Object(config)) => config
            .iter()
            .map(|(key, value)| format!("{key} {value}"))
            .collect::<Vec<_>>()
            .join(" · "),
        _ => profile.config.clone(),
    }
}

struct ProfileForm {
    id: Option<i64>,
    inputs: HashMap<&'static str, Entity<TextInput>>,
    values: Values,
    error: Option<String>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

struct Simulate {
    device: Value,
    profile: Option<i64>,
    overrides: Entity<TextInput>,
    busy: bool,
    result: Option<Result<String, String>>,
    _subscription: Subscription,
}

#[derive(Default)]
struct Sim {
    profiles: Option<Result<Vec<SimProfile>, String>>,
    form: Option<ProfileForm>,
    /// The profile a Delete press is waiting to have confirmed.
    doomed: Option<i64>,
    simulate: Option<Simulate>,
}

fn fetch_profiles(console: &mut Console) {
    console.call(
        CallKey::new(PROFILES),
        Call::get::<Vec<SimProfile>>("/api/v1/sim-profiles"),
    );
}

fn open_profiles(
    console: &mut Console,
    protocol: &'static Protocol,
    _: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !console.allows(protocol.write_role) {
        return;
    }
    fetch_profiles(console);
    let sim = console.state::<Sim>();
    sim.form = None;
    sim.doomed = None;
    let overlay = Overlay::modal(PROFILES_DIALOG, px(640.0), render_profiles, cx);
    console.open_overlay(overlay, window, cx);
    console.focus_list(PROFILE_LIST, window, cx);
}

fn edit_profile(
    console: &mut Console,
    profile: Option<SimProfile>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let values = profile_values(profile.as_ref());
    // The waveform choice has no text field; make_inputs skips it.
    let (mut inputs, mut subscriptions) =
        device_form::make_inputs(PROFILE_FIELDS, &values, save_profile, window, cx);
    let (more, more_subscriptions) =
        device_form::make_inputs(ALL_CONFIG, &values, save_profile, window, cx);
    inputs.extend(more);
    subscriptions.extend(more_subscriptions);
    let name = inputs.get("name").cloned();
    console.state::<Sim>().form = Some(ProfileForm {
        id: profile.map(|profile| profile.id),
        inputs,
        values,
        error: None,
        saving: false,
        _subscriptions: subscriptions,
    });
    if let Some(name) = name {
        name.read(cx).focus(window);
    }
    cx.notify();
}

fn set_profile_value(
    console: &mut Console,
    key: &'static str,
    value: String,
    cx: &mut Context<Console>,
) {
    if let Some(form) = console.state::<Sim>().form.as_mut() {
        form.values.insert(key, value);
    }
    cx.notify();
}

fn save_profile(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<Sim>().form.as_mut() else {
        return;
    };
    if form.saving {
        return;
    }
    device_form::collect(&form.inputs, &mut form.values, cx);
    let body = match profile_body(&form.values) {
        Ok(body) => body,
        Err(error) => {
            form.error = Some(error);
            cx.notify();
            return;
        }
    };
    form.saving = true;
    form.error = None;
    let call = match form.id {
        Some(id) => {
            Call::send::<SimProfile>(Method::PUT, format!("/api/v1/sim-profiles/{id}"), body)
        }
        None => Call::send::<SimProfile>(Method::POST, "/api/v1/sim-profiles", body),
    };
    console.call(CallKey::new(PROFILE_SAVE), call);
    cx.notify();
}

fn delete_profile(console: &mut Console, profile: &SimProfile, cx: &mut Context<Console>) {
    let sim = console.state::<Sim>();
    if sim.doomed != Some(profile.id) {
        sim.doomed = Some(profile.id);
        cx.notify();
        return;
    }
    sim.doomed = None;
    let (id, name) = (profile.id, profile.name.clone());
    console.call(
        CallKey::new(PROFILE_DELETE),
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::DELETE, &format!("/api/v1/sim-profiles/{id}"))
                .await?;
            anyhow::Ok(name)
        }),
    );
}

fn profiles_list(console: &Console) -> Vec<SimProfile> {
    match console
        .state_ref::<Sim>()
        .and_then(|sim| sim.profiles.as_ref())
    {
        Some(Ok(profiles)) => profiles.clone(),
        _ => Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
fn render_profiles(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let profiles = profiles_list(console);
    let sim = console.state::<Sim>();
    let failed = match &sim.profiles {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let loading = sim.profiles.is_none();
    let doomed = sim.doomed;
    let form = sim.form.as_ref().map(|form| {
        (
            form.id,
            form.inputs.clone(),
            form.values.clone(),
            form.error.clone(),
            form.saving,
        )
    });
    let list = keyboard_list(
        ListKeys {
            id: PROFILE_LIST,
            on_move: None,
            on_activate: |console, row, _, window, cx| {
                let profile = profiles_list(console).get(row).cloned();
                if profile.is_some() {
                    edit_profile(console, profile, window, cx);
                }
            },
        },
        profiles.len(),
        profiles.len().max(1),
        console,
        cx,
    );
    let nav = console.list(PROFILE_LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    let rows: Vec<AnyElement> = profiles
        .iter()
        .enumerate()
        .map(|(at, profile)| {
            let hover = theme.hover;
            let edit = profile.clone();
            let remove = profile.clone();
            let confirming = doomed == Some(profile.id);
            div()
                .id(("data-modbus-profile", at))
                .relative()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .h(px(34.0))
                .px(SPACE_5)
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .when(cursor == Some(at), |this| this.bg(theme.selected))
                .hover(move |style| style.bg(hover))
                .when(focused && cursor == Some(at), |this| {
                    this.child(focus_ring(&theme))
                })
                .on_click(cx.listener(move |console, _, window, cx| {
                    console.list(PROFILE_LIST, cx).select(at);
                    edit_profile(console, Some(edit.clone()), window, cx);
                }))
                .child(
                    div()
                        .w(px(160.0))
                        .flex_none()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(profile.name.clone()),
                )
                .child(pill(
                    SIM_TYPES
                        .iter()
                        .find(|(value, _)| *value == profile.data_type)
                        .map_or(profile.data_type.clone(), |(_, label)| (*label).to_owned()),
                    theme.warning,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(config_summary(profile)),
                )
                .child({
                    let button = Button::new(
                        ("data-modbus-profile-delete", at),
                        if confirming {
                            "Confirm delete"
                        } else {
                            "Delete"
                        },
                    )
                    .compact();
                    let button = if confirming {
                        button.danger()
                    } else {
                        button.ghost()
                    };
                    button.on_click(cx.listener(move |console, _, _, cx| {
                        cx.stop_propagation();
                        delete_profile(console, &remove, cx);
                    }))
                })
                .into_any_element()
        })
        .collect();
    let list_body: AnyElement = if let Some(error) = failed {
        device_form::error_line(&error, &theme)
    } else if loading {
        div()
            .text_color(theme.fg_muted)
            .child("Loading…")
            .into_any_element()
    } else if profiles.is_empty() {
        div()
            .text_color(theme.fg_muted)
            .child("No simulation profiles yet. New profile makes one.")
            .into_any_element()
    } else {
        div()
            .flex()
            .flex_col()
            .gap(SPACE_1)
            .children(rows)
            .into_any_element()
    };

    let editor = form.map(|(id, inputs, values, error, saving)| {
        let data_type = values.get("data_type").cloned().unwrap_or_default();
        let mut fields = device_form::field_grid(
            PROFILE_FIELDS,
            false,
            &inputs,
            &values,
            set_profile_value,
            &theme,
            cx,
        );
        fields.extend(device_form::field_grid(
            config_fields(&data_type),
            false,
            &inputs,
            &values,
            set_profile_value,
            &theme,
            cx,
        ));
        div()
            .flex()
            .flex_col()
            .gap(SPACE_5)
            .p(SPACE_6)
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(
                div()
                    .text_size(FONT_SMALL)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.fg_muted)
                    .child(if id.is_some() {
                        "EDIT PROFILE"
                    } else {
                        "NEW PROFILE"
                    }),
            )
            .children(fields)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .child(div().flex_1().min_w_0().child(match &error {
                        Some(error) => device_form::error_line(error, &theme),
                        None => device_form::hints(&[("↵", "save"), ("Esc", "close")], &theme, cx),
                    }))
                    .child(
                        Button::new("data-modbus-profile-cancel", "Cancel")
                            .ghost()
                            .on_click(cx.listener(|console, _, _, cx| {
                                console.state::<Sim>().form = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(
                            "data-modbus-profile-save",
                            if saving { "Saving…" } else { "Save" },
                        )
                        .primary()
                        .icon(Icon::Check)
                        .disabled(saving)
                        .on_click(
                            cx.listener(|console, _, window, cx| save_profile(console, window, cx)),
                        ),
                    ),
            )
    });

    div()
        .flex()
        .flex_col()
        .max_h(px(640.0))
        .child(device_form::dialog_header(
            Icon::Activity,
            "Simulation profiles".to_owned(),
            "Waveforms a simulated device feeds instead of live values".to_owned(),
            &theme,
        ))
        .child(
            div()
                .id("data-modbus-profiles-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .overflow_y_scroll()
                .child(list.child(list_body))
                .children(editor),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(56.0))
                .px(SPACE_8)
                .border_t_1()
                .border_color(theme.border)
                .child(div().flex_1().child(device_form::hints(
                    &[("↵", "edit"), ("Esc", "close")],
                    &theme,
                    cx,
                )))
                .child(
                    Button::new("data-modbus-profile-new", "New profile")
                        .primary()
                        .icon(Icon::Plus)
                        .on_click(cx.listener(|console, _, window, cx| {
                            edit_profile(console, None, window, cx)
                        })),
                ),
        )
        .into_any_element()
}

// ── The simulation switch ────────────────────────────────────────────────

fn open_simulate(
    console: &mut Console,
    protocol: &'static Protocol,
    device: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(device) = device else {
        return;
    };
    if !console.allows(protocol.write_role) {
        return;
    }
    fetch_profiles(console);
    let overrides = cx.new(|cx| {
        TextInput::new(cx)
            .placeholder("{\"voltage\": 230}")
            .kind(InputKind::Text)
            .mono()
    });
    let subscription = cx.subscribe_in(
        &overrides,
        window,
        |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => switch_simulation(console, true, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
    );
    let sim = console.state::<Sim>();
    sim.simulate = Some(Simulate {
        device,
        profile: None,
        overrides,
        busy: false,
        result: None,
        _subscription: subscription,
    });
    let overlay = Overlay::modal(SIMULATE_DIALOG, px(560.0), render_simulate, cx);
    console.open_overlay(overlay, window, cx);
}

/// `{"tag": number}` from the overrides field, for a constant profile.
pub(crate) fn overrides(text: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) if map.values().all(Value::is_number) => {
            Ok(Some(Value::Object(map).to_string()))
        }
        _ => Err("Overrides are a JSON object of numbers: {\"voltage\": 230}".to_owned()),
    }
}

/// The simulate request's body.
pub(crate) fn simulate_body(profile: Option<i64>, overrides: Option<String>) -> Value {
    let mut body = json!({ "simulation_mode": true });
    if let Some(profile) = profile {
        body["sim_profile_id"] = json!(profile);
    }
    if let Some(config) = overrides {
        body["sim_config"] = json!(config);
    }
    body
}

fn switch_simulation(console: &mut Console, on: bool, cx: &mut Context<Console>) {
    let constant = {
        let profiles = profiles_list(console);
        let chosen = console
            .state_ref::<Sim>()
            .and_then(|sim| sim.simulate.as_ref())
            .and_then(|simulate| simulate.profile);
        chosen.is_some_and(|id| {
            profiles
                .iter()
                .any(|profile| profile.id == id && profile.data_type == "constant")
        })
    };
    let Some(simulate) = console.state::<Sim>().simulate.as_mut() else {
        return;
    };
    if simulate.busy {
        return;
    }
    let id = row_id(&simulate.device);
    let name = name_of(&simulate.device);
    let call = if on {
        let config = if constant {
            match overrides(simulate.overrides.read(cx).value()) {
                Ok(config) => config,
                Err(error) => {
                    simulate.result = Some(Err(error));
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let body = simulate_body(simulate.profile, config);
        let path = format!("/api/v1/devices/{}/simulate", encode(&id));
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(Method::POST, &path, &body)
                .await?;
            anyhow::Ok(format!("{name} is simulated"))
        })
    } else {
        let path = format!("/api/v1/devices/{}/go-live", encode(&id));
        Call::new(move |api| async move {
            api.send_empty::<IgnoredAny>(Method::POST, &path).await?;
            anyhow::Ok(format!("{name} is live"))
        })
    };
    simulate.busy = true;
    simulate.result = None;
    console.call(CallKey::new(SIMULATE), call);
    cx.notify();
}

#[allow(clippy::too_many_lines)]
fn render_simulate(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let profiles = profiles_list(console);
    let Some(simulate) = console.state::<Sim>().simulate.as_ref() else {
        return div().into_any_element();
    };
    let (device, chosen, busy, result) = (
        simulate.device.clone(),
        simulate.profile,
        simulate.busy,
        simulate.result.clone(),
    );
    let overrides_field = simulate.overrides.clone();
    let constant = chosen.is_some_and(|id| {
        profiles
            .iter()
            .any(|profile| profile.id == id && profile.data_type == "constant")
    });
    let choices = div()
        .flex()
        .flex_wrap()
        .gap(SPACE_2)
        .child(profile_chip(
            None,
            chosen.is_none(),
            "No profile",
            &theme,
            cx,
        ))
        .children(profiles.iter().map(|profile| {
            profile_chip(
                Some(profile.id),
                chosen == Some(profile.id),
                &format!("{} ({})", profile.name, profile.data_type),
                &theme,
                cx,
            )
        }));
    div()
        .flex()
        .flex_col()
        .child(device_form::dialog_header(
            Icon::Activity,
            format!("Simulate {}", name_of(&device)),
            format!("{} · {}", row_id(&device), value_text(&device, "ip")),
            &theme,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .child(
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child("A simulated device gets values from a profile instead of the PLC. The server has no way to read the current mode back, so this only switches it; the poller takes it on its next restart."),
                )
                .child(vt_ui::components::field("Profile", choices, cx))
                .when(constant, |this| {
                    this.child(vt_ui::components::field("Values per tag (constant profile, optional)", overrides_field, cx))
                })
                .children(result.map(|result| match result {
                    Ok(message) => div()
                        .flex()
                        .items_center()
                        .gap(SPACE_3)
                        .text_size(FONT_SMALL)
                        .text_color(theme.success)
                        .child(vt_ui::icon(Icon::CircleCheck, ICON_SM, theme.success))
                        .child(message)
                        .into_any_element(),
                    Err(error) => device_form::error_line(&error, &theme),
                })),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_4)
                .h(px(56.0))
                .px(SPACE_8)
                .border_t_1()
                .border_color(theme.border)
                .child(div().flex_1().child(device_form::hints(&[("Esc", "close")], &theme, cx)))
                .child(
                    Button::new("data-modbus-go-live", "Go live")
                        .icon(Icon::Zap)
                        .disabled(busy)
                        .on_click(cx.listener(|console, _, _, cx| switch_simulation(console, false, cx))),
                )
                .child(
                    Button::new("data-modbus-simulate", if busy { "Switching…" } else { "Simulate" })
                        .primary()
                        .icon(Icon::Activity)
                        .disabled(busy)
                        .on_click(cx.listener(|console, _, _, cx| switch_simulation(console, true, cx))),
                ),
        )
        .into_any_element()
}

fn profile_chip(
    id: Option<i64>,
    chosen: bool,
    label: &str,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let hover = theme.hover;
    div()
        .id((
            "data-modbus-sim-profile",
            usize::try_from(id.unwrap_or(-1) + 1).unwrap_or(0),
        ))
        .h(px(24.0))
        .px(SPACE_4)
        .flex()
        .items_center()
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(if chosen { theme.accent } else { theme.border })
        .when(chosen, |this| {
            this.bg(theme.accent_soft).text_color(theme.fg)
        })
        .when(!chosen, |this| {
            this.text_color(theme.fg_muted)
                .hover(move |style| style.bg(hover))
        })
        .text_size(FONT_SMALL)
        .cursor_pointer()
        .tab_index(0)
        .focus(|style| style.border_color(theme.accent))
        .child(label.to_owned())
        .on_click(cx.listener(move |console, _, _, cx| {
            if let Some(simulate) = console.state::<Sim>().simulate.as_mut() {
                simulate.profile = id;
            }
            cx.notify();
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::views::data_manager::device_form::{body, values_of};

    #[test]
    fn a_device_is_named_after_its_id_and_edits_leave_the_id() {
        let mut values = values_of(PROTOCOL.fields, None);
        values.insert("id", "plts_9".into());
        values.insert("ip", "10.0.0.9".into());
        let mut sent = body(PROTOCOL.fields, &values, false).unwrap();
        finish_device(&mut sent, false);
        assert_eq!(
            Value::Object(sent),
            json!({"id": "plts_9", "name": "plts_9", "ip": "10.0.0.9", "port": 502,
                "unit_id": 1, "interval_ms": 1000, "endianness": "big"})
        );
        let editing = body(PROTOCOL.fields, &values, true).unwrap();
        assert!(
            !editing.contains_key("id"),
            "the server cannot change an id"
        );
    }

    #[test]
    fn a_register_goes_by_offset_or_by_reference() {
        let mut values = values_of(REGISTERS.fields, None);
        values.insert("name", "voltage".into());
        values.insert("at", "12".into());
        let mut sent = body(REGISTERS.fields, &values, false).unwrap();
        finish_register(&mut sent, false);
        assert_eq!(
            Value::Object(sent),
            json!({"name": "voltage", "offset": 12, "register_type": "float32",
                "register_area": "holding", "scale": 1.0})
        );
        values.insert("addressing", "address".into());
        values.insert("at", "40001".into());
        let mut sent = body(REGISTERS.fields, &values, false).unwrap();
        finish_register(&mut sent, false);
        assert_eq!(sent["address"], 40001);
        assert!(!sent.contains_key("offset") && !sent.contains_key("addressing"));
        assert_eq!(
            register_address(&json!({"address": 12, "zero_based": true})),
            "+12"
        );
        assert_eq!(
            register_address(&json!({"address": 40001, "zero_based": false})),
            "40001"
        );
    }

    #[test]
    fn a_register_scan_asks_with_the_devices_link_and_adds_by_offset() {
        let device = json!({"id": "plts_1", "ip": "10.0.0.5", "port": 1502, "unit_id": 2,
            "endianness": "swapped"});
        let mut values = values_of(SCAN.fields, None);
        values.insert("register_area", "coil".into());
        let (path, body) = scan_request(Some(&device), &values).unwrap();
        assert_eq!(path, "/api/v1/modbus-devices/scan");
        assert_eq!(
            body.unwrap(),
            json!({"ip": "10.0.0.5", "port": 1502, "unit_id": 2, "start_addr": 0, "end_addr": 100,
                "endianness": "swapped", "register_area": "coil", "scan_delay_ms": 500})
        );
        values.insert("end_addr", "0".into());
        values.insert("start_addr", "5".into());
        assert!(
            scan_request(Some(&device), &values).is_err(),
            "the server would overflow"
        );
        values.insert("start_addr", "0".into());
        values.insert("end_addr", "1000".into());
        assert!(scan_request(Some(&device), &values).is_err());

        let answer = scan_decode(
            json!([
                {"address": 1, "raw_value": 1, "raw_value2": 1, "changed": false, "readable": true,
                 "likely_type": "uint16", "interpreted_value": 1.0},
                {"address": 2, "raw_value": null, "raw_value2": null, "changed": false, "readable": false,
                 "likely_type": "uint16", "interpreted_value": null}
            ]),
            &values,
        )
        .unwrap();
        assert_eq!(answer.rows.len(), 1, "only readable registers");
        assert_eq!(
            answer.rows[0].payload,
            json!({"name": "reg_00001", "offset": 0, "register_type": "uint16",
                "register_area": "coil", "scale": 1.0}),
            "coil 00001 is offset 0"
        );
    }

    #[test]
    fn a_network_scan_adds_devices_named_by_address() {
        let values = values_of(NETWORK.fields, None);
        assert_eq!(
            network_request(None, &values).unwrap().1.unwrap(),
            json!({"range": "192.168.1.0/24", "port": 502})
        );
        let answer = network_decode(
            json!({"devices": [{"ip": "10.0.0.7", "port": 502, "unit_id": 1,
                "responsive_registers": [{"address": 0, "value": 1.0, "raw": [1]}]}], "count": 1}),
            &values,
        )
        .unwrap();
        assert_eq!(
            answer.rows[0].payload,
            json!({"id": "mb_10_0_0_7_u1", "name": "mb_10_0_0_7_u1", "ip": "10.0.0.7",
                "port": 502, "unit_id": 1})
        );
    }

    #[test]
    fn a_profile_stores_its_waveform_as_a_json_string() {
        let mut values = profile_values(None);
        assert_eq!(profile_body(&values).unwrap_err(), "Name is required");
        values.insert("name", "Sine".into());
        values.insert("amplitude", "10".into());
        values.insert("max", "999".into());
        let body = profile_body(&values).unwrap();
        assert_eq!(body["name"], "Sine");
        assert_eq!(body["data_type"], "sinusoidal");
        let config: Value = serde_json::from_str(body["config"].as_str().unwrap()).unwrap();
        assert_eq!(
            config,
            json!({"amplitude": 10.0, "center": 0.0, "period_sec": 0.0, "noise_pct": 0.0}),
            "only the waveform's own numbers"
        );
        values.insert("noise_pct", "150".into());
        assert!(profile_body(&values).is_err());

        let stored = SimProfile {
            id: 3,
            name: "Flat".into(),
            data_type: "constant".into(),
            config: "{\"value\":42.5}".into(),
            created_at: 0,
        };
        let values = profile_values(Some(&stored));
        assert_eq!(values["data_type"], "constant");
        assert_eq!(values["value"], "42.5");
        assert_eq!(config_summary(&stored), "value 42.5");
    }

    #[test]
    fn simulation_sends_the_profile_and_constant_overrides() {
        assert_eq!(simulate_body(None, None), json!({"simulation_mode": true}));
        assert_eq!(
            simulate_body(Some(3), overrides("{\"voltage\": 230}").unwrap()),
            json!({"simulation_mode": true, "sim_profile_id": 3, "sim_config": "{\"voltage\":230}"})
        );
        assert_eq!(overrides("  ").unwrap(), None);
        assert!(overrides("{\"voltage\": \"high\"}").is_err());
        assert!(simulate_error("HTTP 404 Not Found: ").contains("not Modbus devices"));
    }
}
