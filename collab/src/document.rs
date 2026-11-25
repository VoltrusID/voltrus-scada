//! The screen document: a screen's 2D rows as a Yjs document, and the way
//! back. docs/collaboration.md describes the layout; the web Designer's
//! `client/src/designer/collab/ydoc.ts` reads and writes the same shape.
//!
//! ```text
//! widgets:      Y.Map  id -> Y.Map { type, name, x, y, width, height,
//!                                     sensors: [tag ids], zi: "V",
//!                                     config: Y.Map { key -> value,
//!                                                     bindings: Y.Map { prop -> binding } } }
//! connections:  Y.Map  id -> Y.Map { from, to, type, waypoints: [[x, y], ...] }
//! ```
//!
//! * `config` is a nested map so two people changing different properties of
//!   one widget merge; `config.bindings` is nested again so two people binding
//!   different properties merge too (one binding is replaced whole).
//! * Stacking order is defined once, by the fractional index `zi`: widgets
//!   sorted by `(zi, id)`. `config.z` is not in the document; rows get it on
//!   the way out ([`Screen::to_rows`]) and lose it on the way in.
//! * The rotation column and `config.rotation` are one fact: a row whose
//!   config has no rotation gets the column's (as the web's `loadLayoutFromAPI`
//!   does), and the column is written from `config.rotation`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value};
use yrs::types::ToJson;
use yrs::{Any, In, Map, MapPrelim, MapRef, Out, ReadTxn, TransactionMut, WriteTxn};

use crate::fractional;

/// Root map of the widgets.
pub const WIDGETS: &str = "widgets";
/// Root map of the connections.
pub const CONNECTIONS: &str = "connections";
/// The config key of property bindings.
pub const BINDINGS: &str = "bindings";

/// One `scada_equipment` row of a screen, without the screen id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetRow {
    pub id: String,
    /// `equipment_type`.
    pub kind: String,
    /// `equipment_name`.
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    /// `sensor_bindings`, JSON text.
    pub sensors: String,
    /// JSON text.
    pub config: String,
}

/// One `scada_connections` row of a screen, without the screen id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConnectionRow {
    pub id: String,
    pub from: String,
    pub to: String,
    /// `connection_type`.
    pub kind: String,
    /// JSON text.
    pub waypoints: String,
}

/// A screen's rows.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rows {
    pub widgets: Vec<WidgetRow>,
    pub connections: Vec<ConnectionRow>,
}

/// A widget as the document holds it (no id, no stacking index).
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub kind: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub sensors: Value,
    /// Without `z`; `bindings` inside as an object.
    pub config: JsonMap<String, Value>,
}

/// A connection as the document holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    pub from: String,
    pub to: String,
    pub kind: String,
    pub waypoints: Value,
}

/// The kind of a design object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Widget,
    Connection,
}

impl ObjectKind {
    /// The change log's `object_type`.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectKind::Widget => "widget",
            ObjectKind::Connection => "connection",
        }
    }
}

/// The whole document, read out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Screen {
    /// id -> (widget, zi).
    pub widgets: BTreeMap<String, (Widget, String)>,
    pub connections: BTreeMap<String, Connection>,
}

// ── JSON and Yjs values ──

/// The largest integer a JavaScript number holds exactly.
const SAFE_INT: f64 = 9_007_199_254_740_991.0;

/// A JSON value as a Yjs value. Integers stay integers.
pub fn json_to_any(v: &Value) -> Any {
    match v {
        Value::Null => Any::Null,
        Value::Bool(b) => Any::Bool(*b),
        Value::Number(n) => match n.as_i64() {
            Some(i) if (i as f64).abs() <= SAFE_INT => Any::from(i),
            _ => Any::from(n.as_f64().unwrap_or(0.0)),
        },
        Value::String(s) => Any::from(s.as_str()),
        Value::Array(a) => Any::Array(a.iter().map(json_to_any).collect()),
        Value::Object(m) => Any::from(
            m.iter()
                .map(|(k, v)| (k.clone(), json_to_any(v)))
                .collect::<std::collections::HashMap<_, _>>(),
        ),
    }
}

