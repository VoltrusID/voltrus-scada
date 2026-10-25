//! What the Protocols sections (Modbus, OPC UA, S7, EtherNet/IP, DNP3,
//! BACnet) share, after the web client's `*DeviceManager.tsx`: a searchable,
//! sortable device table whose rows expand into their registers, nodes,
//! tags, points or objects; a form for a device or a child; delete with a
//! confirmation; the device's tools (start, stop, browse, scans …) with
//! result lists to pick from; and device file import, from a file prompt
//! or by dropping files on the section.
//!
//! A protocol is data: one [`Protocol`] (paths, columns, form fields,
//! child table, tools) and four thin hooks in its module that call
//! [`render`], [`on_open`], [`focus`], [`on_reply`] and [`search`] here.
//! The pure parts (form bodies, search, sort, rows) have unit tests below.
//!
//! Keys in the table (`keys::DATA_DEVICES`): N new, A add a child, ↵ edit,
//! Space expand, E enable or disable, D duplicate (Modbus), ⌫ delete,
//! `.` the row's actions, I import, R refresh, S and ⇧S sort.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, AnyWindowHandle, App, Context, ElementId, Entity, ExternalPaths, FocusHandle,
    FontWeight, KeyDownEvent, PathPromptOptions, SharedString, Subscription, Window, div, px,
    uniform_list,
};
use serde::de::IgnoredAny;
use serde_json::{Map, Value, json};
use voltrus_scada_native::api::ops::encode;
use voltrus_scada_native::api::protocols::{CreatedId, ModbusRegisterRow, not_routed};
use voltrus_scada_native::api::{Method, VoltrusApi};
use voltrus_scada_native::device_file::{
    DeviceFileKind, device_file_kind, parse_bacnet_device_file, parse_modbus_device_file,
};
use voltrus_scada_native::runtime::{Call, CallKey, Reply};
use vt_ui::Icon;
use vt_ui::components::{Button, IconButton, empty_state, field, kbd, pill};
use vt_ui::text_input::{InputEvent, InputKind, TextInput};
use vt_ui::theme::*;

use super::sidebar_width;
use crate::console::Console;
use crate::console::keys::{
    DATA_DEVICES, DataAddChild, DataDeleteSource, DataDuplicateSource, DataImportFile,
    DataNewDevice, DataRefreshSection, DataReverseSort, DataRowActions, DataSectionTool,
    DataSortNext, DataToggleEnabled,
};
use crate::console::nav::Move;
use crate::console::overlay::Overlay;
use crate::console::pages::Role;
use crate::console::views::{
    ListKeys, focus_ring, keyboard_list, page_header, rows_on_screen, tail_row, tail_rows,
};

// ── The protocol tables ──────────────────────────────────────────────────

/// A protocol's call names, `data.<id>.<what>`, so the page routes their
/// answers to its section.
pub(crate) struct Calls {
    pub list: &'static str,
    pub children: &'static str,
    pub save: &'static str,
    pub delete: &'static str,
    pub toggle: &'static str,
    pub action: &'static str,
    pub probe: &'static str,
    pub import: &'static str,
    pub duplicate: &'static str,
}

macro_rules! calls {
    ($id:literal) => {
        $crate::console::views::data_manager::device_form::Calls {
            list: concat!("data.", $id, ".list"),
            children: concat!("data.", $id, ".children"),
            save: concat!("data.", $id, ".save"),
            delete: concat!("data.", $id, ".delete"),
            toggle: concat!("data.", $id, ".toggle"),
            action: concat!("data.", $id, ".action"),
            probe: concat!("data.", $id, ".probe"),
            import: concat!("data.", $id, ".import"),
            duplicate: concat!("data.", $id, ".duplicate"),
        }
    };
}
pub(crate) use calls;

/// The `SectionSpec` hooks of a protocol module over its `PROTOCOL`, but
/// `on_reply`, which a module may extend.
macro_rules! section_hooks {
    () => {
        pub fn render(
            console: &mut $crate::console::Console,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<$crate::console::Console>,
        ) -> gpui::AnyElement {
            super::device_form::render(&PROTOCOL, console, window, cx)
        }

        pub fn focus(
            console: &mut $crate::console::Console,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<$crate::console::Console>,
        ) {
            super::device_form::focus(&PROTOCOL, console, window, cx);
        }

        pub fn search(
            console: &$crate::console::Console,
        ) -> Option<gpui::Entity<vt_ui::text_input::TextInput>> {
            super::device_form::search(&PROTOCOL, console)
        }
    };
}
pub(crate) use section_hooks;

/// What a table cell shows.
#[derive(Clone, Copy)]
pub(crate) enum Cell {
    /// The row's field as it is.
    Key(&'static str),
    /// A number of milliseconds: `1000ms`.
    Ms(&'static str),
    /// A flag as On or Off.
    OnOff(&'static str),
    /// Computed from the row.
    Fn(fn(&Value) -> String),
    /// The `enabled` flag, as a pill.
    Enabled,
    /// What the server last said of the device's link.
    Link,
}

pub(crate) struct Column {
    pub title: &'static str,
    /// Pixels; `0.0` takes the rest (the name).
    pub width: f32,
    pub cell: Cell,
    pub mono: bool,
    /// When the window is narrow, columns go in descending order of this;
    /// `0` always stays.
    pub drop: u8,
}

pub(crate) const fn col(
    title: &'static str,
    width: f32,
    cell: Cell,
    mono: bool,
    drop: u8,
) -> Column {
    Column {
        title,
        width,
        cell,
        mono,
        drop,
    }
}

/// What a form field takes and how it goes into the request body.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Kind {
    /// Text; `required` refuses it blank.
    Text { required: bool, mono: bool },
    /// A password: never shown again, left out of the body when blank.
    Secret,
    /// A whole number within the range; blank takes the field's default,
    /// or is refused without one.
    Int { min: i64, max: i64 },
    /// A whole number, or `null` when blank.
    OptionalInt { min: i64, max: i64 },
    /// Any number; blank takes the default.
    Float,
    /// On or off (`"true"` / `"false"`).
    Flag,
    /// One of the values (value, label).
    Choice(&'static [(&'static str, &'static str)]),
}

pub(crate) struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub kind: Kind,
    pub default: &'static str,
    /// Only when creating (an id the server cannot change).
    pub create_only: bool,
}

pub(crate) const fn text(
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
) -> Field {
    Field {
        key,
        label,
        placeholder,
        kind: Kind::Text {
            required: true,
            mono: false,
        },
        default: "",
        create_only: false,
    }
}

pub(crate) const fn mono(
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
) -> Field {
    Field {
        key,
        label,
        placeholder,
        kind: Kind::Text {
            required: true,
            mono: true,
        },
        default: "",
        create_only: false,
    }
}

pub(crate) const fn optional(
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
) -> Field {
    Field {
        key,
        label,
        placeholder,
        kind: Kind::Text {
            required: false,
            mono: false,
        },
        default: "",
        create_only: false,
    }
}

pub(crate) const fn int(
    key: &'static str,
    label: &'static str,
    default: &'static str,
    min: i64,
    max: i64,
) -> Field {
    Field {
        key,
        label,
        placeholder: default,
        kind: Kind::Int { min, max },
        default,
        create_only: false,
    }
}

pub(crate) const fn flag(key: &'static str, label: &'static str, default: &'static str) -> Field {
    Field {
        key,
        label,
        placeholder: "",
        kind: Kind::Flag,
        default,
        create_only: false,
    }
}

pub(crate) const fn choice(
    key: &'static str,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
    default: &'static str,
) -> Field {
    Field {
        key,
        label,
        placeholder: "",
        kind: Kind::Choice(options),
        default,
        create_only: false,
    }
}

/// Reshape a form's body before sending (fallbacks, composite choices).
pub(crate) type Finish = fn(&mut Map<String, Value>, bool);

/// How a protocol's children are read and written.
pub(crate) enum ChildApi {
    /// `GET {device}/{segment}` (or `GET {device}` and its `within` list),
    /// `POST {device}/{segment}`, `DELETE {device}/{segment}/{child id}`.
    Rest {
        segment: &'static str,
        within: Option<&'static str>,
    },
    /// Modbus registers: read from `GET {device}`, written as a whole
    /// with `PUT {device}/registers`.
    ModbusRegisters,
}

pub(crate) struct Child {
    /// `Registers`.
    pub title: &'static str,
    /// `register`.
    pub noun: &'static str,
    pub api: ChildApi,
    pub decode: fn(Value) -> Result<Vec<Value>, String>,
    pub columns: &'static [Column],
    pub fields: &'static [Field],
    pub finish: Option<Finish>,
    /// Adding or removing one makes or drops a data source.
    pub makes_tags: bool,
}

/// What a tool acts on.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope {
    Device,
    Section,
}

/// A custom tool: a dialog of the protocol's own.
pub(crate) type Open =
    fn(&mut Console, &'static Protocol, Option<Value>, &mut Window, &mut Context<Console>);

pub(crate) enum ToolKind {
    /// `POST {device}/{segment}` without a body; the answer's `message`
    /// (or `done`) is the toast.
    Post {
        segment: &'static str,
        done: &'static str,
    },
    Probe(&'static ProbeSpec),
    Open(Open),
}

pub(crate) struct Tool {
    pub label: &'static str,
    pub icon: Icon,
    pub scope: Scope,
    pub kind: ToolKind,
    /// Offered only for rows this holds for (Start for a stopped device).
    pub when: Option<fn(&Value) -> bool>,
}

/// A form value per field key: text as typed, choices by value, flags as
/// `"true"` / `"false"`.
pub(crate) type Values = HashMap<&'static str, String>;

/// One result of a probe (a scan, a browse, a discovery).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProbeRow {
    pub label: String,
    pub detail: String,
    /// What picking it works with.
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProbeAnswer {
    /// A line above the results (S7: whether the PLC answered).
    pub note: Option<String>,
    pub rows: Vec<ProbeRow>,
}

/// A probe's path and body (`None`: no body), from the device and fields.
pub(crate) type Request = fn(Option<&Value>, &Values) -> Result<(String, Option<Value>), String>;

pub(crate) type OnPick = fn(
    &mut Console,
    &'static Protocol,
    Option<Value>,
    Vec<Value>,
    &mut Window,
    &mut Context<Console>,
);

/// A tool that asks the server and lists what it found.
pub(crate) struct ProbeSpec {
    pub title: &'static str,
    /// The line under the title, about the device if any.
    pub intro: fn(Option<&Value>) -> String,
    pub fields: &'static [Field],
    pub run_label: &'static str,
    /// Path and body (`None`: no body) from the device and the fields.
    pub request: Request,
    /// How long the server may take.
    pub timeout: fn(&Values) -> Duration,
    /// The answer as rows, given the fields it was asked with.
    pub decode: fn(Value, &Values) -> Result<ProbeAnswer, String>,
    /// Space picks several rows.
    pub multi: bool,
    pub pick_label: &'static str,
    pub on_pick: OnPick,
    pub empty: &'static str,
    /// Runs as it opens (no fields to fill).
    pub auto: bool,
}

/// One Protocols section.
pub(crate) struct Protocol {
    pub id: &'static str,
    pub title: &'static str,
    /// `Modbus device`.
    pub noun: &'static str,
    pub icon: Icon,
    /// The table's keyboard list.
    pub list_id: &'static str,
    pub calls: Calls,
    /// `/api/v1/modbus-devices`.
    pub base: &'static str,
    pub decode: fn(Value) -> Result<Vec<Value>, String>,
    /// Who may change devices (reads may be open to more).
    pub write_role: Role,
    /// The device's key in the WebSocket's `DeviceStatus` (`bacnet_7`).
    pub link: fn(&Value) -> String,
    pub columns: &'static [Column],
    /// The fields searched with `/`.
    pub search: &'static [&'static str],
    pub fields: &'static [Field],
    pub finish: Option<Finish>,
    pub child: Option<&'static Child>,
    pub tools: &'static [Tool],
    pub import: Option<DeviceFileKind>,
    /// D copies a device with its children (Modbus).
    pub duplicate: bool,
    /// The empty table's line.
    pub about: &'static str,
    /// The list route, when the core router may not serve it.
    pub unrouted: Option<&'static str>,
}

// ── Pure helpers ─────────────────────────────────────────────────────────

/// A field of a row as text: strings as they are, numbers and flags
/// written out, `null` and missing fields empty.
pub(crate) fn value_text(row: &Value, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// A row's id as a path segment.
pub(crate) fn row_id(row: &Value) -> String {
    value_text(row, "id")
}

/// `{base}/{id}`, the id escaped.
pub(crate) fn device_path(protocol: &Protocol, row: &Value) -> String {
    format!("{}/{}", protocol.base, encode(&row_id(row)))
}

fn enabled(row: &Value) -> bool {
    row.get("enabled").and_then(Value::as_bool).unwrap_or(true)
}

/// A device's name for messages: its name, else its id.
pub(crate) fn name_of(row: &Value) -> String {
    match value_text(row, "name") {
        name if name.trim().is_empty() => row_id(row),
        name => name,
    }
}

/// What a cell reads.
pub(crate) fn cell_text(cell: Cell, row: &Value) -> String {
    match cell {
        Cell::Key(key) => match value_text(row, key) {
            empty if empty.is_empty() => "—".to_owned(),
            text => text,
        },
        Cell::Ms(key) => format!("{}ms", value_text(row, key)),
        Cell::OnOff(key) => {
            if row.get(key).and_then(Value::as_bool).unwrap_or(false) {
                "On".to_owned()
            } else {
                "Off".to_owned()
            }
        }
        Cell::Fn(text) => text(row),
        Cell::Enabled => (if enabled(row) { "on" } else { "off" }).to_owned(),
        Cell::Link => String::new(),
    }
}

/// Whether `row` has `query` (any case) in one of `keys`.
pub(crate) fn matches(row: &Value, keys: &[&str], query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || keys
            .iter()
            .any(|key| value_text(row, key).to_lowercase().contains(&query))
}

/// Numbers by value, anything else by text in any case.
pub(crate) fn compare_text(a: &str, b: &str) -> Ordering {
    let number = |text: &str| text.trim_end_matches("ms").parse::<f64>().ok();
    match (number(a), number(b)) {
        (Some(a), Some(b)) => a.total_cmp(&b),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    }
}

/// The rows (indices of `list`) that match `query`, sorted by `cell`.
pub(crate) fn visible_rows(
    list: &[Value],
    keys: &[&str],
    query: &str,
    sort: Option<(Cell, bool)>,
) -> Vec<usize> {
    let mut rows: Vec<usize> = (0..list.len())
        .filter(|index| matches(&list[*index], keys, query))
        .collect();
    if let Some((cell, descending)) = sort {
        rows.sort_by(|a, b| {
            let order = compare_text(&cell_text(cell, &list[*a]), &cell_text(cell, &list[*b]));
            if descending { order.reverse() } else { order }
        });
    }
    rows
}

/// The form's values for `row` (or the defaults for a new one).
pub(crate) fn values_of(fields: &[Field], row: Option<&Value>) -> Values {
    fields
        .iter()
        .map(|field| {
            let stored = row
                .and_then(|row| row.get(field.key))
                .filter(|value| !value.is_null())
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                });
            let value = match field.kind {
                Kind::Secret => String::new(),
                Kind::Choice(options) => stored
                    .filter(|value| options.iter().any(|(option, _)| option == value))
                    .unwrap_or_else(|| field.default.to_owned()),
                Kind::OptionalInt { .. } if row.is_some() => stored.unwrap_or_default(),
                _ => stored.unwrap_or_else(|| field.default.to_owned()),
            };
            (field.key, value)
        })
        .collect()
}

/// The request body of a form, or why it cannot be sent.
pub(crate) fn body(
    fields: &[Field],
    values: &Values,
    editing: bool,
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for field in fields {
        if editing && field.create_only {
            continue;
        }
        let raw = values.get(field.key).map_or("", String::as_str).trim();
        let value = match field.kind {
            Kind::Text { required, .. } => {
                if required && raw.is_empty() {
                    return Err(format!("{} is required", field.label));
                }
                Value::from(raw)
            }
            Kind::Secret => {
                if raw.is_empty() {
                    continue;
                }
                Value::from(raw)
            }
            Kind::Int { min, max } => {
                let raw = if raw.is_empty() { field.default } else { raw };
                if raw.is_empty() {
                    return Err(format!("{} is required", field.label));
                }
                Value::from(whole(field.label, raw, min, max)?)
            }
            Kind::OptionalInt { min, max } => {
                if raw.is_empty() {
                    Value::Null
                } else {
                    Value::from(whole(field.label, raw, min, max)?)
                }
            }
            Kind::Float => {
                let raw = if raw.is_empty() { field.default } else { raw };
                let number: f64 = raw
                    .parse()
                    .ok()
                    .filter(|number: &f64| number.is_finite())
                    .ok_or_else(|| format!("{} must be a number", field.label))?;
                Value::from(number)
            }
            Kind::Flag => Value::from(raw == "true"),
            Kind::Choice(options) => {
                if !options.iter().any(|(value, _)| *value == raw) {
                    return Err(format!("Choose a {}", field.label.to_lowercase()));
                }
                Value::from(raw)
            }
        };
        out.insert(field.key.to_owned(), value);
    }
    Ok(out)
}

fn whole(label: &str, raw: &str, min: i64, max: i64) -> Result<i64, String> {
    raw.parse::<i64>()
        .ok()
        .filter(|number| (min..=max).contains(number))
        .ok_or_else(|| format!("{label} must be a whole number from {min} to {max}"))
}

/// One line of the table: a device, its children's header, a child, or
/// the line under them (loading, empty, failed, add).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Device(usize),
    Header(usize),
    Child(usize, usize),
    Note(usize),
}

/// The table's lines: `devices` in order, each expanded one followed by
/// its children (`children(device)`: `None` while unknown).
pub(crate) fn lines(
    devices: &[usize],
    expanded: impl Fn(usize) -> bool,
    children: impl Fn(usize) -> Option<usize>,
) -> Vec<Row> {
    let mut out = Vec::with_capacity(devices.len());
    for &device in devices {
        out.push(Row::Device(device));
        if !expanded(device) {
            continue;
        }
        match children(device) {
            Some(count) if count > 0 => {
                out.push(Row::Header(device));
                out.extend((0..count).map(|child| Row::Child(device, child)));
                out.push(Row::Note(device));
            }
            _ => out.push(Row::Note(device)),
        }
    }
    out
}

// ── State ────────────────────────────────────────────────────────────────

/// What a list call answered.
pub(crate) enum Listed {
    Rows(Vec<Value>),
    /// The server has no such route; why.
    Unavailable(String),
}

#[derive(Default)]
pub(crate) struct Section {
    search: Option<Entity<TextInput>>,
    _search: Option<Subscription>,
    asked: bool,
    loading: bool,
    pub list: Option<Result<Rc<Vec<Value>>, String>>,
    unavailable: Option<String>,
    /// Children by device id.
    pub children: HashMap<String, Result<Vec<Value>, String>>,
    expanded: HashSet<String>,
    /// Sorted by this device column, descending when set.
    sort: Option<(usize, bool)>,
    /// The import's report, shown until dismissed.
    notice: Vec<String>,
    importing: bool,
    /// Devices with a tool or toggle on its way.
    busy: HashSet<String>,
    /// Modbus poller states by device id.
    pub status: HashMap<String, String>,
    /// Put the cursor on this device once listed.
    focus_id: Option<String>,
    /// A key pressed before the list arrived (a launch's `--overlay`).
    pending: Option<Pending>,
}

/// What a key asked for before there was a row to do it on.
#[derive(Clone, Copy)]
enum Pending {
    AddChild,
    Menu,
}

/// Hold `pending` for the list, if it has not come yet; whether it is held.
fn wait_for_list(protocol: &'static Protocol, console: &mut Console, pending: Pending) -> bool {
    let state = section(console, protocol);
    if state.list.is_some() {
        return false;
    }
    state.pending = Some(pending);
    true
}

/// Run a held key on the first device once the list is there (replies
/// have no window, so it runs right after this update).
fn replay(
    protocol: &'static Protocol,
    console: &mut Console,
    pending: Pending,
    cx: &mut Context<Console>,
) {
    let Some(handle) = console.state::<Devices>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                let (rows, list) = table(protocol, console, cx);
                let Some(device) = rows.first().and_then(|row| device_at(&list, *row)) else {
                    return;
                };
                console.list(protocol.list_id, cx).select(0);
                match pending {
                    Pending::AddChild => {
                        open_child_form(protocol, console, device, None, window, cx);
                    }
                    Pending::Menu => open_menu(protocol, console, Some(device), window, cx),
                }
            });
        });
    });
}

/// What the dialogs are about.
enum Target {
    Device(Option<Value>),
    Child(Value),
}

struct Form {
    protocol: &'static Protocol,
    target: Target,
    fields: &'static [Field],
    inputs: HashMap<&'static str, Entity<TextInput>>,
    values: Values,
    error: Option<String>,
    saving: bool,
    /// A duplicate's children, sent with the new device.
    carry: Option<Vec<Value>>,
    /// Shown above the fields.
    note: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct Probe {
    protocol: &'static Protocol,
    spec: &'static ProbeSpec,
    device: Option<Value>,
    inputs: HashMap<&'static str, Entity<TextInput>>,
    values: Values,
    running: bool,
    seq: i64,
    answer: Option<Result<ProbeAnswer, String>>,
    picked: BTreeSet<usize>,
    _subscriptions: Vec<Subscription>,
}

enum Doomed {
    Device(Value),
    Child(Value, Value),
}

struct Confirm {
    protocol: &'static Protocol,
    doomed: Doomed,
    focus: FocusHandle,
}

#[derive(Clone, Copy)]
enum MenuItem {
    Edit,
    AddChild,
    Toggle,
    Duplicate,
    Tool(usize),
    Import,
    Refresh,
    Delete,
}

struct Menu {
    protocol: &'static Protocol,
    row: Option<Value>,
    items: Vec<(MenuItem, SharedString, Icon)>,
}

/// Every Protocols section's state, and the one dialog open among them.
#[derive(Default)]
pub(crate) struct Devices {
    sections: HashMap<&'static str, Section>,
    form: Option<Form>,
    probe: Option<Probe>,
    confirm: Option<Confirm>,
    menu: Option<Menu>,
    window: Option<AnyWindowHandle>,
    seq: i64,
}

impl Devices {
    pub(crate) fn section(&mut self, id: &'static str) -> &mut Section {
        self.sections.entry(id).or_default()
    }

    fn section_ref(&self, id: &str) -> Option<&Section> {
        self.sections.get(id)
    }
}

pub(crate) fn section<'a>(console: &'a mut Console, protocol: &Protocol) -> &'a mut Section {
    console.state::<Devices>().section(protocol.id)
}

const FORM: &str = "data-device-form";
const PROBE: &str = "data-device-probe";
const CONFIRM: &str = "data-device-delete";
const MENU: &str = "data-device-menu";
const PROBE_LIST: &str = "data.devices.probe";
const MENU_LIST: &str = "data.devices.menu";
const ROW_HEIGHT: f32 = 32.0;
const NAME_MIN: f32 = 140.0;
const ACTIONS: f32 = 176.0;
const INDENT: f32 = 34.0;

// ── Hooks ────────────────────────────────────────────────────────────────

/// The section came on screen: fetch the list once.
pub(crate) fn on_open(protocol: &'static Protocol, console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    let state = section(console, protocol);
    if state.asked {
        return;
    }
    state.asked = true;
    fetch(protocol, console);
}

pub(crate) fn focus(
    protocol: &'static Protocol,
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    console.focus_list(protocol.list_id, window, cx);
}

pub(crate) fn search(protocol: &'static Protocol, console: &Console) -> Option<Entity<TextInput>> {
    console
        .state_ref::<Devices>()?
        .section_ref(protocol.id)?
        .search
        .clone()
}

/// Fetch the device list again.
pub(crate) fn fetch(protocol: &'static Protocol, console: &mut Console) {
    if console.stress.is_some() {
        return;
    }
    section(console, protocol).loading = true;
    let path = protocol.base.to_owned();
    let decode = protocol.decode;
    let unrouted = protocol.unrouted;
    console.call(
        CallKey::new(protocol.calls.list),
        Call::new(move |api| async move {
            match api.get_json::<Value>(&path).await {
                Ok(answer) => Ok(match decode(answer) {
                    Ok(rows) => Listed::Rows(rows),
                    Err(error) => anyhow::bail!("unexpected answer: {error}"),
                }),
                Err(error) if not_routed(&error) => Ok(Listed::Unavailable(
                    unrouted.map_or_else(|| error.to_string(), str::to_owned),
                )),
                Err(error) => Err(error),
            }
        }),
    );
}

fn fetch_children(protocol: &'static Protocol, console: &mut Console, row: &Value) {
    let Some(child) = protocol.child else {
        return;
    };
    let id = row_id(row);
    let device = device_path(protocol, row);
    let (path, within) = match child.api {
        ChildApi::Rest { segment, within } => match within {
            Some(key) => (device, Some(key)),
            None => (format!("{device}/{segment}"), None),
        },
        ChildApi::ModbusRegisters => (device, Some("registers")),
    };
    let decode = child.decode;
    console.call(
        CallKey::new(protocol.calls.children),
        Call::new(move |api| async move {
            let answer = api
                .get_json::<Value>(&path)
                .await
                .map_err(|error| error.to_string())
                .and_then(|mut answer| match within {
                    Some(key) => answer
                        .get_mut(key)
                        .map(Value::take)
                        .ok_or_else(|| format!("the answer has no `{key}`")),
                    None => Ok(answer),
                })
                .and_then(decode);
            anyhow::Ok((id, answer))
        }),
    );
}

/// Whether the console may change this protocol's devices.
pub(crate) fn may_write(protocol: &Protocol, console: &Console) -> bool {
    console.allows(protocol.write_role)
}

fn refuse(protocol: &Protocol, console: &mut Console, cx: &mut Context<Console>) -> bool {
    if may_write(protocol, console) {
        return false;
    }
    console.show_toast(
        format!(
            "Changing {} devices needs the {} role",
            protocol.title,
            role_name(protocol.write_role)
        ),
        true,
        cx,
    );
    true
}

pub(crate) fn role_name(role: Role) -> &'static str {
    match role {
        Role::Any => "viewer",
        Role::Operator => "operator (admin, demo or integration)",
        Role::Admin => "admin",
    }
}

/// The answers to the section's calls.
pub(crate) fn on_reply(
    protocol: &'static Protocol,
    console: &mut Console,
    key: CallKey,
    reply: Reply,
    cx: &mut Context<Console>,
) {
    let calls = &protocol.calls;
    let name = key.name;
    if name == calls.list {
        let state = section(console, protocol);
        state.loading = false;
        match reply.take::<Listed>() {
            Ok(Listed::Rows(rows)) => {
                state.unavailable = None;
                // Children of devices that are gone are dropped.
                let ids: HashSet<String> = rows.iter().map(row_id).collect();
                state.children.retain(|id, _| ids.contains(id));
                state.expanded.retain(|id| ids.contains(id));
                state.list = Some(Ok(Rc::new(rows)));
            }
            Ok(Listed::Unavailable(reason)) => {
                state.unavailable = Some(reason);
                state.list = Some(Ok(Rc::new(Vec::new())));
            }
            Err(error) => state.list = Some(Err(error)),
        }
        // Expanded rows show what the server has now.
        let expanded: Vec<Value> = expanded_rows(protocol, console);
        for row in expanded {
            fetch_children(protocol, console, &row);
        }
        if let Some(pending) = section(console, protocol).pending.take() {
            replay(protocol, console, pending, cx);
        }
    } else if name == calls.children {
        if let Ok((id, answer)) = reply.take::<(String, Result<Vec<Value>, String>)>() {
            section(console, protocol).children.insert(id, answer);
        }
    } else if name == calls.save {
        match reply.take::<Saved>() {
            Ok(saved) => {
                after_change(protocol, console, saved.device.as_deref());
                if saved.device.is_some() && protocol.child.is_some_and(|child| child.makes_tags) {
                    console.refresh_tags();
                }
                if let Some(id) = saved.created {
                    section(console, protocol).focus_id = Some(id);
                }
                console.show_toast(saved.message, false, cx);
                close_later(console, FORM, cx);
            }
            Err(error) => match console.state::<Devices>().form.as_mut() {
                Some(form) => {
                    form.saving = false;
                    form.error = Some(error);
                }
                None => console.show_toast(format!("Saving failed: {error}"), true, cx),
            },
        }
    } else if name == calls.delete {
        match reply.take::<Saved>() {
            Ok(saved) => {
                after_change(protocol, console, saved.device.as_deref());
                if protocol.child.is_some_and(|child| child.makes_tags) {
                    console.refresh_tags();
                }
                console.show_toast(saved.message, false, cx);
            }
            Err(error) => {
                fetch(protocol, console);
                console.show_toast(format!("Delete failed: {error}"), true, cx);
            }
        }
    } else if name == calls.toggle || name == calls.action {
        match reply.take::<(String, String)>() {
            Ok((id, message)) => {
                section(console, protocol).busy.remove(&id);
                fetch(protocol, console);
                console.show_toast(message, false, cx);
            }
            Err(error) => {
                section(console, protocol).busy.clear();
                fetch(protocol, console);
                console.show_toast(error, true, cx);
            }
        }
    } else if name == calls.probe {
        let answer = reply.take::<Result<ProbeAnswer, String>>();
        if let Some(probe) = console.state::<Devices>().probe.as_mut()
            && probe.seq == key.arg
        {
            probe.running = false;
            probe.picked.clear();
            probe.answer = Some(answer.and_then(|answer| answer));
        }
    } else if name == calls.import {
        let state = section(console, protocol);
        state.importing = false;
        match reply.take::<Vec<String>>() {
            Ok(lines) => {
                state.notice = lines;
                fetch(protocol, console);
                console.refresh_tags();
            }
            Err(error) => console.show_toast(format!("Import failed: {error}"), true, cx),
        }
    } else if name == calls.duplicate {
        if let Ok((id, Ok(children))) = reply.take::<(String, Result<Vec<Value>, String>)>()
            && let Some(form) = console.state::<Devices>().form.as_mut()
            && form.carry.is_some()
        {
            form.note = Some(format!(
                "Copies the {} {} of {id}.",
                children.len(),
                form.protocol
                    .child
                    .map_or("children", |child| child.title)
                    .to_lowercase()
            ));
            form.carry = Some(children);
        }
    } else {
        return;
    }
    cx.notify();
}