/// A Yjs value as JSON. A whole float is written as an integer, as
/// JavaScript's `JSON.stringify` writes it; `undefined`, buffers and
/// non-finite numbers become `null`.
pub fn any_to_json(a: &Any) -> Value {
    match a {
        Any::Null | Any::Undefined | Any::Buffer(_) => Value::Null,
        Any::Bool(b) => Value::Bool(*b),
        Any::Number(n) => number_json(n.as_f64().unwrap_or(f64::NAN)),
        Any::String(s) => Value::String(s.to_string()),
        Any::Array(a) => Value::Array(a.iter().map(any_to_json).collect()),
        Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), any_to_json(v))).collect()),
    }
}

fn number_json(f: f64) -> Value {
    if !f.is_finite() {
        return Value::Null;
    }
    if f.fract() == 0.0 && f.abs() <= SAFE_INT {
        return Value::from(f as i64);
    }
    serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
}

/// The same value with every number written as [`any_to_json`] writes it,
/// so `3` and `3.0` compare equal.
pub fn normalize(v: &Value) -> Value {
    match v {
        Value::Number(n) => number_json(n.as_f64().unwrap_or(f64::NAN)),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), normalize(v))).collect())
        }
        other => other.clone(),
    }
}

fn out_to_json<T: ReadTxn>(txn: &T, o: &Out) -> Value {
    match o {
        Out::Any(a) => any_to_json(a),
        other => any_to_json(&other.to_json(txn)),
    }
}

fn number_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

// ── Rows and document objects ──

impl Widget {
    /// The widget of a row, and the row's `config.z` when it has one.
    pub fn from_row(row: &WidgetRow) -> (Widget, Option<f64>) {
        let mut config = match serde_json::from_str::<Value>(&row.config) {
            Ok(Value::Object(m)) => m,
            _ => JsonMap::new(),
        };
        let z = config.remove("z").as_ref().and_then(number_of);
        let has_rotation = config.get("rotation").is_some_and(|v| !v.is_null());
        if !has_rotation && row.rotation != 0.0 {
            config.insert("rotation".into(), number_json(row.rotation));
        }
        let mut config = match normalize(&Value::Object(config)) {
            Value::Object(m) => m,
            _ => JsonMap::new(),
        };
        drop_empty_bindings(&mut config);
        let sensors = serde_json::from_str::<Value>(&row.sensors)
            .map(|v| normalize(&v))
            .unwrap_or(Value::Array(Vec::new()));
        (
            Widget {
                kind: row.kind.clone(),
                name: row.name.clone(),
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
                sensors,
                config,
            },
            z,
        )
    }

    /// The row of this widget, with `config.z` set to `z` when given.
    pub fn to_row(&self, id: &str, z: Option<&Value>) -> WidgetRow {
        let mut config = self.config.clone();
        if let Some(z) = z {
            config.insert("z".into(), z.clone());
        }
        let rotation = config.get("rotation").and_then(number_of).unwrap_or(0.0);
        WidgetRow {
            id: id.to_string(),
            kind: self.kind.clone(),
            name: self.name.clone(),
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
            rotation: if rotation.is_finite() { rotation } else { 0.0 },
            sensors: self.sensors.to_string(),
            config: Value::Object(config).to_string(),
        }
    }

    /// The bindings when they are a map (none counts as an empty one), or
    /// `None` when `bindings` holds something else.
    fn bindings(&self) -> Option<JsonMap<String, Value>> {
        match self.config.get(BINDINGS) {
            None => Some(JsonMap::new()),
            Some(Value::Object(m)) => Some(m.clone()),
            Some(_) => None,
        }
    }
}

impl Connection {
    pub fn from_row(row: &ConnectionRow) -> Connection {
        Connection {
            from: row.from.clone(),
            to: row.to.clone(),
            kind: row.kind.clone(),
            waypoints: serde_json::from_str::<Value>(&row.waypoints)
                .map(|v| normalize(&v))
                .unwrap_or(Value::Array(Vec::new())),
        }
    }

    pub fn to_row(&self, id: &str) -> ConnectionRow {
        ConnectionRow {
            id: id.to_string(),
            from: self.from.clone(),
            to: self.to.clone(),
            kind: self.kind.clone(),
            waypoints: self.waypoints.to_string(),
        }
    }
}

/// Widget ids of `rows` in stacking order: by `config.z`, a row without one
/// at its position, ties in row order (the web's `loadLayoutFromAPI`).
pub fn row_order(rows: &[WidgetRow]) -> Vec<String> {
    let mut keyed: Vec<(f64, usize, &str)> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (_, z) = Widget::from_row(r);
            (
                z.filter(|z| z.is_finite()).unwrap_or(i as f64),
                i,
                r.id.as_str(),
            )
        })
        .collect();
    keyed.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    keyed.into_iter().map(|(_, _, id)| id.to_string()).collect()
}

impl Screen {
    /// The document `rows` load into (stacking indexes spread evenly).
    pub fn from_rows(rows: &Rows) -> Screen {
        let order = row_order(&rows.widgets);
        let keys = fractional::n_keys_between(None, None, order.len());
        let zi: BTreeMap<&str, &String> =
            order.iter().map(String::as_str).zip(keys.iter()).collect();
        Screen {
            widgets: rows
                .widgets
                .iter()
                .map(|r| {
                    let (w, _) = Widget::from_row(r);
                    let key = zi
                        .get(r.id.as_str())
                        .map(|k| (*k).clone())
                        .unwrap_or_default();
                    (r.id.clone(), (w, key))
                })
                .collect(),
            connections: rows
                .connections
                .iter()
                .map(|r| (r.id.clone(), Connection::from_row(r)))
                .collect(),
        }
    }

    /// Widget ids in stacking order: by `(zi, id)`.
    pub fn order(&self) -> Vec<String> {
        let mut ids: Vec<(&str, &str)> = self
            .widgets
            .iter()
            .map(|(id, (_, zi))| (zi.as_str(), id.as_str()))
            .collect();
        ids.sort();
        ids.into_iter().map(|(_, id)| id.to_string()).collect()
    }

    /// The rows of this document. `prev` are the rows it was last written
    /// as: a row whose object did not change is returned as it was (its text
    /// untouched), and `config.z` is only renumbered (0, 1, 2, …) when the
    /// stacking order changed. Rows come out in id order.
    pub fn to_rows(&self, prev: &Rows) -> Rows {
        let order = self.order();
        let prev_order = row_order(&prev.widgets);
        let same_order = order == prev_order;
        let prev_widgets: BTreeMap<&str, &WidgetRow> =
            prev.widgets.iter().map(|r| (r.id.as_str(), r)).collect();
        let rank: BTreeMap<&str, usize> = order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let mut widgets = Vec::with_capacity(self.widgets.len());
        for (id, (w, _)) in &self.widgets {
            let prev_row = prev_widgets.get(id.as_str()).copied();
            let prev_parsed = prev_row.map(Widget::from_row);
            let z: Option<Value> = if same_order {
                // The order stands: each row keeps the z it had.
                prev_row.and_then(|r| match serde_json::from_str::<Value>(&r.config) {
                    Ok(Value::Object(m)) => m.get("z").cloned(),
                    _ => None,
                })
            } else {
                Some(Value::from(
                    rank.get(id.as_str()).copied().unwrap_or(0) as u64
                ))
            };
            let unchanged = match (&prev_parsed, prev_row) {
                (Some((pw, pz)), Some(_)) => {
                    pw == w && (same_order || z.as_ref().and_then(number_of) == *pz)
                }
                _ => false,
            };
            match prev_row {
                Some(r) if unchanged => widgets.push(r.clone()),
                _ => widgets.push(w.to_row(id, z.as_ref())),
            }
        }
        let prev_conns: BTreeMap<&str, &ConnectionRow> = prev
            .connections
            .iter()
            .map(|r| (r.id.as_str(), r))
            .collect();
        let connections = self
            .connections
            .iter()
            .map(|(id, c)| match prev_conns.get(id.as_str()) {
                Some(r) if Connection::from_row(r) == *c => (*r).clone(),
                _ => c.to_row(id),
            })
            .collect();
        Rows {
            widgets,
            connections,
        }
    }
}

// ── Reading the document ──