fn expanded_rows(protocol: &Protocol, console: &mut Console) -> Vec<Value> {
    let state = section(console, protocol);
    match &state.list {
        Some(Ok(list)) => list
            .iter()
            .filter(|row| state.expanded.contains(&row_id(row)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// A device or a child changed: fetch the list (and that device's
/// children) again.
fn after_change(protocol: &'static Protocol, console: &mut Console, device: Option<&str>) {
    fetch(protocol, console);
    if let Some(id) = device {
        let state = section(console, protocol);
        state.children.remove(id);
        state.expanded.insert(id.to_owned());
    }
}

/// What a save or a delete answered.
pub(crate) struct Saved {
    /// The device whose children changed.
    device: Option<String>,
    message: String,
    /// A new device's id, for the cursor.
    created: Option<String>,
}

impl Saved {
    pub(crate) fn device(message: String) -> Self {
        Self {
            device: None,
            message,
            created: None,
        }
    }

    pub(crate) fn child(device: String, message: String) -> Self {
        Self {
            device: Some(device),
            message,
            created: None,
        }
    }
}

/// Close dialog `id` once the server answered (replies have no window).
fn close_later(console: &mut Console, id: &'static str, cx: &mut Context<Console>) {
    let Some(handle) = console.state::<Devices>().window else {
        return;
    };
    let entity = cx.entity();
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            entity.update(cx, |console, cx| {
                if console.overlay_is(id) {
                    console.close_overlay(window, cx);
                }
                let state = console.state::<Devices>();
                match id {
                    FORM => state.form = None,
                    PROBE => state.probe = None,
                    _ => {}
                }
            });
        });
    });
}

// ── The table ────────────────────────────────────────────────────────────

fn ensure_search(
    protocol: &'static Protocol,
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if section(console, protocol).search.is_some() {
        return;
    }
    let field = cx.new(|cx| TextInput::new(cx).placeholder("Search  /"));
    let list = protocol.list_id;
    let subscription = cx.subscribe_in(
        &field,
        window,
        move |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Changed => {
                console.list(list, cx).cursor = None;
                cx.notify();
            }
            InputEvent::Submitted | InputEvent::Cancelled => {
                let nav = console.list(list, cx);
                nav.step(Move::First);
                window.focus(&nav.focus);
                cx.notify();
            }
            InputEvent::Blurred => {}
        },
    );
    let state = section(console, protocol);
    state.search = Some(field);
    state._search = Some(subscription);
}

/// The table's lines for this render, and the list they index.
fn table(
    protocol: &'static Protocol,
    console: &mut Console,
    cx: &App,
) -> (Rc<[Row]>, Rc<Vec<Value>>) {
    let state = section(console, protocol);
    let query = state
        .search
        .as_ref()
        .map(|field| field.read(cx).value().to_owned())
        .unwrap_or_default();
    let list = match &state.list {
        Some(Ok(list)) => Rc::clone(list),
        _ => Rc::new(Vec::new()),
    };
    let sort = state
        .sort
        .and_then(|(column, descending)| Some((protocol.columns.get(column)?.cell, descending)));
    let devices = visible_rows(&list, protocol.search, &query, sort);
    let rows = lines(
        &devices,
        |index| state.expanded.contains(&row_id(&list[index])),
        |index| match state.children.get(&row_id(&list[index])) {
            Some(Ok(children)) => Some(children.len()),
            _ => None,
        },
    );
    (rows.into(), list)
}

fn cursor_row(
    protocol: &'static Protocol,
    console: &mut Console,
    cx: &mut Context<Console>,
) -> Option<(Row, Rc<Vec<Value>>)> {
    let cursor = console.list(protocol.list_id, cx).cursor?;
    let (rows, list) = table(protocol, console, cx);
    rows.get(cursor).map(|row| (*row, list))
}

fn device_at(list: &[Value], row: Row) -> Option<Value> {
    match row {
        Row::Device(index) | Row::Header(index) | Row::Child(index, _) | Row::Note(index) => {
            list.get(index).cloned()
        }
    }
}

fn child_at(
    protocol: &'static Protocol,
    console: &mut Console,
    device: &Value,
    child: usize,
) -> Option<Value> {
    match section(console, protocol).children.get(&row_id(device)) {
        Some(Ok(children)) => children.get(child).cloned(),
        _ => None,
    }
}

fn toggle_expand(
    protocol: &'static Protocol,
    console: &mut Console,
    row: &Value,
    cx: &mut Context<Console>,
) {
    if protocol.child.is_none() {
        return;
    }
    let id = row_id(row);
    let state = section(console, protocol);
    if !state.expanded.remove(&id) {
        state.expanded.insert(id.clone());
        if !state.children.contains_key(&id) {
            fetch_children(protocol, console, row);
        }
    }
    cx.notify();
}

fn activate(
    protocol: &'static Protocol,
    console: &mut Console,
    at: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let (rows, list) = table(protocol, console, cx);
    let Some(row) = rows.get(at).copied() else {
        return;
    };
    let Some(device) = device_at(&list, row) else {
        return;
    };
    match row {
        Row::Device(_) if toggle => toggle_expand(protocol, console, &device, cx),
        Row::Device(_) => open_device_form(protocol, console, Some(device), window, cx),
        Row::Header(_) | Row::Note(_) if !toggle => {
            open_child_form(protocol, console, device, None, window, cx)
        }
        Row::Child(..) | Row::Header(_) | Row::Note(_) => {}
    }
}

/// Delete what the cursor is on.
fn ask_delete_cursor(
    protocol: &'static Protocol,
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some((row, list)) = cursor_row(protocol, console, cx) else {
        return;
    };
    let Some(device) = device_at(&list, row) else {
        return;
    };
    match row {
        Row::Device(_) => ask_delete(protocol, console, Doomed::Device(device), window, cx),
        Row::Child(_, child) => {
            if let Some(child) = child_at(protocol, console, &device, child) {
                ask_delete(protocol, console, Doomed::Child(device, child), window, cx);
            }
        }
        _ => {}
    }
}

fn toggle_enabled(
    protocol: &'static Protocol,
    console: &mut Console,
    device: &Value,
    cx: &mut Context<Console>,
) {
    if refuse(protocol, console, cx) {
        return;
    }
    let id = row_id(device);
    let now = !enabled(device);
    let path = device_path(protocol, device);
    let name = name_of(device);
    section(console, protocol).busy.insert(id.clone());
    console.call(
        CallKey::new(protocol.calls.toggle),
        Call::new(move |api| async move {
            api.send_json::<_, IgnoredAny>(Method::PUT, &path, &json!({ "enabled": now }))
                .await?;
            anyhow::Ok((
                id,
                format!("{name} {}", if now { "enabled" } else { "disabled" }),
            ))
        }),
    );
    cx.notify();
}

/// Run a device tool or open its dialog.
pub(crate) fn run_tool(
    protocol: &'static Protocol,
    console: &mut Console,
    tool: &'static Tool,
    device: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if refuse(protocol, console, cx) {
        return;
    }
    match &tool.kind {
        ToolKind::Post { segment, done } => {
            let Some(device) = device else {
                return;
            };
            let id = row_id(&device);
            let path = format!("{}/{segment}", device_path(protocol, &device));
            let name = name_of(&device);
            let done = *done;
            section(console, protocol).busy.insert(id.clone());
            console.call(
                CallKey::new(protocol.calls.action),
                Call::new(move |api| async move {
                    match api.send_empty::<Value>(Method::POST, &path).await {
                        Ok(answer) => {
                            let message = answer
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or(done);
                            anyhow::Ok((id, format!("{name}: {message}")))
                        }
                        Err(error) => anyhow::bail!("{name}: {error}"),
                    }
                }),
            );
            cx.notify();
        }
        ToolKind::Probe(spec) => open_probe(protocol, console, spec, device, window, cx),
        ToolKind::Open(open) => open(console, protocol, device, window, cx),
    }
}

/// The tools a device row offers.
fn device_tools(protocol: &'static Protocol, row: &Value) -> Vec<(usize, &'static Tool)> {
    protocol
        .tools
        .iter()
        .enumerate()
        .filter(|(_, tool)| tool.scope == Scope::Device && tool.when.is_none_or(|when| when(row)))
        .collect()
}

fn set_sort(
    protocol: &'static Protocol,
    console: &mut Console,
    column: usize,
    cx: &mut Context<Console>,
) {
    let state = section(console, protocol);
    state.sort = match state.sort {
        Some((current, descending)) if current == column => Some((column, !descending)),
        _ => Some((column, false)),
    };
    console.list(protocol.list_id, cx).cursor = None;
    cx.notify();
}

fn sort_next(protocol: &'static Protocol, console: &mut Console, cx: &mut Context<Console>) {
    let sortable: Vec<usize> = protocol
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| !matches!(column.cell, Cell::Link))
        .map(|(index, _)| index)
        .collect();
    let state = section(console, protocol);
    let next = match state.sort {
        Some((current, _)) => sortable
            .iter()
            .position(|column| *column == current)
            .and_then(|at| sortable.get(at + 1))
            .copied(),
        None => sortable.first().copied(),
    };
    state.sort = next.map(|column| (column, false));
    cx.notify();
}

fn reverse_sort(protocol: &'static Protocol, console: &mut Console, cx: &mut Context<Console>) {
    let state = section(console, protocol);
    state.sort = Some(match state.sort {
        Some((column, descending)) => (column, !descending),
        None => (0, true),
    });
    cx.notify();
}

/// Which columns fit in `width` (a bit per column) and how wide the name
/// column is: the least useful columns drop until the name has room.
pub(crate) fn fit(columns: &[Column], width: f32, reserved: f32) -> (u32, f32) {
    let gap = f32::from(SPACE_5);
    let room = width - reserved - f32::from(SPACE_8) * 2.0 - 16.0 - gap * 2.0;
    let name = |shown: u32| {
        let (used, count) = columns
            .iter()
            .enumerate()
            .filter(|(index, _)| shown & (1 << index) != 0)
            .fold((0.0, 0.0), |(used, count), (_, column)| {
                (used + column.width, count + 1.0)
            });
        room - used - gap * count
    };
    let mut shown: u32 = (1u32 << columns.len().min(31)) - 1;
    let mut order: Vec<usize> = (0..columns.len())
        .filter(|index| columns[*index].drop > 0)
        .collect();
    order.sort_by_key(|index| std::cmp::Reverse(columns[*index].drop));
    for index in order {
        if name(shown) >= NAME_MIN {
            break;
        }
        shown &= !(1 << index);
    }
    (shown, name(shown).max(NAME_MIN))
}