fn read_widget<T: ReadTxn>(txn: &T, m: &MapRef) -> (Widget, String) {
    let text = |k: &str| match m.get(txn, k) {
        Some(Out::Any(Any::String(s))) => s.to_string(),
        _ => String::new(),
    };
    let num = |k: &str| match m.get(txn, k) {
        Some(Out::Any(a)) => any_to_json(&a).as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    let sensors = match m.get(txn, "sensors") {
        Some(o) => out_to_json(txn, &o),
        None => Value::Array(Vec::new()),
    };
    let config = match m.get(txn, "config") {
        Some(Out::YMap(c)) => c
            .iter(txn)
            .map(|(k, v)| (k.to_string(), out_to_json(txn, &v)))
            .collect(),
        Some(o) => match out_to_json(txn, &o) {
            Value::Object(m) => m,
            _ => JsonMap::new(),
        },
        None => JsonMap::new(),
    };
    let mut config: JsonMap<String, Value> = config;
    config.remove("z");
    drop_empty_bindings(&mut config);
    (
        Widget {
            kind: text("type"),
            name: text("name"),
            x: num("x"),
            y: num("y"),
            width: num("width"),
            height: num("height"),
            sensors,
            config,
        },
        text("zi"),
    )
}

fn read_connection<T: ReadTxn>(txn: &T, m: &MapRef) -> Connection {
    let text = |k: &str| match m.get(txn, k) {
        Some(Out::Any(Any::String(s))) => s.to_string(),
        _ => String::new(),
    };
    Connection {
        from: text("from"),
        to: text("to"),
        kind: text("type"),
        waypoints: match m.get(txn, "waypoints") {
            Some(o) => out_to_json(txn, &o),
            None => Value::Array(Vec::new()),
        },
    }
}

/// The document as it is now. Entries that are not maps are skipped.
pub fn read<T: ReadTxn>(txn: &T) -> Screen {
    let mut screen = Screen::default();
    if let Some(widgets) = txn.get_map(WIDGETS) {
        for (id, v) in widgets.iter(txn) {
            if let Out::YMap(m) = v {
                screen.widgets.insert(id.to_string(), read_widget(txn, &m));
            }
        }
    }
    if let Some(conns) = txn.get_map(CONNECTIONS) {
        for (id, v) in conns.iter(txn) {
            if let Out::YMap(m) = v {
                screen
                    .connections
                    .insert(id.to_string(), read_connection(txn, &m));
            }
        }
    }
    screen
}

// ── Writing the document ──

/// A config as it is stored. `bindings` is always there (an empty map when
/// nothing is bound), so two people binding the first properties of a widget
/// at once write into one map instead of each creating their own.
fn config_prelim(config: &JsonMap<String, Value>) -> MapPrelim {
    let mut m: MapPrelim = config
        .iter()
        .filter(|(k, _)| k.as_str() != "z")
        .map(|(k, v)| (k.clone(), value_in(k, v)))
        .collect();
    if !config.contains_key(BINDINGS) {
        m.insert(BINDINGS.into(), In::Map(MapPrelim::default()));
    }
    m
}

/// An empty `bindings` means none: drop it, so rows without one compare equal.
fn drop_empty_bindings(config: &mut JsonMap<String, Value>) {
    if config
        .get(BINDINGS)
        .is_some_and(|b| b.as_object().is_some_and(|m| m.is_empty()))
    {
        config.remove(BINDINGS);
    }
}

/// A config value as it is stored: `bindings` as a nested map.
fn value_in(key: &str, v: &Value) -> In {
    match (key, v) {
        (BINDINGS, Value::Object(m)) => In::Map(
            m.iter()
                .map(|(p, b)| (p.clone(), In::Any(json_to_any(b))))
                .collect(),
        ),
        _ => In::Any(json_to_any(v)),
    }
}

fn widget_prelim(w: &Widget, zi: &str) -> MapPrelim {
    MapPrelim::from_iter([
        ("type".to_string(), In::Any(Any::from(w.kind.as_str()))),
        ("name".to_string(), In::Any(Any::from(w.name.as_str()))),
        ("x".to_string(), In::Any(json_to_any(&number_json(w.x)))),
        ("y".to_string(), In::Any(json_to_any(&number_json(w.y)))),
        (
            "width".to_string(),
            In::Any(json_to_any(&number_json(w.width))),
        ),
        (
            "height".to_string(),
            In::Any(json_to_any(&number_json(w.height))),
        ),
        ("sensors".to_string(), In::Any(json_to_any(&w.sensors))),
        ("zi".to_string(), In::Any(Any::from(zi))),
        ("config".to_string(), In::Map(config_prelim(&w.config))),
    ])
}

fn connection_prelim(c: &Connection) -> MapPrelim {
    MapPrelim::from_iter([
        ("from".to_string(), In::Any(Any::from(c.from.as_str()))),
        ("to".to_string(), In::Any(Any::from(c.to.as_str()))),
        ("type".to_string(), In::Any(Any::from(c.kind.as_str()))),
        ("waypoints".to_string(), In::Any(json_to_any(&c.waypoints))),
    ])
}

/// One field write of a widget.
#[derive(Clone, Debug, PartialEq)]
enum WidgetField {
    Top(&'static str, Value),
    /// A config key set (`Some`) or removed (`None`).
    Config(String, Option<Value>),
    /// One binding set or removed, inside a `bindings` map on both sides.
    Binding(String, Option<Value>),
}

fn top_fields(w: &Widget) -> [(&'static str, Value); 7] {
    [
        ("type", Value::String(w.kind.clone())),
        ("name", Value::String(w.name.clone())),
        ("x", number_json(w.x)),
        ("y", number_json(w.y)),
        ("width", number_json(w.width)),
        ("height", number_json(w.height)),
        ("sensors", w.sensors.clone()),
    ]
}

/// The writes that turn `from` into `to`, field by field.
fn widget_changes(from: &Widget, to: &Widget) -> Vec<WidgetField> {
    let mut out = Vec::new();
    for ((k, a), (_, b)) in top_fields(from).into_iter().zip(top_fields(to)) {
        if a != b {
            out.push(WidgetField::Top(k, b));
        }
    }
    let keys: BTreeSet<&String> = from.config.keys().chain(to.config.keys()).collect();
    for k in keys {
        let (a, b) = (from.config.get(k), to.config.get(k));
        if a == b {
            continue;
        }
        if k == BINDINGS
            && let (Some(fa), Some(fb)) = (from.bindings(), to.bindings())
        {
            let props: BTreeSet<&String> = fa.keys().chain(fb.keys()).collect();
            for p in props {
                if fa.get(p) != fb.get(p) {
                    out.push(WidgetField::Binding(p.clone(), fb.get(p).cloned()));
                }
            }
            continue;
        }
        out.push(WidgetField::Config(k.clone(), b.cloned()));
    }
    out
}

fn apply_widget_changes(
    txn: &mut TransactionMut,
    m: &MapRef,
    changes: &[WidgetField],
    target: &Widget,
) {
    let config = match m.get(txn, "config") {
        Some(Out::YMap(c)) => c,
        _ => {
            // No config map (a damaged entry): write the target's whole.
            m.insert(txn, "config", config_prelim(&target.config))
        }
    };
    for c in changes {
        match c {
            WidgetField::Top(k, v) => {
                m.insert(txn, *k, json_to_any(v));
            }
            WidgetField::Config(k, Some(v)) => {
                config.insert(txn, k.as_str(), value_in(k, v));
            }
            WidgetField::Config(k, None) => {
                config.remove(txn, k);
            }
            WidgetField::Binding(p, v) => {
                let bindings = match config.get(txn, BINDINGS) {
                    Some(Out::YMap(b)) => b,
                    _ => config.insert(txn, BINDINGS, MapPrelim::default()),
                };
                match v {
                    Some(v) => {
                        bindings.insert(txn, p.as_str(), json_to_any(v));
                    }
                    None => {
                        bindings.remove(txn, p);
                    }
                }
            }
        }
    }
}

fn connection_changes(from: &Connection, to: &Connection) -> Vec<(&'static str, Value)> {
    let f = |c: &Connection| {
        [
            ("from", Value::String(c.from.clone())),
            ("to", Value::String(c.to.clone())),
            ("type", Value::String(c.kind.clone())),
            ("waypoints", c.waypoints.clone()),
        ]
    };
    f(from)
        .into_iter()
        .zip(f(to))
        .filter(|((_, a), (_, b))| a != b)
        .map(|(_, (k, b))| (k, b))
        .collect()
}

/// Fill an empty document with `rows`.
pub fn load(txn: &mut TransactionMut, rows: &Rows) {
    let widgets = txn.get_or_insert_map(WIDGETS);
    let conns = txn.get_or_insert_map(CONNECTIONS);
    let screen = Screen::from_rows(rows);
    for (id, (w, zi)) in &screen.widgets {
        widgets.insert(txn, id.as_str(), widget_prelim(w, zi));
    }
    for (id, c) in &screen.connections {
        conns.insert(txn, id.as_str(), connection_prelim(c));
    }
}

/// Make one widget what `target` says (`None`: remove it), writing only the
/// fields that differ from the document. `zi` is kept unless given.
pub fn put_widget(txn: &mut TransactionMut, id: &str, target: Option<&Widget>, zi: Option<&str>) {
    let widgets = txn.get_or_insert_map(WIDGETS);
    let Some(target) = target else {
        widgets.remove(txn, id);
        return;
    };
    match widgets.get(txn, id) {
        Some(Out::YMap(m)) => {
            let (current, cur_zi) = read_widget(txn, &m);
            let changes = widget_changes(&current, target);
            apply_widget_changes(txn, &m, &changes, target);
            if let Some(zi) = zi
                && zi != cur_zi
            {
                m.insert(txn, "zi", zi);
            }
        }
        _ => {
            let key = match zi {
                Some(k) => k.to_string(),
                None => {
                    let screen = read(txn);
                    let last = screen
                        .order()
                        .last()
                        .and_then(|id| screen.widgets.get(id))
                        .map(|(_, z)| z.clone());
                    fractional::key_between(last.as_deref(), None)
                }
            };
            widgets.insert(txn, id, widget_prelim(target, &key));
        }
    }
}

/// Make one connection what `target` says (`None`: remove it).
pub fn put_connection(txn: &mut TransactionMut, id: &str, target: Option<&Connection>) {
    let conns = txn.get_or_insert_map(CONNECTIONS);
    let Some(target) = target else {
        conns.remove(txn, id);
        return;
    };
    match conns.get(txn, id) {
        Some(Out::YMap(m)) => {
            let current = read_connection(txn, &m);
            for (k, v) in connection_changes(&current, target) {
                m.insert(txn, k, json_to_any(&v));
            }
        }
        _ => {
            conns.insert(txn, id, connection_prelim(target));
        }
    }
}

/// What [`apply_rows`] changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    pub objects: Vec<(ObjectKind, String)>,
    pub reordered: bool,
}

/// Apply the difference between `before` and `after` (rows written outside
/// the document: REST, MCP, import, the server binding equipment) to the
/// document, field by field, so edits in the document to other fields stand.
/// An object removed in the document but changed in `after` comes back; one
/// removed in `after` goes. When the stacking order of `after` differs from
/// `before`, the widgets of `after` get new stacking indexes in its order.
pub fn apply_rows(txn: &mut TransactionMut, before: &Rows, after: &Rows) -> Applied {
    let mut applied = Applied::default();
    let b = Screen::from_rows(before);
    let a = Screen::from_rows(after);
    let widgets = txn.get_or_insert_map(WIDGETS);
    let ids: BTreeSet<&String> = b.widgets.keys().chain(a.widgets.keys()).collect();
    for id in ids {
        match (b.widgets.get(id), a.widgets.get(id)) {
            (Some((bw, _)), Some((aw, _))) if bw == aw => {}
            (_, None) => {
                if widgets.get(txn, id).is_some() {
                    widgets.remove(txn, id);
                }
                applied.objects.push((ObjectKind::Widget, id.clone()));
            }
            (before_w, Some((aw, azi))) => {
                match (widgets.get(txn, id), before_w) {
                    (Some(Out::YMap(m)), Some((bw, _))) => {
                        let changes = widget_changes(bw, aw);
                        apply_widget_changes(txn, &m, &changes, aw);
                    }
                    (Some(Out::YMap(_)), None) => put_widget(txn, id, Some(aw), None),
                    _ => {
                        widgets.insert(txn, id.as_str(), widget_prelim(aw, azi));
                    }
                }
                applied.objects.push((ObjectKind::Widget, id.clone()));
            }
        }
    }
    let conns = txn.get_or_insert_map(CONNECTIONS);
    let ids: BTreeSet<&String> = b.connections.keys().chain(a.connections.keys()).collect();
    for id in ids {
        match (b.connections.get(id), a.connections.get(id)) {
            (Some(bc), Some(ac)) if bc == ac => {}
            (_, None) => {
                if conns.get(txn, id).is_some() {
                    conns.remove(txn, id);
                }
                applied.objects.push((ObjectKind::Connection, id.clone()));
            }
            (bc, Some(ac)) => {
                match (conns.get(txn, id), bc) {
                    (Some(Out::YMap(m)), Some(bc)) => {
                        for (k, v) in connection_changes(bc, ac) {
                            m.insert(txn, k, json_to_any(&v));
                        }
                    }
                    _ => put_connection(txn, id, Some(ac)),
                }
                applied.objects.push((ObjectKind::Connection, id.clone()));
            }
        }
    }
    let order_after = row_order(&after.widgets);
    if order_after != row_order(&before.widgets) {
        let keys = fractional::n_keys_between(None, None, order_after.len());
        for (id, key) in order_after.iter().zip(keys) {
            if let Some(Out::YMap(m)) = widgets.get(txn, id) {
                m.insert(txn, "zi", key);
            }
        }
        applied.reordered = true;
    }
    applied
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use yrs::{Doc, Transact};

    pub(crate) fn wrow(id: &str, x: f64, config: &str) -> WidgetRow {
        WidgetRow {
            id: id.into(),
            kind: "symbol".into(),
            name: id.to_uppercase(),
            x,
            y: 10.0,
            width: 90.0,
            height: 100.0,
            rotation: 0.0,
            sensors: "[1]".into(),
            config: config.into(),
        }
    }

    pub(crate) fn crow(id: &str, from: &str, to: &str) -> ConnectionRow {
        ConnectionRow {
            id: id.into(),
            from: from.into(),
            to: to.into(),
            kind: "flow".into(),
            waypoints: "[]".into(),
        }
    }

    fn sample() -> Rows {
        Rows {
            widgets: vec![
                wrow(
                    "a",
                    0.0,
                    r#"{"symbol":"isa_pump","z":1,"bindings":{"label":{"kind":"tag","tag":"d/k"}}}"#,
                ),
                wrow("b", 100.0, r#"{"label":"B","z":0}"#),
            ],
            connections: vec![crow("c1", "a", "b")],
        }
    }

    fn doc_of(rows: &Rows) -> Doc {
        let doc = Doc::new();
        load(&mut doc.transact_mut(), rows);
        doc
    }

    #[test]
    fn rows_round_trip_untouched() {
        let rows = sample();
        let doc = doc_of(&rows);
        let screen = read(&doc.transact());
        assert_eq!(screen.order(), vec!["b", "a"]);
        assert_eq!(
            screen.to_rows(&rows),
            rows,
            "nothing changed: the same text"
        );
    }

    #[test]
    fn a_moved_widget_is_one_changed_row() {
        let rows = sample();
        let doc = doc_of(&rows);
        {
            let mut txn = doc.transact_mut();
            let screen = read(&txn);
            let mut a = screen.widgets["a"].0.clone();
            a.x = 55.0;
            put_widget(&mut txn, "a", Some(&a), None);
        }
        let out = read(&doc.transact()).to_rows(&rows);
        assert_eq!(out.widgets[1], rows.widgets[1]);
        assert_eq!(out.widgets[0].x, 55.0);
        let (w, z) = Widget::from_row(&out.widgets[0]);
        assert_eq!(z, Some(1.0), "order unchanged: z kept");
        assert_eq!(w.config["bindings"]["label"]["tag"], "d/k");
    }

    #[test]
    fn a_reorder_renumbers_z() {
        let rows = sample();
        let doc = doc_of(&rows);
        {
            let mut txn = doc.transact_mut();
            let screen = read(&txn);
            let top = screen.widgets["a"].1.clone();
            let b = screen.widgets["b"].0.clone();
            let key = fractional::key_between(Some(&top), None);
            put_widget(&mut txn, "b", Some(&b), Some(&key));
        }
        let out = read(&doc.transact()).to_rows(&rows);
        let z = |i: usize| Widget::from_row(&out.widgets[i]).1;
        assert_eq!((z(0), z(1)), (Some(0.0), Some(1.0)), "a under b");
    }

    #[test]
    fn rotation_column_folds_into_config() {
        let mut r = wrow("r", 0.0, r#"{"z":0}"#);
        r.rotation = 90.0;
        let rows = Rows {
            widgets: vec![r.clone()],
            connections: vec![],
        };
        let screen = Screen::from_rows(&rows);
        assert_eq!(screen.widgets["r"].0.config["rotation"], 90);
        assert_eq!(
            screen.to_rows(&rows).widgets[0],
            r,
            "same meaning: row kept"
        );
        let row = screen.widgets["r"].0.to_row("r", None);
        assert_eq!(row.rotation, 90.0);
    }

    #[test]
    fn external_rows_merge_field_by_field() {
        let rows = sample();
        let doc = doc_of(&rows);
        // In the document: a's label set.
        {
            let mut txn = doc.transact_mut();
            let mut a = read(&txn).widgets["a"].0.clone();
            a.config.insert("label".into(), "Pump".into());
            put_widget(&mut txn, "a", Some(&a), None);
        }
        // Outside: a moved, b removed (and its connection), d added.
        let mut after = rows.clone();
        after.widgets[0].x = 300.0;
        after.widgets.remove(1);
        after.connections.clear();
        after.widgets.push(wrow("d", 5.0, r#"{"z":7}"#));
        let applied = apply_rows(&mut doc.transact_mut(), &rows, &after);
        assert!(applied.reordered);
        let screen = read(&doc.transact());
        let a = &screen.widgets["a"].0;
        assert_eq!((a.x, a.config["label"].as_str()), (300.0, Some("Pump")));
        assert!(!screen.widgets.contains_key("b"));
        assert!(screen.connections.is_empty());
        assert_eq!(screen.order(), vec!["a", "d"]);
    }

    #[test]
    fn bindings_of_different_properties_merge_between_replicas() {
        let rows = sample();
        let d1 = doc_of(&rows);
        let d2 = Doc::new();
        {
            use yrs::updates::decoder::Decode;
            let u = d1
                .transact()
                .encode_state_as_update_v1(&yrs::StateVector::default());
            d2.transact_mut()
                .apply_update(yrs::Update::decode_v1(&u).unwrap())
                .unwrap();
        }
        let bind = |doc: &Doc, prop: &str| {
            let mut txn = doc.transact_mut();
            let mut a = read(&txn).widgets["a"].0.clone();
            a.config
                .get_mut("bindings")
                .and_then(Value::as_object_mut)
                .unwrap()
                .insert(
                    prop.into(),
                    serde_json::json!({"kind": "tag", "tag": format!("d/{prop}")}),
                );
            put_widget(&mut txn, "a", Some(&a), None);
        };
        bind(&d1, "visible");
        bind(&d2, "opacity");
        crate::wire::tests::exchange(&d1, &d2);
        let s1 = read(&d1.transact());
        let s2 = read(&d2.transact());
        assert_eq!(s1, s2);
        let b = &s1.widgets["a"].0.config["bindings"];
        assert!(
            b.get("visible").is_some() && b.get("opacity").is_some() && b.get("label").is_some()
        );
    }

    #[test]
    fn numbers_compare_as_javascript_writes_them() {
        assert_eq!(normalize(&serde_json::json!(3.0)), serde_json::json!(3));
        assert_eq!(
            any_to_json(&json_to_any(&serde_json::json!({"a": [1, 2.5]}))),
            serde_json::json!({"a": [1, 2.5]})
        );
    }
}