/// A device's link as text: the Modbus poller's state, else the last
/// WebSocket `DeviceStatus`.
fn link_text(protocol: &Protocol, console: &Console, row: &Value) -> Option<(String, bool)> {
    if !enabled(row) {
        return Some(("disabled".to_owned(), false));
    }
    let key = (protocol.link)(row);
    if let Some(status) = console
        .state_ref::<Devices>()
        .and_then(|state| state.section_ref(protocol.id))
        .and_then(|section| section.status.get(&key))
    {
        return Some((status.clone(), status == "online"));
    }
    console.devices.get(&key).map(|link| {
        let text = match (link.online, link.latency_ms) {
            (true, Some(ms)) => format!("online · {ms}ms"),
            (true, None) => "online".to_owned(),
            (false, _) => "offline".to_owned(),
        };
        (text, link.online)
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn render(
    protocol: &'static Protocol,
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    ensure_search(protocol, console, window, cx);
    console.state::<Devices>().window = Some(window.window_handle());
    let theme = theme(cx).clone();
    let (rows, list) = table(protocol, console, cx);
    let writable = may_write(protocol, console);
    let state = section(console, protocol);
    let search_field = state.search.clone();
    let loading = state.loading && state.list.is_none();
    let failed = match &state.list {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let unavailable = state.unavailable.clone();
    let notice = state.notice.clone();
    let importing = state.importing;
    let sort = state.sort;
    let focus_id = state.focus_id.clone();
    let shown_devices = rows
        .iter()
        .filter(|row| matches!(row, Row::Device(_)))
        .count();
    let total = list.len();

    if let Some(id) = focus_id
        && let Some(at) = rows
            .iter()
            .position(|row| matches!(row, Row::Device(index) if row_id(&list[*index]) == id))
    {
        section(console, protocol).focus_id = None;
        console.list(protocol.list_id, cx).select(at);
    }

    let width = f32::from(window.viewport_size().width - sidebar_width());
    let (shown_columns, name) = fit(protocol.columns, width, ACTIONS);

    let subtitle = match (&unavailable, total) {
        (Some(_), _) => "Unavailable on this server".to_owned(),
        _ if !writable => format!(
            "{total} {}s · read-only: changing them needs the {} role",
            protocol.noun,
            role_name(protocol.write_role)
        ),
        _ => {
            let child = protocol.child.map_or(String::new(), |child| {
                format!(" · A add {} · Space expand", child.noun)
            });
            let scan = protocol
                .tools
                .iter()
                .find(|tool| tool.scope == Scope::Section)
                .map_or(String::new(), |tool| {
                    format!(" · T {}", tool.label.to_lowercase())
                });
            format!(
                "{total} {}s · N new · ↵ edit{child} · E enable · . actions{scan} · ⌫ delete",
                protocol.noun
            )
        }
    };

    let mut controls = vec![
        div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .w(px(200.0))
            .child(vt_ui::icon(Icon::Search, ICON_MD, theme.fg_dim))
            .children(search_field.map(|field| div().flex_1().child(field)))
            .into_any_element(),
    ];
    if unavailable.is_none() && writable {
        for (index, tool) in protocol.tools.iter().enumerate() {
            if tool.scope != Scope::Section {
                continue;
            }
            controls.push(
                Button::new(
                    ElementId::NamedInteger(
                        format!("data-{}-tool", protocol.id).into(),
                        index as u64,
                    ),
                    tool.label,
                )
                .icon(tool.icon)
                .on_click(cx.listener(move |console, _, window, cx| {
                    run_tool(protocol, console, &protocol.tools[index], None, window, cx);
                }))
                .into_any_element(),
            );
        }
        if protocol.import.is_some() {
            controls.push(
                Button::new(
                    SharedString::from(format!("data-{}-import", protocol.id)),
                    "Import…",
                )
                .icon(Icon::Upload)
                .tooltip("I · or drop device files here")
                .disabled(importing)
                .on_click(
                    cx.listener(move |console, _, _, cx| prompt_import(protocol, console, cx)),
                )
                .into_any_element(),
            );
        }
        controls.push(
            Button::new(
                SharedString::from(format!("data-{}-new", protocol.id)),
                "New device",
            )
            .primary()
            .icon(Icon::Plus)
            .tooltip("N")
            .on_click(cx.listener(move |console, _, window, cx| {
                open_device_form(protocol, console, None, window, cx);
            }))
            .into_any_element(),
        );
    }

    let arrow = |column: usize| match sort {
        Some((at, descending)) if at == column => Some(if descending { "↓" } else { "↑" }),
        _ => None,
    };
    let header = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(30.0))
        .px(SPACE_8)
        .bg(theme.surface)
        .border_b_1()
        .border_color(theme.border)
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.fg_dim)
        .child(div().w(px(16.0)).flex_none())
        .children(
            protocol
                .columns
                .iter()
                .enumerate()
                .filter(|(index, _)| shown_columns & (1 << index) != 0)
                .map(|(index, column)| {
                    let sorted = arrow(index);
                    div()
                        .id(ElementId::NamedInteger(
                            format!("data-{}-sort", protocol.id).into(),
                            index as u64,
                        ))
                        .flex()
                        .items_center()
                        .gap(SPACE_1)
                        .flex_none()
                        .w(px(if column.width > 0.0 {
                            column.width
                        } else {
                            name
                        }))
                        .cursor_pointer()
                        .when(sorted.is_some(), |this| this.text_color(theme.fg))
                        .hover(|style| style.text_color(theme.fg))
                        .child(column.title.to_uppercase())
                        .children(sorted)
                        .on_click(cx.listener(move |console, _, _, cx| {
                            set_sort(protocol, console, index, cx)
                        }))
                }),
        )
        .child(div().w(px(ACTIONS)).flex_none());

    let list_keys = ListKeys {
        id: protocol.list_id,
        on_move: None,
        // List hooks are plain fns: the table drawn last is the one with
        // the keys (one section is on screen at a time).
        on_activate: |console, row, toggle, window, cx| {
            if let Some(protocol) = console.state::<Active>().0 {
                activate(protocol, console, row, toggle, window, cx);
            }
        },
    };
    console.state::<Active>().0 = Some(protocol);
    let keyboard = keyboard_list(
        list_keys,
        rows.len(),
        rows_on_screen(window, ROW_HEIGHT),
        console,
        cx,
    );
    let nav = console.list(protocol.list_id, cx);
    // A focused table keeps a cursor once it has rows, so the keys act.
    if nav.cursor.is_none() && !rows.is_empty() && nav.focus.is_focused(window) {
        nav.cursor = Some(0);
    }
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());

    let body = if let Some(reason) = unavailable {
        empty_state(
            Icon::Unplug,
            format!("{} is not available on this server", protocol.title),
            reason,
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if loading {
        empty_state(
            protocol.icon,
            "Loading…",
            format!("Asking the server for its {}s.", protocol.noun),
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else if let Some(error) = failed {
        empty_state(
            Icon::CircleAlert,
            format!("The {}s did not load", protocol.noun),
            error,
            vec![
                Button::new(
                    SharedString::from(format!("data-{}-retry", protocol.id)),
                    "Try again",
                )
                .icon(Icon::RefreshCw)
                .on_click(cx.listener(move |console, _, _, cx| {
                    fetch(protocol, console);
                    cx.notify();
                }))
                .into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    } else if shown_devices == 0 {
        empty_state(
            if total == 0 {
                protocol.icon
            } else {
                Icon::Search
            },
            if total == 0 {
                format!("No {}s yet", protocol.noun)
            } else {
                format!("No {} matches", protocol.noun)
            },
            if total == 0 {
                protocol.about.to_owned()
            } else {
                "Nothing has this text in its name or address.".to_owned()
            },
            Vec::new(),
            cx,
        )
        .into_any_element()
    } else {
        let look = Look {
            cursor,
            focused,
            name,
            shown: shown_columns,
            width,
            writable,
        };
        let shown = rows.len();
        uniform_list(
            SharedString::from(format!("data-{}-rows", protocol.id)),
            shown + tail_rows(shown, ROW_HEIGHT),
            cx.processor(
                move |console: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                    let theme = vt_ui::theme::theme(cx).clone();
                    range
                        .map(|at| match rows.get(at).copied() {
                            Some(row) => {
                                draw_row(protocol, console, &list, row, at, look, &theme, cx)
                            }
                            None => tail_row(ROW_HEIGHT),
                        })
                        .collect::<Vec<_>>()
                },
            ),
        )
        .track_scroll(scroll)
        .size_full()
        .into_any_element()
    };

    let notice_card = (!notice.is_empty()).then(|| {
        div()
            .flex()
            .flex_none()
            .gap(SPACE_4)
            .mx(SPACE_8)
            .mt(SPACE_5)
            .p(SPACE_5)
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(vt_ui::icon(Icon::Info, ICON_MD, theme.info))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(SPACE_1)
                    .text_size(FONT_SMALL)
                    .font_family(MONO)
                    .children(notice.iter().map(|line| div().child(line.clone()))),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("data-{}-notice", protocol.id)),
                    Icon::X,
                )
                .tooltip("Dismiss")
                .on_click(cx.listener(move |console, _, _, cx| {
                    section(console, protocol).notice.clear();
                    cx.notify();
                })),
            )
    });

    let accept_drop = protocol.import.is_some() && writable;
    let accent = theme.accent;
    let accent_soft = theme.accent_soft;
    div()
        .flex()
        .flex_col()
        .size_full()
        .relative()
        .child(page_header(protocol.title, subtitle, controls, cx))
        .children(notice_card)
        .child(
            div()
                .key_context(DATA_DEVICES)
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .on_action(cx.listener(move |console, _: &DataNewDevice, window, cx| {
                    open_device_form(protocol, console, None, window, cx);
                }))
                .on_action(cx.listener(
                    move |console, _: &DataAddChild, window, cx| match cursor_row(
                        protocol, console, cx,
                    ) {
                        Some((row, list)) => {
                            if let Some(device) = device_at(&list, row) {
                                open_child_form(protocol, console, device, None, window, cx);
                            }
                        }
                        None => {
                            wait_for_list(protocol, console, Pending::AddChild);
                        }
                    },
                ))
                .on_action(
                    cx.listener(move |console, _: &DataDeleteSource, window, cx| {
                        ask_delete_cursor(protocol, console, window, cx);
                    }),
                )
                .on_action(cx.listener(move |console, _: &DataToggleEnabled, _, cx| {
                    if let Some((row @ Row::Device(_), list)) = cursor_row(protocol, console, cx)
                        && let Some(device) = device_at(&list, row)
                    {
                        toggle_enabled(protocol, console, &device, cx);
                    }
                }))
                .on_action(
                    cx.listener(move |console, _: &DataDuplicateSource, window, cx| {
                        if let Some((row @ Row::Device(_), list)) =
                            cursor_row(protocol, console, cx)
                            && let Some(device) = device_at(&list, row)
                        {
                            duplicate(protocol, console, device, window, cx);
                        }
                    }),
                )
                .on_action(cx.listener(move |console, _: &DataRowActions, window, cx| {
                    let Some((row, list)) = cursor_row(protocol, console, cx) else {
                        if !wait_for_list(protocol, console, Pending::Menu) {
                            open_menu(protocol, console, None, window, cx);
                        }
                        return;
                    };
                    let device = matches!(row, Row::Device(_))
                        .then(|| device_at(&list, row))
                        .flatten();
                    open_menu(protocol, console, device, window, cx);
                }))
                .on_action(
                    cx.listener(move |console, _: &DataSectionTool, window, cx| {
                        if let Some(tool) = protocol
                            .tools
                            .iter()
                            .find(|tool| tool.scope == Scope::Section)
                        {
                            run_tool(protocol, console, tool, None, window, cx);
                        }
                    }),
                )
                .on_action(cx.listener(move |console, _: &DataImportFile, _, cx| {
                    prompt_import(protocol, console, cx);
                }))
                .on_action(cx.listener(move |console, _: &DataRefreshSection, _, cx| {
                    fetch(protocol, console);
                    section(console, protocol).children.clear();
                    let expanded = expanded_rows(protocol, console);
                    for row in expanded {
                        fetch_children(protocol, console, &row);
                    }
                    cx.notify();
                }))
                .on_action(cx.listener(move |console, _: &DataSortNext, _, cx| {
                    sort_next(protocol, console, cx)
                }))
                .on_action(cx.listener(move |console, _: &DataReverseSort, _, cx| {
                    reverse_sort(protocol, console, cx)
                }))
                .child(header)
                .child(keyboard.flex_1().min_h_0().child(body)),
        )
        // Device files dropped anywhere on the section are imported.
        .when(accept_drop, |this| {
            this.on_drop(cx.listener(move |console, paths: &ExternalPaths, _, cx| {
                import_paths(protocol, console, paths.paths().to_vec(), cx);
            }))
            .drag_over::<ExternalPaths>(move |style, _, _, _| {
                style.border_2().border_color(accent).bg(accent_soft)
            })
        })
        .into_any_element()
}

/// The protocol whose table was drawn last.
#[derive(Default)]
struct Active(Option<&'static Protocol>);

/// What every row of a render shares.
#[derive(Clone, Copy)]
struct Look {
    cursor: Option<usize>,
    focused: bool,
    /// The device name column's width.
    name: f32,
    /// The device columns that fit (a bit each).
    shown: u32,
    /// The table's width, for the child columns' own fit.
    width: f32,
    writable: bool,
}

/// Child rows: the columns that fit beside the indent and remove button.
fn child_fit(child: &Child, look: Look) -> (u32, f32) {
    fit(child.columns, look.width - INDENT, 40.0)
}

fn row_frame(id: ElementId, at: usize, look: Look, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let current = look.cursor == Some(at);
    let hover = theme.hover;
    div()
        .id(id)
        .group("data-device-row")
        .relative()
        .w_full()
        .flex()
        .items_center()
        .gap(SPACE_5)
        .h(px(ROW_HEIGHT))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border.alpha(0.6))
        .when(current, |this| this.bg(theme.selected))
        .when(!current, |this| this.hover(move |style| style.bg(hover)))
        .when(look.focused && current, |this| {
            this.child(focus_ring(theme))
        })
}

fn cell(column: &Column, row: &Value, width: f32, theme: &Theme) -> AnyElement {
    div()
        .w(px(width))
        .flex_none()
        .truncate()
        .when(column.mono, |this| {
            this.font_family(MONO).text_size(FONT_SMALL)
        })
        .text_color(theme.fg_muted)
        .child(cell_text(column.cell, row))
        .into_any_element()
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn draw_row(
    protocol: &'static Protocol,
    console: &mut Console,
    list: &[Value],
    row: Row,
    at: usize,
    look: Look,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let select = move |console: &mut Console, window: &mut Window, cx: &mut Context<Console>| {
        let nav = console.list(protocol.list_id, cx);
        nav.select(at);
        let focus = nav.focus.clone();
        window.focus(&focus);
        cx.notify();
    };
    match row {
        Row::Device(index) => {
            let device = list[index].clone();
            let id = row_id(&device);
            let state = section(console, protocol);
            let expanded = state.expanded.contains(&id);
            let busy = state.busy.contains(&id);
            let link = link_text(protocol, console, &device);
            let current = look.cursor == Some(at);
            let dot = match &link {
                Some((_, true)) => theme.success,
                Some((text, false)) if text == "disabled" => theme.fg_dim,
                Some(_) => theme.danger,
                None => theme.fg_dim,
            };
            let mut cells = Vec::new();
            for (index, column) in protocol.columns.iter().enumerate() {
                if look.shown & (1 << index) == 0 {
                    continue;
                }
                let width = if column.width > 0.0 {
                    column.width
                } else {
                    look.name
                };
                cells.push(match column.cell {
                    Cell::Enabled => div()
                        .w(px(width))
                        .flex_none()
                        .child(pill(
                            if enabled(&device) { "on" } else { "off" },
                            if enabled(&device) {
                                theme.success
                            } else {
                                theme.fg_dim
                            },
                        ))
                        .into_any_element(),
                    Cell::Link => div()
                        .w(px(width))
                        .flex_none()
                        .truncate()
                        .text_size(FONT_SMALL)
                        .text_color(match &link {
                            Some((_, true)) => theme.success,
                            Some((text, false)) if text != "disabled" => theme.danger,
                            _ => theme.fg_dim,
                        })
                        .child(
                            link.as_ref()
                                .map_or_else(|| "no report".to_owned(), |(text, _)| text.clone()),
                        )
                        .into_any_element(),
                    _ if column.width == 0.0 => div()
                        .w(px(width))
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .min_w_0()
                        .child(div().flex_none().size(px(8.0)).rounded(px(4.0)).bg(dot))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.fg)
                                .child(cell_text(column.cell, &device)),
                        )
                        .into_any_element(),
                    _ => cell(column, &device, width, theme),
                });
            }
            let tools = device_tools(protocol, &device);
            let edit_device = device.clone();
            let expand_device = device.clone();
            let delete_device = device.clone();
            row_frame(
                ElementId::Name(format!("data-{}-{id}", protocol.id).into()),
                at,
                look,
                theme,
            )
            .cursor_pointer()
            .on_click(
                cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                    select(console, window, cx);
                    if event.click_count() >= 2 {
                        open_device_form(protocol, console, Some(edit_device.clone()), window, cx);
                    }
                }),
            )
            .child(
                div()
                    .id(ElementId::Name(
                        format!("data-{}-{id}-expand", protocol.id).into(),
                    ))
                    .w(px(16.0))
                    .flex_none()
                    .when(protocol.child.is_some(), |this| {
                        this.cursor_pointer()
                            .child(vt_ui::icon(
                                if expanded {
                                    Icon::ChevronDown
                                } else {
                                    Icon::ChevronRight
                                },
                                ICON_SM,
                                theme.fg_dim,
                            ))
                            .on_click(cx.listener(move |console, _, _, cx| {
                                cx.stop_propagation();
                                toggle_expand(protocol, console, &expand_device, cx);
                            }))
                    }),
            )
            .children(cells)
            .child(
                div()
                    .w(px(ACTIONS))
                    .flex_none()
                    .flex()
                    .justify_end()
                    .gap(SPACE_1)
                    .opacity(if current || busy { 1.0 } else { 0.0 })
                    .group_hover("data-device-row", |style| style.opacity(1.0))
                    .when(busy, |this| {
                        this.child(
                            div()
                                .text_size(FONT_SMALL)
                                .text_color(theme.fg_dim)
                                .child("working…"),
                        )
                    })
                    .when(look.writable && !busy, |this| {
                        this.children(tools.into_iter().map(|(index, tool)| {
                            let device = device.clone();
                            IconButton::new(
                                ElementId::Name(
                                    format!("data-{}-{id}-tool-{index}", protocol.id).into(),
                                ),
                                tool.icon,
                            )
                            .tooltip(tool.label)
                            .on_click(cx.listener(
                                move |console, _, window, cx| {
                                    run_tool(
                                        protocol,
                                        console,
                                        &protocol.tools[index],
                                        Some(device.clone()),
                                        window,
                                        cx,
                                    );
                                },
                            ))
                        }))
                        .child(
                            IconButton::new(
                                ElementId::Name(format!("data-{}-{id}-edit", protocol.id).into()),
                                Icon::Pencil,
                            )
                            .tooltip("Edit  ↵")
                            .on_click(cx.listener(
                                move |console, _, window, cx| {
                                    open_device_form(
                                        protocol,
                                        console,
                                        Some(device.clone()),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .child(
                            IconButton::new(
                                ElementId::Name(format!("data-{}-{id}-delete", protocol.id).into()),
                                Icon::Trash,
                            )
                            .tooltip("Delete  ⌫")
                            .color(theme.danger)
                            .on_click(cx.listener(
                                move |console, _, window, cx| {
                                    ask_delete(
                                        protocol,
                                        console,
                                        Doomed::Device(delete_device.clone()),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                    }),
            )
            .into_any_element()
        }
        Row::Header(index) => {
            let Some(child) = protocol.child else {
                return tail_row(ROW_HEIGHT);
            };
            let id = row_id(&list[index]);
            let (shown, name) = child_fit(child, look);
            row_frame(
                ElementId::Name(format!("data-{}-{id}-header", protocol.id).into()),
                at,
                look,
                theme,
            )
            .bg(theme.surface)
            .text_size(FONT_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.fg_dim)
            .child(div().w(px(INDENT)).flex_none())
            .children(
                child
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| shown & (1 << index) != 0)
                    .map(|(_, column)| {
                        div()
                            .flex_none()
                            .w(px(if column.width > 0.0 {
                                column.width
                            } else {
                                name
                            }))
                            .truncate()
                            .child(column.title.to_uppercase())
                    }),
            )
            .on_click(cx.listener(move |console, _, window, cx| select(console, window, cx)))
            .into_any_element()
        }
        Row::Child(index, child_index) => {
            let Some(child) = protocol.child else {
                return tail_row(ROW_HEIGHT);
            };
            let device = list[index].clone();
            let id = row_id(&device);
            let Some(item) = child_at(protocol, console, &device, child_index) else {
                return tail_row(ROW_HEIGHT);
            };
            let current = look.cursor == Some(at);
            let doomed = (device.clone(), item.clone());
            let (shown, name) = child_fit(child, look);
            row_frame(
                ElementId::Name(format!("data-{}-{id}-child-{child_index}", protocol.id).into()),
                at,
                look,
                theme,
            )
            .bg(theme.surface.alpha(0.5))
            .on_click(cx.listener(move |console, _, window, cx| select(console, window, cx)))
            .child(
                div()
                    .w(px(INDENT))
                    .h_full()
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(div().w(px(1.0)).h_full().bg(theme.border_strong)),
            )
            .children(
                child
                    .columns
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| shown & (1 << index) != 0)
                    .map(|(_, column)| {
                        cell(
                            column,
                            &item,
                            if column.width > 0.0 {
                                column.width
                            } else {
                                name
                            },
                            theme,
                        )
                    }),
            )
            .child(div().flex_1())
            .when(look.writable, |this| {
                this.child(
                    div()
                        .flex_none()
                        .opacity(if current { 1.0 } else { 0.0 })
                        .group_hover("data-device-row", |style| style.opacity(1.0))
                        .child(
                            IconButton::new(
                                ElementId::Name(
                                    format!("data-{}-{id}-child-{child_index}-delete", protocol.id)
                                        .into(),
                                ),
                                Icon::Trash,
                            )
                            .tooltip("Remove  ⌫")
                            .color(theme.danger)
                            .on_click(cx.listener(
                                move |console, _, window, cx| {
                                    let (device, item) = doomed.clone();
                                    ask_delete(
                                        protocol,
                                        console,
                                        Doomed::Child(device, item),
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        ),
                )
            })
            .into_any_element()
        }
        Row::Note(index) => {
            let Some(child) = protocol.child else {
                return tail_row(ROW_HEIGHT);
            };
            let device = list[index].clone();
            let id = row_id(&device);
            let (text, color) = match section(console, protocol).children.get(&id) {
                None => ("Loading…".to_owned(), theme.fg_dim),
                Some(Err(error)) => (
                    format!("{} did not load: {error}", child.title),
                    theme.danger,
                ),
                Some(Ok(children)) if children.is_empty() => (
                    if look.writable {
                        format!("No {}s · ↵ or A adds one", child.noun)
                    } else {
                        format!("No {}s", child.noun)
                    },
                    theme.fg_dim,
                ),
                Some(Ok(_)) => (
                    if look.writable {
                        format!("Add a {}  ↵", child.noun)
                    } else {
                        String::new()
                    },
                    theme.accent,
                ),
            };
            row_frame(
                ElementId::Name(format!("data-{}-{id}-note", protocol.id).into()),
                at,
                look,
                theme,
            )
            .cursor_pointer()
            .on_click(cx.listener(move |console, _, window, cx| {
                select(console, window, cx);
                open_child_form(protocol, console, device.clone(), None, window, cx);
            }))
            .child(div().w(px(INDENT)).flex_none())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(SPACE_3)
                    .text_size(FONT_SMALL)
                    .text_color(color)
                    .when(look.writable, |this| {
                        this.child(vt_ui::icon(Icon::Plus, ICON_SM, color))
                    })
                    .child(text),
            )
            .into_any_element()
        }
    }
}

// ── Device and child forms ───────────────────────────────────────────────

fn subscribe(
    input: &Entity<TextInput>,
    submit: fn(&mut Console, &mut Window, &mut Context<Console>),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> Subscription {
    cx.subscribe_in(
        input,
        window,
        move |console, _, event: &InputEvent, window, cx| match event {
            InputEvent::Submitted => submit(console, window, cx),
            InputEvent::Cancelled => console.close_overlay(window, cx),
            InputEvent::Changed | InputEvent::Blurred => {}
        },
    )
}

/// Text fields for the text-like `fields`, filled from `values`.
pub(crate) fn make_inputs(
    fields: &'static [Field],
    values: &Values,
    submit: fn(&mut Console, &mut Window, &mut Context<Console>),
    window: &mut Window,
    cx: &mut Context<Console>,
) -> (HashMap<&'static str, Entity<TextInput>>, Vec<Subscription>) {
    let mut inputs = HashMap::new();
    let mut subscriptions = Vec::new();
    for field in fields {
        let (kind, mono, secret) = match field.kind {
            Kind::Text { mono, .. } => (InputKind::Text, mono, false),
            Kind::Secret => (InputKind::Text, false, true),
            Kind::Int { min, .. } | Kind::OptionalInt { min, .. } => (
                if min >= 0 {
                    InputKind::Unsigned
                } else {
                    InputKind::Number
                },
                true,
                false,
            ),
            Kind::Float => (InputKind::Number, true, false),
            Kind::Flag | Kind::Choice(_) => continue,
        };
        let text = values.get(field.key).cloned().unwrap_or_default();
        let input = cx.new(|cx| {
            let input = TextInput::new(cx)
                .text(text)
                .placeholder(field.placeholder)
                .kind(kind);
            let input = if mono { input.mono() } else { input };
            if secret { input.secure() } else { input }
        });
        subscriptions.push(subscribe(&input, submit, window, cx));
        inputs.insert(field.key, input);
    }
    (inputs, subscriptions)
}

pub(crate) fn collect(
    inputs: &HashMap<&'static str, Entity<TextInput>>,
    values: &mut Values,
    cx: &App,
) {
    for (key, input) in inputs {
        values.insert(key, input.read(cx).value().to_owned());
    }
}

/// Open the form for a device (`None`: a new one).
pub(crate) fn open_device_form(
    protocol: &'static Protocol,
    console: &mut Console,
    device: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if refuse(protocol, console, cx) {
        return;
    }
    let values = values_of(protocol.fields, device.as_ref());
    open_form(
        protocol,
        console,
        Target::Device(device),
        protocol.fields,
        values,
        None,
        window,
        cx,
    );
}

/// Open the form for a new child of `device`, `prefill` over its defaults.
pub(crate) fn open_child_form(
    protocol: &'static Protocol,
    console: &mut Console,
    device: Value,
    prefill: Option<Values>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(child) = protocol.child else {
        return;
    };
    if refuse(protocol, console, cx) {
        return;
    }
    let mut values = values_of(child.fields, None);
    if let Some(prefill) = prefill {
        values.extend(prefill);
    }
    // The device's children show behind the form.
    let id = row_id(&device);
    let state = section(console, protocol);
    if state.expanded.insert(id.clone()) && !state.children.contains_key(&id) {
        fetch_children(protocol, console, &device);
    }
    open_form(
        protocol,
        console,
        Target::Child(device),
        child.fields,
        values,
        None,
        window,
        cx,
    );
}

#[allow(clippy::too_many_arguments)]
fn open_form(
    protocol: &'static Protocol,
    console: &mut Console,
    target: Target,
    fields: &'static [Field],
    values: Values,
    carry: Option<Vec<Value>>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let (inputs, subscriptions) = make_inputs(fields, &values, save, window, cx);
    let first = fields
        .iter()
        .filter(|field| !(matches!(target, Target::Device(Some(_))) && field.create_only))
        .find_map(|field| inputs.get(field.key).cloned());
    let state = console.state::<Devices>();
    state.window = Some(window.window_handle());
    state.form = Some(Form {
        protocol,
        target,
        fields,
        inputs,
        values,
        error: None,
        saving: false,
        carry,
        note: None,
        _subscriptions: subscriptions,
    });
    let overlay = Overlay::modal(FORM, px(560.0), render_form, cx);
    console.open_overlay(overlay, window, cx);
    if let Some(input) = first {
        input.read(cx).focus(window);
    }
}

/// Copy a device (Modbus): the form filled from it with a new id, and its
/// children fetched to go with it.
fn duplicate(
    protocol: &'static Protocol,
    console: &mut Console,
    device: Value,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if !protocol.duplicate || refuse(protocol, console, cx) {
        return;
    }
    let mut values = values_of(protocol.fields, Some(&device));
    for field in protocol.fields.iter().filter(|field| field.create_only) {
        values.insert(
            field.key,
            format!("{}_copy", value_text(&device, field.key)),
        );
    }
    if let Some(name) = values.get_mut("name") {
        name.push_str(" (copy)");
    }
    open_form(
        protocol,
        console,
        Target::Device(None),
        protocol.fields,
        values,
        Some(Vec::new()),
        window,
        cx,
    );
    if let Some(form) = console.state::<Devices>().form.as_mut() {
        form.note = Some(format!("A copy of {}.", name_of(&device)));
    }
    if let Some(child) = protocol.child {
        let path = device_path(protocol, &device);
        let id = row_id(&device);
        let decode = child.decode;
        let within = match child.api {
            ChildApi::ModbusRegisters => Some("registers"),
            ChildApi::Rest { within, .. } => within,
        };
        console.call(
            CallKey::new(protocol.calls.duplicate),
            Call::new(move |api| async move {
                let mut answer: Value = api.get_json(&path).await?;
                let list = match within {
                    Some(key) => answer.get_mut(key).map(Value::take).unwrap_or_default(),
                    None => answer,
                };
                anyhow::Ok((id, decode(list)))
            }),
        );
    }
}

fn save(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let Some(form) = console.state::<Devices>().form.as_mut() else {
        return;
    };
    if form.saving {
        return;
    }
    collect(&form.inputs, &mut form.values, cx);
    match build_save(form) {
        Ok(call) => {
            form.saving = true;
            form.error = None;
            let key = CallKey::new(form.protocol.calls.save);
            console.call(key, call);
        }
        Err(error) => form.error = Some(error),
    }
    cx.notify();
}

fn build_save(form: &Form) -> Result<Call, String> {
    let protocol = form.protocol;
    match &form.target {
        Target::Device(original) => {
            let editing = original.is_some();
            let mut body = body(form.fields, &form.values, editing)?;
            if let Some(finish) = protocol.finish {
                finish(&mut body, editing);
            }
            if let Some(carry) = &form.carry
                && !carry.is_empty()
            {
                body.insert("registers".to_owned(), carry_registers(carry));
            }
            let label = body
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .or_else(|| body.get("id").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_else(|| protocol.noun.to_owned());
            match original {
                Some(row) => {
                    let path = device_path(protocol, row);
                    Ok(Call::new(move |api| async move {
                        api.send_json::<_, IgnoredAny>(Method::PUT, &path, &body)
                            .await?;
                        anyhow::Ok(Saved::device(format!("Saved {label}")))
                    }))
                }
                None => {
                    let path = protocol.base.to_owned();
                    Ok(Call::new(move |api| async move {
                        let created: CreatedId = api.send_json(Method::POST, &path, &body).await?;
                        anyhow::Ok(Saved {
                            created: Some(created.text()),
                            ..Saved::device(format!("Added {label}"))
                        })
                    }))
                }
            }
        }
        Target::Child(device) => {
            let child = protocol.child.ok_or("no children")?;
            let mut body = body(form.fields, &form.values, false)?;
            if let Some(finish) = child.finish {
                finish(&mut body, false);
            }
            let id = row_id(device);
            let device_path = device_path(protocol, device);
            let label = body
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(child.noun)
                .to_owned();
            let device_name = name_of(device);
            match child.api {
                ChildApi::Rest { segment, .. } => Ok(Call::new(move |api| async move {
                    let _: CreatedId = api
                        .send_json(Method::POST, &format!("{device_path}/{segment}"), &body)
                        .await?;
                    anyhow::Ok(Saved::child(id, format!("Added {label} to {device_name}")))
                })),
                ChildApi::ModbusRegisters => Ok(Call::new(move |api| async move {
                    let mut registers = modbus_registers(&api, &device_path).await?;
                    registers.push(Value::Object(body));
                    replace_registers(&api, &device_path, registers).await?;
                    anyhow::Ok(Saved::child(id, format!("Added {label} to {device_name}")))
                })),
            }
        }
    }
}

/// A copied device's registers as the create body takes them.
fn carry_registers(children: &[Value]) -> Value {
    Value::Array(
        children
            .iter()
            .filter_map(|row| serde_json::from_value::<ModbusRegisterRow>(row.clone()).ok())
            .map(|register| register.input())
            .collect(),
    )
}

/// A Modbus device's registers as `ModbusRegisterInput`s, read now (the
/// server replaces the whole list, so a change starts from what it has).
pub(crate) async fn modbus_registers(
    api: &VoltrusApi,
    device_path: &str,
) -> anyhow::Result<Vec<Value>> {
    let mut answer: Value = api.get_json(device_path).await?;
    let rows: Vec<ModbusRegisterRow> = serde_json::from_value(
        answer
            .get_mut("registers")
            .map(Value::take)
            .unwrap_or_default(),
    )?;
    Ok(rows.iter().map(ModbusRegisterRow::input).collect())
}

pub(crate) async fn replace_registers(
    api: &VoltrusApi,
    device_path: &str,
    registers: Vec<Value>,
) -> anyhow::Result<()> {
    api.send_json::<_, IgnoredAny>(
        Method::PUT,
        &format!("{device_path}/registers"),
        &json!({ "registers": registers }),
    )
    .await?;
    Ok(())
}

fn set_value(console: &mut Console, key: &'static str, value: String, cx: &mut Context<Console>) {
    if let Some(form) = console.state::<Devices>().form.as_mut() {
        form.values.insert(key, value);
    }
    cx.notify();
}

fn set_probe_value(
    console: &mut Console,
    key: &'static str,
    value: String,
    cx: &mut Context<Console>,
) {
    if let Some(probe) = console.state::<Devices>().probe.as_mut() {
        probe.values.insert(key, value);
    }
    cx.notify();
}

pub(crate) type SetValue = fn(&mut Console, &'static str, String, &mut Context<Console>);

/// A flag or a choice: focusable chips; ←→ (or Space on a flag) change it.
fn choice_control(
    field: &'static Field,
    value: &str,
    set: SetValue,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> AnyElement {
    let key = field.key;
    match field.kind {
        Kind::Flag => {
            let on = value == "true";
            div()
                .id(SharedString::from(format!("data-field-{key}")))
                .flex()
                .items_center()
                .gap(SPACE_3)
                .h(px(28.0))
                .px(SPACE_4)
                .rounded(RADIUS_MD)
                .border_1()
                .border_color(theme.border_strong)
                .cursor_pointer()
                .tab_index(0)
                .focus(|style| style.border_color(theme.accent))
                .child(vt_ui::icon(
                    if on { Icon::CircleCheck } else { Icon::Circle },
                    ICON_SM,
                    if on { theme.accent } else { theme.fg_dim },
                ))
                .child(if on { "On" } else { "Off" })
                .on_click(
                    cx.listener(move |console, _, _, cx| set(console, key, (!on).to_string(), cx)),
                )
                .on_key_down(cx.listener(move |console, event: &KeyDownEvent, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "space" | "left" | "right") {
                        set(console, key, (!on).to_string(), cx);
                        cx.stop_propagation();
                    }
                }))
                .into_any_element()
        }
        Kind::Choice(options) => {
            let at = options
                .iter()
                .position(|(option, _)| *option == value)
                .unwrap_or(0);
            div()
                .id(SharedString::from(format!("data-field-{key}")))
                .flex()
                .flex_wrap()
                .gap(SPACE_2)
                .on_key_down(cx.listener(move |console, event: &KeyDownEvent, _, cx| {
                    let next = match event.keystroke.key.as_str() {
                        "left" => at.checked_sub(1).unwrap_or(options.len() - 1),
                        "right" => (at + 1) % options.len(),
                        _ => return,
                    };
                    set(console, key, options[next].0.to_owned(), cx);
                    cx.stop_propagation();
                }))
                .children(options.iter().enumerate().map(|(index, (option, label))| {
                    let chosen = index == at;
                    let hover = theme.hover;
                    div()
                        .id(ElementId::NamedInteger(
                            format!("data-field-{key}").into(),
                            index as u64,
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
                        .child(*label)
                        .on_click(cx.listener(move |console, _, _, cx| {
                            set(console, key, (*option).to_owned(), cx)
                        }))
                }))
                .into_any_element()
        }
        _ => div().into_any_element(),
    }
}

/// The fields in a two-column grid; choices take a whole row.
pub(crate) fn field_grid(
    fields: &'static [Field],
    editing: bool,
    inputs: &HashMap<&'static str, Entity<TextInput>>,
    values: &Values,
    set: SetValue,
    theme: &Theme,
    cx: &mut Context<Console>,
) -> Vec<AnyElement> {
    let mut out = Vec::new();
    let mut pair: Vec<AnyElement> = Vec::new();
    for field in fields {
        if editing && field.create_only {
            continue;
        }
        let label = match field.kind {
            Kind::Int { min, max } if max < i64::from(u32::MAX) => {
                format!("{} ({min}–{max})", field.label)
            }
            Kind::Text {
                required: false, ..
            }
            | Kind::OptionalInt { .. } => format!("{} (optional)", field.label),
            Kind::Secret if editing => format!("{} (blank keeps it)", field.label),
            _ => field.label.to_owned(),
        };
        match field.kind {
            Kind::Choice(_) => {
                if !pair.is_empty() {
                    out.push(grid(std::mem::take(&mut pair)));
                }
                let value = values.get(field.key).cloned().unwrap_or_default();
                out.push(field_el(
                    label,
                    choice_control(field, &value, set, theme, cx),
                    cx,
                ));
            }
            Kind::Flag => {
                let value = values.get(field.key).cloned().unwrap_or_default();
                pair.push(field_el(
                    label,
                    choice_control(field, &value, set, theme, cx),
                    cx,
                ));
            }
            _ => {
                let control = inputs
                    .get(field.key)
                    .cloned()
                    .map_or_else(|| div().into_any_element(), IntoElement::into_any_element);
                pair.push(field_el(label, control, cx));
            }
        }
        if pair.len() == 2 {
            out.push(grid(std::mem::take(&mut pair)));
        }
    }
    if !pair.is_empty() {
        pair.push(div().into_any_element());
        out.push(grid(pair));
    }
    out
}

fn field_el(label: String, control: AnyElement, cx: &Context<Console>) -> AnyElement {
    field(label, control, cx).into_any_element()
}

fn grid(children: Vec<AnyElement>) -> AnyElement {
    div()
        .grid()
        .grid_cols(2)
        .gap(SPACE_6)
        .children(children)
        .into_any_element()
}

pub(crate) fn dialog_header(
    icon: Icon,
    title: String,
    detail: String,
    theme: &Theme,
) -> AnyElement {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_5)
        .h(px(56.0))
        .px(SPACE_8)
        .border_b_1()
        .border_color(theme.border)
        .child(vt_ui::icon(icon, ICON_LG, theme.accent))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .font_family(MONO)
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_dim)
                        .child(detail),
                ),
        )
        .into_any_element()
}

pub(crate) fn error_line(error: &str, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_3)
        .min_w_0()
        .text_size(FONT_SMALL)
        .text_color(theme.danger)
        .child(vt_ui::icon(Icon::CircleAlert, ICON_SM, theme.danger))
        .child(div().min_w_0().truncate().child(error.to_owned()))
        .into_any_element()
}

pub(crate) fn hints(pairs: &[(&'static str, &'static str)], theme: &Theme, cx: &App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(SPACE_2)
        .text_size(FONT_CAPTION)
        .text_color(theme.fg_dim)
        .children(pairs.iter().flat_map(|(key, what)| {
            [
                kbd(*key, cx).into_any_element(),
                div().child(*what).into_any_element(),
            ]
        }))
        .into_any_element()
}

fn render_form(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(form) = console.state::<Devices>().form.as_ref() else {
        return div().into_any_element();
    };
    let protocol = form.protocol;
    let (title, detail, editing) = match &form.target {
        Target::Device(Some(row)) => (
            format!("Edit {}", name_of(row)),
            format!("{} · {}", protocol.noun, row_id(row)),
            true,
        ),
        Target::Device(None) => (
            format!("New {}", protocol.noun),
            protocol.base.to_owned(),
            false,
        ),
        Target::Child(row) => (
            format!(
                "Add a {}",
                protocol.child.map_or("child", |child| child.noun)
            ),
            format!("to {}", name_of(row)),
            false,
        ),
    };
    let fields = form.fields;
    let inputs = form.inputs.clone();
    let values = form.values.clone();
    let (error, saving, note) = (form.error.clone(), form.saving, form.note.clone());
    let body = field_grid(fields, editing, &inputs, &values, set_value, &theme, cx);
    let footer = div()
        .flex()
        .flex_none()
        .items_center()
        .gap(SPACE_4)
        .h(px(56.0))
        .px(SPACE_8)
        .border_t_1()
        .border_color(theme.border)
        .child(div().flex_1().min_w_0().child(match &error {
            Some(error) => error_line(error, &theme),
            None => hints(
                &[("↵", "save"), ("Tab", "next field"), ("Esc", "close")],
                &theme,
                cx,
            ),
        }))
        .child(
            Button::new("data-device-cancel", "Cancel")
                .ghost()
                .on_click(cx.listener(|console, _, window, cx| console.close_overlay(window, cx))),
        )
        .child(
            Button::new(
                "data-device-save",
                match (saving, editing) {
                    (true, _) => "Saving…",
                    (false, true) => "Save",
                    (false, false) => "Add",
                },
            )
            .primary()
            .icon(Icon::Check)
            .disabled(saving)
            .on_click(cx.listener(|console, _, window, cx| save(console, window, cx))),
        );
    div()
        .flex()
        .flex_col()
        .max_h(px(620.0))
        .child(dialog_header(
            if editing { Icon::Pencil } else { Icon::Plus },
            title,
            detail,
            &theme,
        ))
        .child(
            div()
                .id("data-device-form-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .overflow_y_scroll()
                .children(note.map(|note| {
                    div()
                        .text_size(FONT_SMALL)
                        .text_color(theme.fg_muted)
                        .child(note)
                }))
                .children(body),
        )
        .child(footer)
        .into_any_element()
}

// ── Delete ───────────────────────────────────────────────────────────────

fn ask_delete(
    protocol: &'static Protocol,
    console: &mut Console,
    doomed: Doomed,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if refuse(protocol, console, cx) {
        return;
    }
    let focus = cx.focus_handle();
    console.state::<Devices>().confirm = Some(Confirm {
        protocol,
        doomed,
        focus: focus.clone(),
    });
    let overlay = Overlay::modal(CONFIRM, px(440.0), render_confirm, cx);
    console.open_overlay(overlay, window, cx);
    window.focus(&focus);
}

fn delete_confirmed(console: &mut Console, window: &mut Window, cx: &mut Context<Console>) {
    let Some(confirm) = console.state::<Devices>().confirm.take() else {
        return;
    };
    console.close_overlay(window, cx);
    let protocol = confirm.protocol;
    let key = CallKey::new(protocol.calls.delete);
    match confirm.doomed {
        Doomed::Device(row) => {
            let path = device_path(protocol, &row);
            let name = name_of(&row);
            console.call(
                key,
                Call::new(move |api| async move {
                    api.send_empty::<IgnoredAny>(Method::DELETE, &path).await?;
                    anyhow::Ok(Saved::device(format!("Deleted {name}")))
                }),
            );
        }
        Doomed::Child(device, item) => {
            let Some(child) = protocol.child else {
                return;
            };
            let id = row_id(&device);
            let device_path = device_path(protocol, &device);
            let name = value_text(&item, "name");
            match child.api {
                ChildApi::Rest { segment, .. } => {
                    let path = format!("{device_path}/{segment}/{}", encode(&row_id(&item)));
                    console.call(
                        key,
                        Call::new(move |api| async move {
                            api.send_empty::<IgnoredAny>(Method::DELETE, &path).await?;
                            anyhow::Ok(Saved::child(id, format!("Removed {name}")))
                        }),
                    );
                }
                ChildApi::ModbusRegisters => {
                    let gone = value_text(&item, "name");
                    console.call(
                        key,
                        Call::new(move |api| async move {
                            let registers: Vec<Value> = modbus_registers(&api, &device_path)
                                .await?
                                .into_iter()
                                .filter(|register| value_text(register, "name") != gone)
                                .collect();
                            replace_registers(&api, &device_path, registers).await?;
                            anyhow::Ok(Saved::child(id, format!("Removed {name}")))
                        }),
                    );
                }
            }
        }
    }
}

fn render_confirm(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(confirm) = console.state::<Devices>().confirm.as_ref() else {
        return div().into_any_element();
    };
    let protocol = confirm.protocol;
    let (title, body) = match &confirm.doomed {
        Doomed::Device(row) => (
            format!("Delete {}?", name_of(row)),
            match protocol.child {
                Some(child) => format!(
                    "The server stops polling it and deletes its {}{}. This cannot be undone.",
                    child.title.to_lowercase(),
                    if child.makes_tags {
                        " and their data sources"
                    } else {
                        ""
                    }
                ),
                None => "The server stops polling it. This cannot be undone.".to_owned(),
            },
        ),
        Doomed::Child(device, item) => (
            format!("Remove {}?", value_text(item, "name")),
            format!(
                "It leaves {}{}.",
                name_of(device),
                if protocol.child.is_some_and(|child| child.makes_tags) {
                    ", with its data source"
                } else {
                    ""
                }
            ),
        ),
    };
    let focus = confirm.focus.clone();
    let own = focus.clone();
    div()
        .track_focus(&focus)
        // Enter on a focused button is that button's; Enter on the dialog
        // itself deletes.
        .on_key_down(
            cx.listener(move |console, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" && own.is_focused(window) {
                    delete_confirmed(console, window, cx);
                    cx.stop_propagation();
                }
            }),
        )
        .flex()
        .flex_col()
        .gap(SPACE_6)
        .p(SPACE_8)
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(vt_ui::icon(Icon::Trash, ICON_LG, theme.danger))
                .child(
                    div()
                        .text_size(FONT_TITLE)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                ),
        )
        .child(div().text_color(theme.fg_muted).child(body))
        .child(
            div()
                .flex()
                .items_center()
                .gap(SPACE_4)
                .child(
                    div()
                        .flex_1()
                        .child(hints(&[("↵", "delete"), ("Esc", "keep")], &theme, cx)),
                )
                .child(
                    Button::new("data-device-keep", "Keep")
                        .ghost()
                        .on_click(cx.listener(|console, _, window, cx| {
                            console.state::<Devices>().confirm = None;
                            console.close_overlay(window, cx);
                        })),
                )
                .child(
                    Button::new("data-device-delete-confirm", "Delete")
                        .danger()
                        .icon(Icon::Trash)
                        .on_click(cx.listener(|console, _, window, cx| {
                            delete_confirmed(console, window, cx)
                        })),
                ),
        )
        .into_any_element()
}

// ── Probes ───────────────────────────────────────────────────────────────

fn open_probe(
    protocol: &'static Protocol,
    console: &mut Console,
    spec: &'static ProbeSpec,
    device: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let values = values_of(spec.fields, None);
    let (inputs, subscriptions) = make_inputs(spec.fields, &values, run_probe, window, cx);
    let first = spec
        .fields
        .iter()
        .find_map(|field| inputs.get(field.key).cloned());
    let state = console.state::<Devices>();
    state.window = Some(window.window_handle());
    state.probe = Some(Probe {
        protocol,
        spec,
        device,
        inputs,
        values,
        running: false,
        seq: 0,
        answer: None,
        picked: BTreeSet::new(),
        _subscriptions: subscriptions,
    });
    console.list(PROBE_LIST, cx).cursor = None;
    let overlay = Overlay::modal(PROBE, px(620.0), render_probe, cx);
    console.open_overlay(overlay, window, cx);
    match first {
        Some(input) => input.read(cx).focus(window),
        None => {
            let focus = console.list(PROBE_LIST, cx).focus.clone();
            window.focus(&focus);
        }
    }
    if spec.auto {
        run_probe(console, window, cx);
    }
}

fn run_probe(console: &mut Console, _: &mut Window, cx: &mut Context<Console>) {
    let state = console.state::<Devices>();
    state.seq += 1;
    let seq = state.seq;
    let Some(probe) = state.probe.as_mut() else {
        return;
    };
    if probe.running {
        return;
    }
    collect(&probe.inputs, &mut probe.values, cx);
    let spec = probe.spec;
    let (path, body) = match (spec.request)(probe.device.as_ref(), &probe.values) {
        Ok(request) => request,
        Err(error) => {
            probe.answer = Some(Err(error));
            cx.notify();
            return;
        }
    };
    let timeout = (spec.timeout)(&probe.values);
    probe.running = true;
    probe.seq = seq;
    probe.answer = None;
    let key = CallKey::new(probe.protocol.calls.probe).with(seq);
    let decode = spec.decode;
    let values = probe.values.clone();
    console.call(
        key,
        Call::new(move |api| async move {
            let answer = api
                .send_slow(Method::POST, &path, body.as_ref(), timeout)
                .await
                .map_err(|error| error.to_string())
                .and_then(|answer| decode(answer, &values));
            anyhow::Ok(answer)
        }),
    );
    cx.notify();
}

fn pick(
    console: &mut Console,
    row: usize,
    toggle: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(probe) = console.state::<Devices>().probe.as_mut() else {
        return;
    };
    let Some(Ok(answer)) = &probe.answer else {
        return;
    };
    if toggle && probe.spec.multi {
        if !probe.picked.remove(&row) {
            probe.picked.insert(row);
        }
        cx.notify();
        return;
    }
    let payloads: Vec<Value> = if probe.spec.multi && !probe.picked.is_empty() {
        probe
            .picked
            .iter()
            .filter_map(|at| answer.rows.get(*at))
            .map(|row| row.payload.clone())
            .collect()
    } else {
        answer
            .rows
            .get(row)
            .map(|row| vec![row.payload.clone()])
            .unwrap_or_default()
    };
    if payloads.is_empty() {
        return;
    }
    let (protocol, spec, device) = (probe.protocol, probe.spec, probe.device.clone());
    (spec.on_pick)(console, protocol, device, payloads, window, cx);
}

fn render_probe(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(probe) = console.state::<Devices>().probe.as_ref() else {
        return div().into_any_element();
    };
    let spec = probe.spec;
    let intro = (spec.intro)(probe.device.as_ref());
    let inputs = probe.inputs.clone();
    let values = probe.values.clone();
    let running = probe.running;
    let answer = probe.answer.clone();
    let picked = probe.picked.clone();
    let fields = field_grid(
        spec.fields,
        false,
        &inputs,
        &values,
        set_probe_value,
        &theme,
        cx,
    );
    let rows: Rc<Vec<ProbeRow>> = Rc::new(match &answer {
        Some(Ok(answer)) => answer.rows.clone(),
        _ => Vec::new(),
    });
    let list = keyboard_list(
        ListKeys {
            id: PROBE_LIST,
            on_move: None,
            on_activate: pick,
        },
        rows.len(),
        8,
        console,
        cx,
    );
    let nav = console.list(PROBE_LIST, cx);
    let (cursor, focused, scroll) = (nav.cursor, nav.focus.is_focused(window), nav.scroll.clone());
    // The keyboard list is always in the tree, so a probe without fields
    // keeps focus (and Esc) while it runs.
    let results: AnyElement = match &answer {
        None if running => list
            .p(SPACE_6)
            .text_color(theme.fg_muted)
            .child("Asking the server… this waits for the devices to answer.")
            .into_any_element(),
        None => list.into_any_element(),
        Some(Err(error)) => list
            .p(SPACE_6)
            .child(error_line(error, &theme))
            .into_any_element(),
        Some(Ok(answer)) if answer.rows.is_empty() => list
            .p(SPACE_6)
            .flex()
            .flex_col()
            .gap(SPACE_2)
            .text_color(theme.fg_muted)
            .children(answer.note.clone())
            .child(spec.empty)
            .into_any_element(),
        Some(Ok(answer)) => {
            let count = answer.rows.len();
            let multi = spec.multi;
            let picked = picked.clone();
            div()
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .children(answer.note.clone().map(|note| div().text_size(FONT_SMALL).text_color(theme.fg_muted).child(note)))
                .child(
                    list.h(px(ROW_HEIGHT * count.clamp(1, 8) as f32 + 2.0))
                        .rounded(RADIUS_MD)
                        .border_1()
                        .border_color(if focused { theme.accent } else { theme.border_strong })
                        .overflow_hidden()
                        .child(
                            uniform_list(
                                "data-device-probe-rows",
                                count,
                                cx.processor(move |_: &mut Console, range: Range<usize>, _, cx: &mut Context<Console>| {
                                    let theme = vt_ui::theme::theme(cx).clone();
                                    range
                                        .filter_map(|at| {
                                            let item = rows.get(at)?;
                                            let chosen = picked.contains(&at);
                                            let hover = theme.hover;
                                            Some(
                                                div()
                                                    .id(("data-device-probe-row", at))
                                                    .relative()
                                                    .flex()
                                                    .items_center()
                                                    .gap(SPACE_4)
                                                    .h(px(ROW_HEIGHT))
                                                    .px(SPACE_5)
                                                    .cursor_pointer()
                                                    .when(cursor == Some(at), |this| this.bg(theme.selected))
                                                    .hover(move |style| style.bg(hover))
                                                    .when(focused && cursor == Some(at), |this| this.child(focus_ring(&theme)))
                                                    .on_click(cx.listener(move |console, event: &gpui::ClickEvent, window, cx| {
                                                        let nav = console.list(PROBE_LIST, cx);
                                                        nav.select(at);
                                                        let focus = nav.focus.clone();
                                                        window.focus(&focus);
                                                        pick(console, at, multi && event.click_count() < 2, window, cx);
                                                    }))
                                                    .when(multi, |this| {
                                                        this.child(vt_ui::icon(
                                                            if chosen { Icon::CircleCheck } else { Icon::Circle },
                                                            ICON_SM,
                                                            if chosen { theme.accent } else { theme.fg_dim },
                                                        ))
                                                    })
                                                    .child(div().flex_1().min_w_0().truncate().font_family(MONO).text_size(FONT_SMALL).child(item.label.clone()))
                                                    .child(div().flex_none().max_w(px(280.0)).truncate().text_size(FONT_SMALL).text_color(theme.fg_dim).child(item.detail.clone())),
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                }),
                            )
                            .track_scroll(scroll)
                            .size_full(),
                        ),
                )
                .into_any_element()
        }
    };
    let have_rows = matches!(&answer, Some(Ok(answer)) if !answer.rows.is_empty());
    let pick_label: SharedString = if spec.multi && !picked.is_empty() {
        format!("{} {} selected", spec.pick_label, picked.len()).into()
    } else {
        spec.pick_label.into()
    };
    let hint_pairs: &[(&str, &str)] = if spec.multi {
        &[("↵", "pick"), ("Space", "select"), ("Esc", "close")]
    } else {
        &[("↵", "pick"), ("Esc", "close")]
    };
    div()
        .flex()
        .flex_col()
        .max_h(px(640.0))
        .child(dialog_header(
            Icon::Search,
            spec.title.to_owned(),
            intro,
            &theme,
        ))
        .child(
            div()
                .id("data-device-probe-body")
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap(SPACE_6)
                .px(SPACE_8)
                .py(SPACE_7)
                .overflow_y_scroll()
                .children(fields)
                .child(results),
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
                .child(div().flex_1().child(hints(hint_pairs, &theme, cx)))
                .when(!spec.fields.is_empty() || !spec.auto, |this| {
                    this.child(
                        Button::new(
                            "data-device-probe-run",
                            if running {
                                "Working…"
                            } else {
                                spec.run_label
                            },
                        )
                        .icon(Icon::RefreshCw)
                        .disabled(running)
                        .on_click(
                            cx.listener(|console, _, window, cx| run_probe(console, window, cx)),
                        ),
                    )
                })
                .when(have_rows, |this| {
                    this.child(
                        Button::new("data-device-probe-pick", pick_label)
                            .primary()
                            .icon(Icon::Plus)
                            .on_click(cx.listener(|console, _, window, cx| {
                                let row = console.list(PROBE_LIST, cx).cursor.unwrap_or(0);
                                pick(console, row, false, window, cx);
                            })),
                    )
                }),
        )
        .into_any_element()
}

/// A probe's pick: create one device per payload (each a create body),
/// then close the probe; the report lists what was added.
pub(crate) fn pick_devices(
    console: &mut Console,
    protocol: &'static Protocol,
    _: Option<Value>,
    bodies: Vec<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    if refuse(protocol, console, cx) {
        return;
    }
    let path = protocol.base.to_owned();
    console.close_overlay(window, cx);
    console.state::<Devices>().probe = None;
    section(console, protocol).importing = true;
    console.call(
        CallKey::new(protocol.calls.import),
        Call::new(move |api| async move {
            let mut lines = Vec::new();
            for body in bodies {
                let label = value_text(&body, "name");
                match api
                    .send_json::<_, CreatedId>(Method::POST, &path, &body)
                    .await
                {
                    Ok(_) => lines.push(format!("added {label}")),
                    Err(error) => lines.push(format!("{label} was not added: {error}")),
                }
            }
            anyhow::Ok(lines)
        }),
    );
}

/// A probe's pick: the child form, filled from the payload's fields.
pub(crate) fn pick_child(
    console: &mut Console,
    protocol: &'static Protocol,
    device: Option<Value>,
    payloads: Vec<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let (Some(device), Some(child), Some(payload)) = (device, protocol.child, payloads.first())
    else {
        return;
    };
    console.state::<Devices>().probe = None;
    let values = prefill(child.fields, payload);
    open_child_form(protocol, console, device, Some(values), window, cx);
}

/// The values `payload` has for `fields`.
pub(crate) fn prefill(fields: &[Field], payload: &Value) -> Values {
    fields
        .iter()
        .filter(|field| payload.get(field.key).is_some_and(|value| !value.is_null()))
        .map(|field| (field.key, value_text(payload, field.key)))
        .collect()
}

// ── The actions menu ─────────────────────────────────────────────────────

fn open_menu(
    protocol: &'static Protocol,
    console: &mut Console,
    row: Option<Value>,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let writable = may_write(protocol, console);
    let mut items: Vec<(MenuItem, SharedString, Icon)> = Vec::new();
    if let Some(device) = &row
        && writable
    {
        items.push((
            MenuItem::Edit,
            format!("Edit {}", name_of(device)).into(),
            Icon::Pencil,
        ));
        if let Some(child) = protocol.child {
            items.push((
                MenuItem::AddChild,
                format!("Add a {}", child.noun).into(),
                Icon::Plus,
            ));
        }
        items.push((
            MenuItem::Toggle,
            (if enabled(device) { "Disable" } else { "Enable" }).into(),
            Icon::Square,
        ));
        if protocol.duplicate {
            items.push((MenuItem::Duplicate, "Duplicate".into(), Icon::Copy));
        }
        for (index, tool) in device_tools(protocol, device) {
            items.push((MenuItem::Tool(index), tool.label.into(), tool.icon));
        }
    }
    if writable {
        for (index, tool) in protocol.tools.iter().enumerate() {
            if tool.scope == Scope::Section {
                items.push((MenuItem::Tool(index), tool.label.into(), tool.icon));
            }
        }
        if protocol.import.is_some() {
            items.push((
                MenuItem::Import,
                "Import device files…".into(),
                Icon::Upload,
            ));
        }
    }
    items.push((MenuItem::Refresh, "Refresh".into(), Icon::RefreshCw));
    if row.is_some() && writable {
        items.push((MenuItem::Delete, "Delete".into(), Icon::Trash));
    }
    console.state::<Devices>().menu = Some(Menu {
        protocol,
        row,
        items,
    });
    console.list(MENU_LIST, cx).cursor = Some(0);
    let overlay = Overlay::modal(MENU, px(340.0), render_menu, cx);
    console.open_overlay(overlay, window, cx);
    let focus = console.list(MENU_LIST, cx).focus.clone();
    window.focus(&focus);
}

fn run_menu(
    console: &mut Console,
    at: usize,
    _: bool,
    window: &mut Window,
    cx: &mut Context<Console>,
) {
    let Some(menu) = console.state::<Devices>().menu.take() else {
        return;
    };
    let Some(item) = menu.items.get(at).map(|(item, _, _)| *item) else {
        return;
    };
    let protocol = menu.protocol;
    console.close_overlay(window, cx);
    let row = menu.row;
    match item {
        MenuItem::Edit => open_device_form(protocol, console, row, window, cx),
        MenuItem::AddChild => {
            if let Some(row) = row {
                open_child_form(protocol, console, row, None, window, cx);
            }
        }
        MenuItem::Toggle => {
            if let Some(row) = row {
                toggle_enabled(protocol, console, &row, cx);
            }
        }
        MenuItem::Duplicate => {
            if let Some(row) = row {
                duplicate(protocol, console, row, window, cx);
            }
        }
        MenuItem::Tool(index) => {
            let tool = &protocol.tools[index];
            let device = (tool.scope == Scope::Device).then_some(row).flatten();
            run_tool(protocol, console, tool, device, window, cx);
        }
        MenuItem::Import => prompt_import(protocol, console, cx),
        MenuItem::Refresh => {
            fetch(protocol, console);
            cx.notify();
        }
        MenuItem::Delete => {
            if let Some(row) = row {
                ask_delete(protocol, console, Doomed::Device(row), window, cx);
            }
        }
    }
}

fn render_menu(
    console: &mut Console,
    window: &mut Window,
    cx: &mut Context<Console>,
) -> AnyElement {
    let theme = theme(cx).clone();
    let Some(menu) = console.state::<Devices>().menu.as_ref() else {
        return div().into_any_element();
    };
    let title = menu
        .row
        .as_ref()
        .map_or_else(|| menu.protocol.title.to_owned(), name_of);
    let items = menu.items.clone();
    let list = keyboard_list(
        ListKeys {
            id: MENU_LIST,
            on_move: None,
            on_activate: run_menu,
        },
        items.len(),
        items.len(),
        console,
        cx,
    );
    let nav = console.list(MENU_LIST, cx);
    let (cursor, focused) = (nav.cursor, nav.focus.is_focused(window));
    list.flex()
        .flex_col()
        .p(SPACE_3)
        .child(
            div()
                .px(SPACE_4)
                .py(SPACE_3)
                .text_size(FONT_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.fg_dim)
                .truncate()
                .child(title.to_uppercase()),
        )
        .children(
            items
                .into_iter()
                .enumerate()
                .map(|(at, (item, label, icon))| {
                    let hover = theme.hover;
                    let danger = matches!(item, MenuItem::Delete);
                    div()
                        .id(("data-device-menu-item", at))
                        .relative()
                        .flex()
                        .items_center()
                        .gap(SPACE_4)
                        .h(px(30.0))
                        .px(SPACE_4)
                        .rounded(RADIUS_MD)
                        .cursor_pointer()
                        .when(cursor == Some(at), |this| this.bg(theme.selected))
                        .hover(move |style| style.bg(hover))
                        .when(focused && cursor == Some(at), |this| {
                            this.child(focus_ring(&theme))
                        })
                        .text_color(if danger { theme.danger } else { theme.fg })
                        .child(vt_ui::icon(
                            icon,
                            ICON_MD,
                            if danger { theme.danger } else { theme.fg_dim },
                        ))
                        .child(label)
                        .on_click(cx.listener(move |console, _, window, cx| {
                            run_menu(console, at, false, window, cx)
                        }))
                }),
        )
        .into_any_element()
}

// ── Import ───────────────────────────────────────────────────────────────

/// Ask for device files, then import them.
pub(crate) fn prompt_import(
    protocol: &'static Protocol,
    console: &mut Console,
    cx: &mut Context<Console>,
) {
    if protocol.import.is_none() || refuse(protocol, console, cx) {
        return;
    }
    let prompt = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: Some("Import".into()),
    });
    cx.spawn(async move |this, cx| {
        if let Ok(Ok(Some(paths))) = prompt.await {
            let _ = this.update(cx, |console, cx| import_paths(protocol, console, paths, cx));
        }
    })
    .detach();
}

/// Import device files (dropped or picked): every device each holds is
/// created on the server, children included; the report lists what was
/// added and what was not.
pub(crate) fn import_paths(
    protocol: &'static Protocol,
    console: &mut Console,
    paths: Vec<PathBuf>,
    cx: &mut Context<Console>,
) {
    let Some(kind) = protocol.import else {
        return;
    };
    if paths.is_empty() || refuse(protocol, console, cx) {
        return;
    }
    let state = section(console, protocol);
    state.importing = true;
    state.notice = vec![format!("Importing {} file(s)…", paths.len())];
    let base = protocol.base.to_owned();
    console.call(
        CallKey::new(protocol.calls.import),
        Call::new(move |api| async move {
            let mut lines = Vec::new();
            for path in paths {
                let file = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) => {
                        lines.push(format!("{file}: not read: {error}"));
                        continue;
                    }
                };
                for step in import_plan(kind, &file, &text) {
                    match step {
                        ImportStep::Note(line) => lines.push(line),
                        ImportStep::Device {
                            label,
                            body,
                            children,
                        } => {
                            match api
                                .send_json::<_, CreatedId>(Method::POST, &base, &body)
                                .await
                            {
                                Ok(_) if children.is_empty() => {
                                    lines.push(format!("{file}: added {label}"));
                                }
                                Ok(created) => {
                                    let total = children.len();
                                    let mut added = 0;
                                    let path =
                                        format!("{base}/{}/objects", encode(&created.text()));
                                    for object in children {
                                        let name = value_text(&object, "name");
                                        match api
                                            .send_json::<_, CreatedId>(Method::POST, &path, &object)
                                            .await
                                        {
                                            Ok(_) => added += 1,
                                            Err(error) => lines.push(format!(
                                                "{file}: {label}: {name} was not added: {error}"
                                            )),
                                        }
                                    }
                                    lines.push(format!(
                                        "{file}: added {label} with {added} of {total} objects"
                                    ));
                                }
                                Err(error) => {
                                    lines.push(format!("{file}: {label} was not added: {error}"))
                                }
                            }
                        }
                    }
                }
            }
            anyhow::Ok(lines)
        }),
    );
    cx.notify();
}

/// What importing one file does, in order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportStep {
    /// A line for the report (a rejected device or register).
    Note(String),
    /// Create this device, then each child (BACnet objects) under it.
    Device {
        label: String,
        body: Value,
        children: Vec<Value>,
    },
}

/// The steps for importing `text` (from `file`) into a `kind` section,
/// with the web client's messages.
pub(crate) fn import_plan(kind: DeviceFileKind, file: &str, text: &str) -> Vec<ImportStep> {
    let other = match kind {
        DeviceFileKind::Modbus => DeviceFileKind::Bacnet,
        DeviceFileKind::Bacnet => DeviceFileKind::Modbus,
    };
    if device_file_kind(text) == Some(other) {
        return vec![ImportStep::Note(format!(
            "{file}: this is a {} device file, import it on the {} devices page",
            other.label(),
            other.label()
        ))];
    }
    let mut steps = Vec::new();
    match kind {
        DeviceFileKind::Modbus => {
            let parsed = parse_modbus_device_file(text);
            steps.extend(
                parsed
                    .errors
                    .into_iter()
                    .map(|error| ImportStep::Note(format!("{file}: {error}"))),
            );
            for device in parsed.devices {
                let label = format!("{} with {} registers", device.name, device.registers.len());
                steps.push(ImportStep::Device {
                    label,
                    body: serde_json::to_value(&device).unwrap_or_default(),
                    children: Vec::new(),
                });
            }
        }
        DeviceFileKind::Bacnet => {
            let parsed = parse_bacnet_device_file(text);
            steps.extend(
                parsed
                    .errors
                    .into_iter()
                    .map(|error| ImportStep::Note(format!("{file}: {error}"))),
            );
            for device in parsed.devices {
                let children = device
                    .objects
                    .iter()
                    .map(|object| serde_json::to_value(object).unwrap_or_default())
                    .collect();
                steps.push(ImportStep::Device {
                    label: device.name.clone(),
                    body: json!({
                        "name": device.name,
                        "ip": device.ip,
                        "port": device.port,
                        "device_instance": device.device_instance,
                        "interval_ms": device.interval_ms,
                    }),
                    children,
                });
            }
        }
    }
    steps
}

/// The sections whose poll runs now.
#[derive(Default)]
struct Polls(HashSet<&'static str>);

/// Run `tick` every `every` while section `id` is on screen in a
/// signed-in console (not in a stress test); call it from the section's
/// `on_open`. The loop ends when the section goes off screen.
pub(crate) fn poll_section(
    console: &mut Console,
    id: &'static str,
    every: Duration,
    tick: fn(&mut Console, &mut Context<Console>),
    cx: &mut Context<Console>,
) {
    if !console.state::<Polls>().0.insert(id) {
        return;
    }
    cx.spawn(async move |this, cx| {
        loop {
            cx.background_executor().timer(every).await;
            let shown = this.update(cx, |console, cx| {
                let shown = super::section_shown(console, id)
                    && console.is_signed_in()
                    && console.stress.is_none();
                if shown {
                    tick(console, cx);
                } else {
                    console.state::<Polls>().0.remove(id);
                }
                shown
            });
            if !matches!(shown, Ok(true)) {
                break;
            }
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: &[(&str, &str)] = &[("big", "big"), ("little", "little")];
    const FIELDS: &[Field] = &[
        Field {
            create_only: true,
            ..mono("id", "Device ID", "plts_1")
        },
        optional("name", "Name", "PLTS 1"),
        int("port", "Port", "502", 1, 65535),
        Field {
            key: "event",
            label: "Event scan (ms)",
            placeholder: "",
            kind: Kind::OptionalInt {
                min: 0,
                max: 3_600_000,
            },
            default: "",
            create_only: false,
        },
        Field {
            key: "scale",
            label: "Scale",
            placeholder: "1",
            kind: Kind::Float,
            default: "1",
            create_only: false,
        },
        flag("enabled", "Enabled", "true"),
        choice("endianness", "Endianness", KINDS, "big"),
        Field {
            key: "password",
            label: "Password",
            placeholder: "",
            kind: Kind::Secret,
            default: "",
            create_only: false,
        },
    ];

    fn values(pairs: &[(&'static str, &str)]) -> Values {
        let mut values = values_of(FIELDS, None);
        for (key, value) in pairs {
            values.insert(key, (*value).to_owned());
        }
        values
    }

    #[test]
    fn a_new_form_takes_the_defaults_and_builds_the_servers_body() {
        let body = body(FIELDS, &values(&[("id", " plts_1 ")]), false).unwrap();
        assert_eq!(
            Value::Object(body),
            json!({"id": "plts_1", "name": "", "port": 502, "event": null, "scale": 1.0,
                "enabled": true, "endianness": "big"}),
            "blank numbers take their defaults, a blank optional number is null, a blank password is left out"
        );
    }

    #[test]
    fn the_form_refuses_what_the_server_would() {
        assert_eq!(
            body(FIELDS, &values(&[]), false).unwrap_err(),
            "Device ID is required"
        );
        assert_eq!(
            body(FIELDS, &values(&[("id", "a"), ("port", "70000")]), false).unwrap_err(),
            "Port must be a whole number from 1 to 65535"
        );
        assert_eq!(
            body(FIELDS, &values(&[("id", "a"), ("scale", "x")]), false).unwrap_err(),
            "Scale must be a number"
        );
        assert_eq!(
            body(
                FIELDS,
                &values(&[("id", "a"), ("endianness", "middle")]),
                false
            )
            .unwrap_err(),
            "Choose a endianness"
        );
    }

    #[test]
    fn editing_leaves_out_the_id_and_fills_from_the_row() {
        let row = json!({"id": "plts_1", "name": "PLTS", "port": 1502, "event": null,
            "enabled": false, "endianness": "swapped", "password": "secret"});
        let filled = values_of(FIELDS, Some(&row));
        assert_eq!(filled["port"], "1502");
        assert_eq!(filled["enabled"], "false");
        assert_eq!(filled["endianness"], "big", "an unknown choice falls back");
        assert_eq!(filled["password"], "", "a password is never shown");
        assert_eq!(filled["event"], "", "null stays blank");
        let body = body(FIELDS, &filled, true).unwrap();
        assert!(!body.contains_key("id"));
        assert!(!body.contains_key("password"));
        assert_eq!(body["enabled"], false);
        let mut with_secret = filled.clone();
        with_secret.insert("password", "new".into());
        assert_eq!(body_of(&with_secret)["password"], "new");
    }

    fn body_of(values: &Values) -> Map<String, Value> {
        body(FIELDS, values, true).unwrap()
    }

    #[test]
    fn search_and_sort_work_on_the_shown_text() {
        let list = vec![
            json!({"id": 1, "name": "Pump B", "ip": "10.0.0.10", "interval_ms": 500}),
            json!({"id": 2, "name": "pump a", "ip": "10.0.0.9", "interval_ms": 5000}),
            json!({"id": 3, "name": "Meter", "ip": "192.168.1.2", "interval_ms": 1000}),
        ];
        assert_eq!(visible_rows(&list, &["name", "ip"], "PUMP", None), [0, 1]);
        assert_eq!(visible_rows(&list, &["name", "ip"], "192.", None), [2]);
        assert_eq!(
            visible_rows(&list, &["name"], "", Some((Cell::Key("name"), false))),
            [2, 1, 0]
        );
        assert_eq!(
            visible_rows(&list, &["name"], "", Some((Cell::Ms("interval_ms"), true))),
            [1, 2, 0],
            "numbers sort by value"
        );
        assert_eq!(cell_text(Cell::Ms("interval_ms"), &list[0]), "500ms");
        assert_eq!(cell_text(Cell::Key("unit"), &list[0]), "—");
        assert_eq!(cell_text(Cell::OnOff("cov"), &json!({"cov": true})), "On");
        assert_eq!(
            cell_text(Cell::Enabled, &json!({})),
            "on",
            "enabled unless told"
        );
    }

    #[test]
    fn expanded_devices_show_their_children_then_an_add_line() {
        let rows = lines(
            &[2, 0, 1],
            |device| device != 1,
            |device| match device {
                2 => Some(2),
                0 => Some(0),
                _ => None,
            },
        );
        assert_eq!(
            rows,
            [
                Row::Device(2),
                Row::Header(2),
                Row::Child(2, 0),
                Row::Child(2, 1),
                Row::Note(2),
                Row::Device(0),
                Row::Note(0),
                Row::Device(1),
            ]
        );
        let loading = lines(&[0], |_| true, |_| None);
        assert_eq!(loading, [Row::Device(0), Row::Note(0)]);
    }

    #[test]
    fn ids_go_into_paths_escaped() {
        let protocol = &super::super::modbus::PROTOCOL;
        assert_eq!(
            device_path(protocol, &json!({"id": "plts 1/a"})),
            "/api/v1/modbus-devices/plts%201%2Fa"
        );
        assert_eq!(name_of(&json!({"id": 4, "name": " "})), "4");
    }

    #[test]
    fn imports_create_what_the_file_holds_and_report_the_rest() {
        let modbus = r#"{"id": "m1", "ip": "10.0.0.1", "registers": [{"name": "v", "offset": 3}, {"name": "w"}]}"#;
        let steps = import_plan(DeviceFileKind::Modbus, "plant.json", modbus);
        assert_eq!(
            steps[0],
            // A nameless device is labelled by its place, as the web does.
            ImportStep::Note("plant.json: device 1: w has no address or offset".into())
        );
        let ImportStep::Device {
            label,
            body,
            children,
        } = &steps[1]
        else {
            panic!("a device step");
        };
        assert_eq!(label, "m1 with 1 registers");
        assert_eq!(
            body["registers"][0],
            json!({"name": "v", "offset": 3, "register_type": "float32", "register_area": "holding", "scale": 1.0})
        );
        assert!(children.is_empty());

        let bacnet = r#"{"name": "AHU", "ip": "10.0.0.2", "objects": [{"name": "T", "object_type": "analogInput", "object_instance": 1}]}"#;
        let steps = import_plan(DeviceFileKind::Bacnet, "ahu.json", bacnet);
        let ImportStep::Device { body, children, .. } = &steps[0] else {
            panic!("a device step");
        };
        assert_eq!(
            body,
            &json!({"name": "AHU", "ip": "10.0.0.2", "port": 47808, "device_instance": 0, "interval_ms": 1000})
        );
        assert_eq!(
            children[0],
            json!({"object_type": "analogInput", "object_instance": 1, "property": "presentValue", "name": "T"})
        );

        assert_eq!(
            import_plan(DeviceFileKind::Modbus, "ahu.json", bacnet),
            [ImportStep::Note(
                "ahu.json: this is a BACnet device file, import it on the BACnet devices page"
                    .into()
            )]
        );
        assert_eq!(
            import_plan(DeviceFileKind::Bacnet, "plant.json", modbus),
            [ImportStep::Note(
                "plant.json: this is a Modbus device file, import it on the Modbus devices page"
                    .into()
            )]
        );
    }
}
