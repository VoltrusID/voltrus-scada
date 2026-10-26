//! HMI mimic screens (the web client's SCADA view): the server's rows, the
//! screen model they parse into, the symbol library's geometry, how bound
//! tags drive what is drawn, how connections route between equipment, and
//! the faceplate's command rules. No UI here: the console draws it
//! (`console/views/mimic/`), and everything below is tested on its own.
//!
//! # The server's side (`/api/v1`)
//!
//! | What | Endpoint | Role |
//! |---|---|---|
//! | A screen's equipment | `GET /screens/{id}/scada-equipment` → `{items: [EquipmentRow]}` | any |
//! | Its connections | `GET /screens/{id}/scada-connections` → `{items: [ConnectionRow]}` | any |
//! | Save them | `PUT` the same paths with `{items}`; each replaces the screen's rows | operator |
//!
//! The overview keeps its mimic under `__overview`, like its card layouts.
//! `sensor_bindings`, `config` and `waypoints` are JSON **text** inside the
//! rows. The stacking order travels in `config.z` (the server answers rows
//! sorted by id).
//!
//! # The model
//!
//! [`parse`] follows the web client's `loadLayoutFromAPI`
//! (`client/src/scada2/engine/layout.ts`) and [`to_rows`] its `toAPI`
//! (`ScadaView.tsx`), so a [`Layout`] is plain data an editor can change and
//! save back unchanged in shape: [`Widget`]s in stacking order with their
//! bound tags and their `config` as JSON, and [`Connection`]s between them.
//! [`resolve`] maps a widget to the library symbol that draws it
//! (`hmi/resolve.ts`), and the live state of a symbol comes from
//! [`symbol_live`] (`hmi/SymbolView.tsx`).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::api::Alarm;
use crate::screens::layout_id;
use crate::tag_api::WriteMode;

/// Screens are authored on a fixed stage and scaled to the viewport.
pub const STAGE_W: f64 = 1920.0;
pub const STAGE_H: f64 = 1080.0;

/// `GET` and `PUT` path of a screen's equipment.
pub fn equipment_path(screen: &str) -> String {
    format!("/api/v1/screens/{}/scada-equipment", layout_id(screen))
}

/// `GET` and `PUT` path of a screen's connections.
pub fn connections_path(screen: &str) -> String {
    format!("/api/v1/screens/{}/scada-connections", layout_id(screen))
}

// ── The server's rows ─────────────────────────────────────────────────────

/// Mirrors `voltrus::models::ScadaEquipment`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EquipmentRow {
    /// Answered by `GET`; the `PUT` takes the screen from its path.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screen_id: String,
    pub equipment_id: String,
    pub equipment_type: String,
    pub equipment_name: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: f64,
    /// JSON text: the bound data source ids, e.g. `"[12,14]"`.
    pub sensor_bindings: String,
    /// JSON text: the widget's settings, with its stacking order in `z`.
    pub config: String,
}

/// Mirrors `voltrus::models::ScadaConnection`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionRow {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub screen_id: String,
    pub connection_id: String,
    pub from_equipment: String,
    pub to_equipment: String,
    /// JSON text: `[[x, y], …]` on the stage.
    pub waypoints: String,
    pub connection_type: String,
}

/// The `{items: [...]}` envelope of both endpoints, both ways.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Items<T> {
    pub items: Vec<T>,
}

// ── The model ─────────────────────────────────────────────────────────────

/// The widget types the web client has (`widgets/types.ts`). `Symbol` is a
/// library symbol (`config.symbol`); `Pump`, `Motor`, `Valve`, `TankLevel`,
/// `HorizontalTank` and `Pipe` are the legacy equipment widgets, drawn by
/// the same library.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WidgetKind {
    Symbol,
    Area,
    Gauge,
    Indicator,
    Bar,
    StatusLight,
    TankLevel,
    OnOff,
    Label,
    Trend,
    BigNumber,
    DataTable,
    Pipe,
    Slider,
    NumberInput,
    Toggle,
    Meter,
    SemicircleGauge,
    Donut,
    LedDisplay,
    Motor,
    Valve,
    Pump,
    PushButton,
    BarChart,
    PieChart,
    MultiTrend,
    Annunciator,
    PidFaceplate,
    HorizontalTank,
    ProgressBar,
    /// A type this console does not know; kept so a save keeps it.
    Other(String),
}

impl WidgetKind {
    /// Every known type with its wire name.
    pub const ALL: [(Self, &'static str); 31] = [
        (Self::Symbol, "symbol"),
        (Self::Area, "area"),
        (Self::Gauge, "gauge"),
        (Self::Indicator, "indicator"),
        (Self::Bar, "bar"),
        (Self::StatusLight, "status_light"),
        (Self::TankLevel, "tank_level"),
        (Self::OnOff, "on_off"),
        (Self::Label, "label"),
        (Self::Trend, "trend"),
        (Self::BigNumber, "big_number"),
        (Self::DataTable, "data_table"),
        (Self::Pipe, "pipe"),
        (Self::Slider, "slider"),
        (Self::NumberInput, "number_input"),
        (Self::Toggle, "toggle"),
        (Self::Meter, "meter"),
        (Self::SemicircleGauge, "semicircle_gauge"),
        (Self::Donut, "donut"),
        (Self::LedDisplay, "led_display"),
        (Self::Motor, "motor"),
        (Self::Valve, "valve"),
        (Self::Pump, "pump"),
        (Self::PushButton, "push_button"),
        (Self::BarChart, "bar_chart"),
        (Self::PieChart, "pie_chart"),
        (Self::MultiTrend, "multi_trend"),
        (Self::Annunciator, "annunciator"),
        (Self::PidFaceplate, "pid_faceplate"),
        (Self::HorizontalTank, "horizontal_tank"),
        (Self::ProgressBar, "progress_bar"),
    ];

    pub fn parse(name: &str) -> Self {
        Self::ALL
            .iter()
            .find(|(_, wire)| *wire == name)
            .map_or_else(|| Self::Other(name.to_owned()), |(kind, _)| kind.clone())
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Other(name) => name,
            known => Self::ALL
                .iter()
                .find(|(kind, _)| kind == known)
                .map_or("", |(_, wire)| wire),
        }
    }

    /// Drawn by the symbol library.
    pub fn is_equipment(&self) -> bool {
        matches!(
            self,
            Self::Symbol
                | Self::Pump
                | Self::Motor
                | Self::Valve
                | Self::TankLevel
                | Self::HorizontalTank
                | Self::Pipe
        )
    }

    /// Clicking it at runtime opens its faceplate (the web client's
    /// `FACEPLATE_TYPES`), when it has a tag.
    pub fn has_faceplate(&self) -> bool {
        self.is_equipment()
            || self.is_control()
            || matches!(
                self,
                Self::Gauge
                    | Self::Indicator
                    | Self::Bar
                    | Self::StatusLight
                    | Self::BigNumber
                    | Self::Meter
                    | Self::SemicircleGauge
                    | Self::Donut
                    | Self::LedDisplay
                    | Self::Trend
                    | Self::ProgressBar
                    | Self::OnOff
                    | Self::PidFaceplate
            )
    }

    /// An operator control that writes its tag (the web client writes on
    /// click; the console arms the write in the faceplate first).
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Toggle | Self::PushButton | Self::Slider | Self::NumberInput
        )
    }

    /// Strokes may run past the box (symbols draw right at their edge).
    pub fn unclipped(&self) -> bool {
        self.is_equipment() || *self == Self::Area
    }
}

/// One widget on the stage. Plain data: an editor moves, resizes, rebinds
/// and reconfigures it, and [`to_rows`] saves it back.
#[derive(Debug, Clone, PartialEq)]
pub struct Widget {
    pub id: String,
    pub kind: WidgetKind,
    /// `equipment_name`: kept as stored.
    pub name: String,
    /// The bound data source ids; the first is the primary tag.
    pub tags: Vec<i64>,
    /// Stored bindings that are not ids (a FEED template's signal names):
    /// not drawn, but saved back after [`Self::tags`] so a save keeps them.
    pub signals: Vec<Value>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// The widget's settings, without `z` (the order of
    /// [`Layout::widgets`] is the stacking order).
    pub config: Map<String, Value>,
}

impl Widget {
    /// The primary tag.
    pub fn tag(&self) -> Option<i64> {
        self.tags.first().copied()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.config.get(key).filter(|value| !value.is_null())
    }

    /// A number setting (also from text, as the inspector's selects save).
    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(number)
    }

    /// A text setting.
    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// JavaScript truthiness of a setting, `None` when unset.
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.get(key).map(truthy)
    }

    /// A tag setting (`runSensorId`, `faultSensorId`).
    pub fn tag_setting(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(id)
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Whether the point (on the stage) is inside the box.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

/// How a connection is drawn (`connections/ConnectionLayer.tsx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// A process pipe: animated while either end runs.
    Flow,
    /// A power line.
    Power,
    /// A signal: anything else, as the web client draws it.
    Data,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connection {
    pub id: String,
    pub from: String,
    pub to: String,
    /// `connection_type` as stored (`flow`, `power`, `data`, or another
    /// name a FEED template wrote).
    pub kind: String,
    pub waypoints: Vec<(f64, f64)>,
}

impl Connection {
    pub fn line(&self) -> LineKind {
        match self.kind.as_str() {
            "flow" => LineKind::Flow,
            "power" => LineKind::Power,
            _ => LineKind::Data,
        }
    }
}

/// A screen's mimic: widgets in stacking order (first at the back) and the
/// connections between them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Layout {
    pub widgets: Vec<Widget>,
    pub connections: Vec<Connection>,
}

impl Layout {
    pub fn widget(&self, id: &str) -> Option<&Widget> {
        self.widgets.iter().find(|widget| widget.id == id)
    }

    /// Every tag the screen shows, once each, in widget order.
    pub fn tags(&self) -> Vec<i64> {
        let mut tags: Vec<i64> = Vec::new();
        for widget in &self.widgets {
            let settings = ["runSensorId", "faultSensorId"]
                .iter()
                .filter_map(|key| widget.tag_setting(key));
            for tag in widget.tags.iter().copied().chain(settings) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
        tags
    }
}

/// A JSON number, or text holding one.
fn number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    };
    number.filter(|number| number.is_finite())
}

/// A data source id: a whole number, or text holding one.
#[allow(clippy::cast_possible_truncation)]
fn id(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|n| n.fract() == 0.0)
                .map(|n| n as i64)
        }),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// JavaScript's truthiness.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The screen's rows as a [`Layout`], like the web client's
/// `loadLayoutFromAPI`: widgets sorted by `config.z` (row order where it is
/// missing), `z` taken out of the config, bindings and waypoints parsed.
/// Malformed JSON text reads as empty rather than failing the screen, and
/// bindings that are not ids (a FEED template's signal names) go to
/// [`Widget::signals`], kept for the save.
pub fn parse(equipment: &[EquipmentRow], connections: &[ConnectionRow]) -> Layout {
    let mut parsed: Vec<(f64, Widget)> = equipment
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let mut config = match serde_json::from_str::<Value>(&row.config) {
                Ok(Value::Object(map)) => map,
                _ => Map::new(),
            };
            #[allow(clippy::cast_precision_loss)]
            let z = config
                .remove("z")
                .and_then(|z| z.as_f64())
                .unwrap_or(index as f64);
            let (tags, signals) = match serde_json::from_str::<Value>(&row.sensor_bindings) {
                Ok(Value::Array(items)) => {
                    let mut tags = Vec::new();
                    let mut signals = Vec::new();
                    for item in items {
                        match id(&item) {
                            Some(tag) => tags.push(tag),
                            None => signals.push(item),
                        }
                    }
                    (tags, signals)
                }
                _ => (Vec::new(), Vec::new()),
            };
            let widget = Widget {
                id: row.equipment_id.clone(),
                kind: WidgetKind::parse(&row.equipment_type),
                name: row.equipment_name.clone(),
                tags,
                signals,
                x: row.x,
                y: row.y,
                width: row.width,
                height: row.height,
                config,
            };
            (z, widget)
        })
        .collect();
    parsed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let connections = connections
        .iter()
        .map(|row| Connection {
            id: row.connection_id.clone(),
            from: row.from_equipment.clone(),
            to: row.to_equipment.clone(),
            kind: row.connection_type.clone(),
            waypoints: serde_json::from_str::<Vec<(f64, f64)>>(&row.waypoints).unwrap_or_default(),
        })
        .collect();
    Layout {
        widgets: parsed.into_iter().map(|(_, widget)| widget).collect(),
        connections,
    }
}

/// The rows to `PUT` for a layout, like the web client's `toAPI`: the
/// stacking order goes back into `config.z`, the rotation column mirrors
/// `config.rotation`.
pub fn to_rows(layout: &Layout) -> (Vec<EquipmentRow>, Vec<ConnectionRow>) {
    let equipment = layout
        .widgets
        .iter()
        .enumerate()
        .map(|(index, widget)| {
            let mut config = widget.config.clone();
            config.insert("z".to_owned(), Value::from(index));
            EquipmentRow {
                screen_id: String::new(),
                equipment_id: widget.id.clone(),
                equipment_type: widget.kind.as_str().to_owned(),
                equipment_name: widget.name.clone(),
                x: widget.x,
                y: widget.y,
                width: widget.width,
                height: widget.height,
                rotation: widget.number("rotation").unwrap_or(0.0),
                sensor_bindings: bindings(widget),
                config: Value::Object(config).to_string(),
            }
        })
        .collect();
    let connections = layout
        .connections
        .iter()
        .map(|connection| ConnectionRow {
            screen_id: String::new(),
            connection_id: connection.id.clone(),
            from_equipment: connection.from.clone(),
            to_equipment: connection.to.clone(),
            waypoints: serde_json::to_string(&connection.waypoints)
                .unwrap_or_else(|_| "[]".to_owned()),
            connection_type: connection.kind.clone(),
        })
        .collect();
    (equipment, connections)
}

/// `sensor_bindings` as JSON text: the ids, then the bindings that are not.
fn bindings(widget: &Widget) -> String {
    let items: Vec<Value> = widget
        .tags
        .iter()
        .map(|tag| Value::from(*tag))
        .chain(widget.signals.iter().cloned())
        .collect();
    Value::Array(items).to_string()
}

// ── The symbol library (`hmi/symbols.tsx`) ───────────────────────────────

/// How a symbol's primary tag drives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    /// Running when the value is above the threshold.
    State,
    /// A fill level between the tag's minimum and maximum.
    Level,
    /// A value shown on or under it.
    Value,
    /// Nothing, unless a run tag animates it.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Rotating,
    Valves,
    Vessels,
    Process,
    Electrical,
    Instruments,
    Piping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    fn horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// The side a quarter turn clockwise.
    fn clockwise(self) -> Self {
        match self {
            Self::Left => Self::Top,
            Self::Top => Self::Right,
            Self::Right => Self::Bottom,
            Self::Bottom => Self::Left,
        }
    }

    fn mirrored(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            other => other,
        }
    }
}

/// Where connections attach, in view box units; a missing side uses the
/// view box edge's midpoint.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Ports {
    pub left: Option<(f64, f64)>,
    pub right: Option<(f64, f64)>,
    pub top: Option<(f64, f64)>,
    pub bottom: Option<(f64, f64)>,
}

impl Ports {
    pub fn get(&self, side: Side) -> Option<(f64, f64)> {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
            Side::Top => self.top,
            Side::Bottom => self.bottom,
        }
    }

    fn sides(&self) -> Vec<Side> {
        Side::ALL
            .into_iter()
            .filter(|side| self.get(*side).is_some())
            .collect()
    }

    fn any(&self) -> bool {
        self.left.is_some() || self.right.is_some() || self.top.is_some() || self.bottom.is_some()
    }
}

/// One symbol of the library: what the console needs besides its drawing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SymbolDef {
    pub key: &'static str,
    pub name: &'static str,
    pub category: Category,
    pub view_box: (f64, f64),
    /// The size a new one is placed at.
    pub size: (f64, f64),
    pub binding: Binding,
    /// Stretched to the widget's box (pipes, busbars) rather than fitted.
    pub stretch: bool,
    pub ports: Ports,
}

const fn ports(
    left: Option<(f64, f64)>,
    right: Option<(f64, f64)>,
    top: Option<(f64, f64)>,
    bottom: Option<(f64, f64)>,
) -> Ports {
    Ports {
        left,
        right,
        top,
        bottom,
    }
}

#[allow(clippy::too_many_arguments)]
const fn symbol(
    key: &'static str,
    name: &'static str,
    category: Category,
    view_box: (f64, f64),
    size: (f64, f64),
    binding: Binding,
    stretch: bool,
    ports: Ports,
) -> SymbolDef {
    SymbolDef {
        key,
        name,
        category,
        view_box,
        size,
        binding,
        stretch,
        ports,
    }
}

use Binding as B;
use Category as C;

/// The library, in the web client's palette order.
pub static SYMBOLS: &[SymbolDef] = &[
    symbol(
        "pump",
        "Centrifugal Pump",
        C::Rotating,
        (100.0, 100.0),
        (90.0, 90.0),
        B::State,
        false,
        ports(Some((2.0, 52.5)), Some((97.0, 29.5)), None, None),
    ),
    symbol(
        "dosing_pump",
        "Dosing Pump",
        C::Rotating,
        (100.0, 90.0),
        (84.0, 76.0),
        B::State,
        false,
        ports(Some((0.0, 58.0)), Some((100.0, 58.0)), None, None),
    ),
    symbol(
        "fan",
        "Fan / Blower",
        C::Rotating,
        (100.0, 100.0),
        (90.0, 90.0),
        B::State,
        false,
        ports(
            Some((6.0, 50.0)),
            Some((94.0, 50.0)),
            Some((50.0, 6.0)),
            Some((50.0, 94.0)),
        ),
    ),
    symbol(
        "motor",
        "Electric Motor",
        C::Rotating,
        (120.0, 80.0),
        (120.0, 80.0),
        B::State,
        false,
        ports(None, Some((117.0, 40.0)), Some((59.5, 2.0)), None),
    ),
    symbol(
        "compressor",
        "Compressor",
        C::Rotating,
        (100.0, 100.0),
        (90.0, 90.0),
        B::State,
        false,
        ports(Some((2.0, 50.5)), Some((98.0, 50.5)), None, None),
    ),
    symbol(
        "conveyor",
        "Belt Conveyor",
        C::Rotating,
        (200.0, 64.0),
        (220.0, 64.0),
        B::State,
        false,
        ports(Some((10.0, 28.0)), Some((190.0, 28.0)), None, None),
    ),
    symbol(
        "valve",
        "Gate Valve",
        C::Valves,
        (100.0, 80.0),
        (90.0, 72.0),
        B::State,
        false,
        ports(Some((0.0, 52.0)), Some((100.0, 52.0)), None, None),
    ),
    symbol(
        "control_valve",
        "Control Valve",
        C::Valves,
        (100.0, 100.0),
        (90.0, 90.0),
        B::Value,
        false,
        ports(Some((0.0, 72.0)), Some((100.0, 72.0)), None, None),
    ),
    symbol(
        "check_valve",
        "Check Valve",
        C::Valves,
        (100.0, 60.0),
        (80.0, 48.0),
        B::None,
        false,
        ports(Some((0.0, 30.0)), Some((100.0, 30.0)), None, None),
    ),
    symbol(
        "tank",
        "Vertical Tank",
        C::Vessels,
        (100.0, 170.0),
        (110.0, 180.0),
        B::Level,
        false,
        ports(
            Some((12.0, 110.0)),
            Some((88.0, 130.0)),
            Some((50.0, 6.0)),
            Some((50.0, 152.0)),
        ),
    ),
    symbol(
        "tank_horizontal",
        "Horizontal Tank",
        C::Vessels,
        (180.0, 100.0),
        (200.0, 110.0),
        B::Level,
        false,
        ports(
            Some((-1.0, 60.0)),
            Some((181.0, 60.0)),
            Some((90.0, 8.0)),
            Some((90.0, 84.0)),
        ),
    ),
    symbol(
        "silo",
        "Silo / Hopper",
        C::Vessels,
        (100.0, 170.0),
        (100.0, 170.0),
        B::Level,
        false,
        ports(None, None, Some((50.0, 6.0)), Some((50.0, 166.0))),
    ),
    symbol(
        "mixer",
        "Mixing Tank",
        C::Vessels,
        (100.0, 170.0),
        (110.0, 180.0),
        B::Level,
        false,
        ports(
            Some((12.0, 110.0)),
            Some((88.0, 140.0)),
            None,
            Some((50.0, 160.0)),
        ),
    ),
    symbol(
        "clarifier",
        "Clarifier",
        C::Process,
        (220.0, 100.0),
        (240.0, 110.0),
        B::Value,
        false,
        ports(
            Some((0.0, 30.0)),
            Some((220.0, 22.0)),
            None,
            Some((110.0, 98.0)),
        ),
    ),
    symbol(
        "gravity_filter",
        "Gravity Filter",
        C::Process,
        (140.0, 120.0),
        (140.0, 120.0),
        B::Value,
        false,
        ports(
            Some((0.0, 26.0)),
            Some((140.0, 106.0)),
            None,
            Some((70.0, 120.0)),
        ),
    ),
    symbol(
        "heat_exchanger",
        "Heat Exchanger",
        C::Process,
        (140.0, 80.0),
        (150.0, 86.0),
        B::None,
        false,
        ports(
            Some((0.0, 40.0)),
            Some((140.0, 40.0)),
            Some((42.5, 2.0)),
            Some((97.5, 78.0)),
        ),
    ),
    symbol(
        "filter",
        "Filter",
        C::Process,
        (80.0, 120.0),
        (70.0, 110.0),
        B::None,
        false,
        ports(Some((0.0, 28.5)), Some((80.0, 91.5)), None, None),
    ),
    symbol(
        "breaker",
        "Circuit Breaker",
        C::Electrical,
        (60.0, 100.0),
        (54.0, 90.0),
        B::State,
        false,
        ports(None, None, Some((30.0, 0.0)), Some((30.0, 100.0))),
    ),
    symbol(
        "transformer",
        "Transformer",
        C::Electrical,
        (80.0, 120.0),
        (72.0, 108.0),
        B::State,
        false,
        ports(None, None, Some((40.0, 0.0)), Some((40.0, 120.0))),
    ),
    symbol(
        "generator",
        "Generator",
        C::Electrical,
        (100.0, 100.0),
        (90.0, 90.0),
        B::State,
        false,
        ports(Some((12.0, 50.0)), Some((88.0, 50.0)), None, None),
    ),
    symbol(
        "solar_panel",
        "Solar Array",
        C::Electrical,
        (120.0, 100.0),
        (120.0, 100.0),
        B::State,
        false,
        ports(None, None, None, Some((60.0, 97.0))),
    ),
    symbol(
        "battery",
        "Battery (SoC)",
        C::Electrical,
        (120.0, 70.0),
        (120.0, 70.0),
        B::Level,
        false,
        ports(Some((5.0, 35.0)), Some((115.0, 35.0)), None, None),
    ),
    symbol(
        "inverter",
        "Inverter",
        C::Electrical,
        (90.0, 100.0),
        (80.0, 90.0),
        B::State,
        false,
        ports(
            Some((8.0, 50.0)),
            Some((82.0, 50.0)),
            Some((45.0, 8.0)),
            Some((45.0, 92.0)),
        ),
    ),
    symbol(
        "meter",
        "Power Meter",
        C::Electrical,
        (90.0, 90.0),
        (96.0, 96.0),
        B::Value,
        false,
        ports(Some((4.0, 45.0)), Some((86.0, 45.0)), None, None),
    ),
    symbol(
        "busbar",
        "Busbar",
        C::Electrical,
        (200.0, 16.0),
        (240.0, 14.0),
        B::State,
        true,
        ports(None, None, None, None),
    ),
    symbol(
        "instrument",
        "Instrument (ISA)",
        C::Instruments,
        (80.0, 80.0),
        (84.0, 108.0),
        B::Value,
        false,
        ports(
            Some((10.0, 40.0)),
            Some((70.0, 40.0)),
            Some((40.0, 10.0)),
            Some((40.0, 70.0)),
        ),
    ),
    symbol(
        "flow_meter",
        "Mag Flowmeter",
        C::Instruments,
        (100.0, 70.0),
        (96.0, 66.0),
        B::Value,
        false,
        ports(Some((0.0, 46.0)), Some((100.0, 46.0)), None, None),
    ),
    symbol(
        "pipe",
        "Pipe",
        C::Piping,
        (200.0, 20.0),
        (200.0, 20.0),
        B::None,
        true,
        ports(None, None, None, None),
    ),
    symbol(
        "pipe_vertical",
        "Pipe (vertical)",
        C::Piping,
        (20.0, 200.0),
        (20.0, 200.0),
        B::None,
        true,
        ports(None, None, None, None),
    ),
    symbol(
        "pipe_elbow",
        "Pipe Elbow",
        C::Piping,
        (60.0, 60.0),
        (60.0, 60.0),
        B::None,
        false,
        ports(None, None, None, None),
    ),
    symbol(
        "pipe_tee",
        "Pipe Tee",
        C::Piping,
        (80.0, 50.0),
        (80.0, 50.0),
        B::None,
        false,
        ports(None, None, None, None),
    ),
];

/// The symbol with this key.
pub fn symbol_def(key: &str) -> Option<&'static SymbolDef> {
    SYMBOLS.iter().find(|def| def.key == key)
}

/// A widget seen as a library symbol, its settings normalised to the
/// symbol vocabulary (`hmi/resolve.ts`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolved {
    pub def: &'static SymbolDef,
    /// A legacy pump, motor or valve: started and stopped from its
    /// faceplate unless `writable` is turned off; new symbols opt in.
    pub default_writable: bool,
    /// The value badge under the symbol, when the setting says.
    pub show_value: Option<bool>,
    /// The state text under a state-bound symbol.
    pub show_status: bool,
    /// The level's percentage on a vessel.
    pub show_percent: bool,
}

/// The library symbol that draws `widget`, or `None` for other widgets.
pub fn resolve(widget: &Widget) -> Option<Resolved> {
    let key = match &widget.kind {
        WidgetKind::Symbol => widget.text("symbol").unwrap_or("pump"),
        WidgetKind::Pump => "pump",
        WidgetKind::Motor => "motor",
        WidgetKind::Valve => "valve",
        WidgetKind::TankLevel => "tank",
        WidgetKind::HorizontalTank => "tank_horizontal",
        WidgetKind::Pipe => {
            if widget.text("direction") == Some("vertical") {
                "pipe_vertical"
            } else {
                "pipe"
            }
        }
        _ => return None,
    };
    let def = symbol_def(key).unwrap_or(&SYMBOLS[0]);
    let legacy_drive = matches!(
        widget.kind,
        WidgetKind::Pump | WidgetKind::Motor | WidgetKind::Valve
    );
    let legacy_vessel = matches!(
        widget.kind,
        WidgetKind::TankLevel | WidgetKind::HorizontalTank
    );
    Some(Resolved {
        def,
        default_writable: legacy_drive,
        // Legacy "showValue" meant the running/stopped text: no badge.
        show_value: if legacy_drive {
            Some(false)
        } else {
            widget.flag("showValue")
        },
        show_status: if legacy_drive {
            widget
                .flag("showStatus")
                .or_else(|| widget.flag("showValue"))
                .unwrap_or(true)
        } else {
            widget.flag("showStatus").unwrap_or(false)
        },
        // Hidden only by an explicit `false` (`showPercent === false`).
        show_percent: if legacy_vessel {
            widget
                .get("showPercent")
                .or_else(|| widget.get("showPercentage"))
                != Some(&Value::Bool(false))
        } else {
            widget.get("showPercent") != Some(&Value::Bool(false))
        },
    })
}

impl Resolved {
    /// The rotation, a whole number of quarter turns clockwise.
    pub fn quarter_turns(widget: &Widget) -> u8 {
        quarter_turns(widget.number("rotation").unwrap_or(0.0))
    }
}

/// `degrees` as quarter turns, 0 to 3.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn quarter_turns(degrees: f64) -> u8 {
    ((degrees / 90.0).round().rem_euclid(4.0)) as u8
}

/// The rows under a symbol: label, value badge, state text. Fixed heights,
/// since the connection ports are computed from them.
pub mod footer {
    pub const PAD: f64 = 2.0;
    pub const GAP: f64 = 2.0;
    pub const LABEL: f64 = 15.0;
    pub const BADGE: f64 = 19.0;
    pub const STATUS: f64 = 12.0;
}

/// Which footer rows a symbol shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FooterRows {
    pub label: bool,
    pub value: bool,
    pub status: bool,
}

pub fn footer_rows(widget: &Widget, resolved: &Resolved) -> FooterRows {
    let bound = widget.tag().is_some();
    let binding = resolved.def.binding;
    FooterRows {
        label: widget.flag("showLabel") != Some(false)
            && widget.text("label").is_some_and(|label| !label.is_empty()),
        value: bound
            && resolved
                .show_value
                .unwrap_or(matches!(binding, Binding::Level | Binding::Value)),
        status: binding == Binding::State && resolved.show_status,
    }
}

pub fn footer_height(rows: FooterRows) -> f64 {
    let heights: Vec<f64> = [
        (rows.label, footer::LABEL),
        (rows.value, footer::BADGE),
        (rows.status, footer::STATUS),
    ]
    .into_iter()
    .filter_map(|(shown, height)| shown.then_some(height))
    .collect();
    if heights.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let gaps = footer::GAP * (heights.len() - 1) as f64;
    footer::PAD + heights.iter().sum::<f64>() + gaps
}

/// Where a symbol's drawing sits in its widget: the view box fitted (or
/// stretched) into the box above the footer, then mirrored and turned about
/// that area's centre. Coordinates are relative to the widget's corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub scale_x: f64,
    pub scale_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// The drawing area's size (the box less the footer).
    pub width: f64,
    pub height: f64,
    pub quarter_turns: u8,
    pub flip: bool,
}

impl Placement {
    pub fn of(widget: &Widget, resolved: &Resolved) -> Self {
        let (vw, vh) = resolved.def.view_box;
        let height = (widget.height - footer_height(footer_rows(widget, resolved))).max(1.0);
        let width = widget.width;
        let (scale_x, scale_y, offset_x, offset_y) = if resolved.def.stretch {
            (width / vw, height / vh, 0.0, 0.0)
        } else {
            let scale = (width / vw).min(height / vh);
            (
                scale,
                scale,
                (width - vw * scale) / 2.0,
                (height - vh * scale) / 2.0,
            )
        };
        Self {
            scale_x,
            scale_y,
            offset_x,
            offset_y,
            width,
            height,
            quarter_turns: Resolved::quarter_turns(widget),
            flip: widget.flag("flipX").unwrap_or(false),
        }
    }

    /// A view box point, relative to the widget's corner.
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let mut x = self.offset_x + x * self.scale_x;
        let mut y = self.offset_y + y * self.scale_y;
        let (cx, cy) = (self.width / 2.0, self.height / 2.0);
        if self.flip {
            x = 2.0 * cx - x;
        }
        for _ in 0..self.quarter_turns {
            let (dx, dy) = (x - cx, y - cy);
            x = cx - dy;
            y = cy + dx;
        }
        (x, y)
    }
}

/// The view box side that ends up facing `side` of the widget after the
/// mirror and the turns.
fn local_side(side: Side, turns: u8, flip: bool) -> Side {
    let mut local = side;
    for _ in 0..turns {
        // Undo one clockwise turn.
        local = Side::ALL
            .into_iter()
            .find(|candidate| candidate.clockwise() == local)
            .unwrap_or(local);
    }
    if flip { local.mirrored() } else { local }
}

/// The stage point where a connection attaches to `side` of a widget,
/// following the drawn symbol's nozzles (`portPoint`).
pub fn port_point(widget: &Widget, side: Side) -> (f64, f64) {
    let Some(resolved) = resolve(widget) else {
        return match side {
            Side::Left => (widget.x, widget.y + widget.height / 2.0),
            Side::Right => (widget.x + widget.width, widget.y + widget.height / 2.0),
            Side::Top => (widget.x + widget.width / 2.0, widget.y),
            Side::Bottom => (widget.x + widget.width / 2.0, widget.y + widget.height),
        };
    };
    let placement = Placement::of(widget, &resolved);
    let local = local_side(side, placement.quarter_turns, placement.flip);
    let (vw, vh) = resolved.def.view_box;
    let (px, py) = resolved.def.ports.get(local).unwrap_or(match local {
        Side::Left => (0.0, vh / 2.0),
        Side::Right => (vw, vh / 2.0),
        Side::Top => (vw / 2.0, 0.0),
        Side::Bottom => (vw / 2.0, vh),
    });
    let (x, y) = placement.map(px, py);
    let round = |v: f64| (v * 10.0).round() / 10.0;
    (widget.x + round(x), widget.y + round(y))
}

// ── Connections (`connections/ConnectionLayer.tsx`) ──────────────────────

type Pt = (f64, f64);

/// The side of `widget` facing `toward`, weighted by the box's aspect.
fn facing_side(widget: &Widget, toward: Pt) -> Side {
    let (cx, cy) = widget.center();
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    if dx.abs() * widget.height >= dy.abs() * widget.width {
        if dx >= 0.0 { Side::Right } else { Side::Left }
    } else if dy >= 0.0 {
        Side::Bottom
    } else {
        Side::Top
    }
}

/// The sides a symbol has nozzles on, on the stage; plain widgets connect
/// anywhere.
fn nozzle_sides(widget: &Widget) -> Vec<Side> {
    let Some(resolved) = resolve(widget) else {
        return Side::ALL.to_vec();
    };
    let ports = resolved.def.ports;
    if resolved.def.stretch || !ports.any() {
        return Side::ALL.to_vec();
    }
    let turns = Resolved::quarter_turns(widget);
    let flip = widget.flag("flipX").unwrap_or(false);
    ports
        .sides()
        .into_iter()
        .map(|side| {
            let mut side = if flip { side.mirrored() } else { side };
            for _ in 0..turns {
                side = side.clockwise();
            }
            side
        })
        .collect()
}

/// The nozzle on `widget` toward `other`: the preferred axis first, and only
/// sides facing the other end.
fn pick_side(widget: &Widget, other: &Widget, horizontal_first: bool) -> Side {
    let (cx, cy) = widget.center();
    let (ox, oy) = other.center();
    let h = if ox >= cx { Side::Right } else { Side::Left };
    let v = if oy >= cy { Side::Bottom } else { Side::Top };
    let available = nozzle_sides(widget);
    let order = if horizontal_first { [h, v] } else { [v, h] };
    order
        .into_iter()
        .find(|side| available.contains(side))
        .unwrap_or(order[0])
}

/// A stretched symbol (busbar, straight pipe): tapped anywhere along it.
fn is_run(widget: &Widget) -> bool {
    resolve(widget).is_some_and(|resolved| resolved.def.stretch)
}

/// The point on a run's long edge nearest `toward`.
fn tap_point(widget: &Widget, toward: Pt) -> Pt {
    if widget.width >= widget.height {
        let x = toward.0.clamp(widget.x, widget.x + widget.width);
        let y = if toward.1 >= widget.y + widget.height / 2.0 {
            widget.y + widget.height
        } else {
            widget.y
        };
        (x, y)
    } else {
        let y = toward.1.clamp(widget.y, widget.y + widget.height);
        let x = if toward.0 >= widget.x + widget.width / 2.0 {
            widget.x + widget.width
        } else {
            widget.x
        };
        (x, y)
    }
}

/// Drop the middle points of straight runs.
fn simplify(points: Vec<Pt>) -> Vec<Pt> {
    let last = points.len().saturating_sub(1);
    let mut kept = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        if index == 0 || index == last {
            kept.push(*point);
            continue;
        }
        let (a, c) = (points[index - 1], points[index + 1]);
        #[allow(clippy::float_cmp)]
        let straight = (a.0 == point.0 && point.0 == c.0) || (a.1 == point.1 && point.1 == c.1);
        if !straight {
            kept.push(*point);
        }
    }
    kept
}

/// The orthogonal route between two widgets' nozzles: straight, L or Z
/// shaped (`routeConnection`); through the waypoints when it has any.
pub fn route(a: &Widget, b: &Widget, waypoints: &[(f64, f64)]) -> Vec<Pt> {
    if let (Some(first), Some(last)) = (waypoints.first(), waypoints.last()) {
        let mut points = vec![port_point(a, facing_side(a, *first))];
        points.extend_from_slice(waypoints);
        points.push(port_point(b, facing_side(b, *last)));
        return points;
    }
    let (ax, ay) = a.center();
    let (bx, by) = b.center();
    let horizontal = (bx - ax).abs() >= (by - ay).abs();
    let sa = pick_side(a, b, horizontal);
    let sb = pick_side(b, a, horizontal);
    let mut pa = port_point(a, sa);
    let mut pb = port_point(b, sb);
    let (run_a, run_b) = (is_run(a), is_run(b));
    if run_a && !run_b {
        pa = tap_point(a, pb);
    } else if run_b && !run_a {
        pb = tap_point(b, pa);
    }
    if run_a != run_b && ((pa.0 - pb.0).abs() < 0.5 || (pa.1 - pb.1).abs() < 0.5) {
        return vec![pa, pb];
    }
    if sa.horizontal() && sb.horizontal() {
        if (pa.1 - pb.1).abs() < 0.5 {
            return vec![pa, pb];
        }
        let mx = f64::midpoint(pa.0, pb.0).round();
        return simplify(vec![pa, (mx, pa.1), (mx, pb.1), pb]);
    }
    if !sa.horizontal() && !sb.horizontal() {
        if (pa.0 - pb.0).abs() < 0.5 {
            return vec![pa, pb];
        }
        let my = f64::midpoint(pa.1, pb.1).round();
        return simplify(vec![pa, (pa.0, my), (pb.0, my), pb]);
    }
    // One horizontal nozzle, one vertical: a single elbow.
    simplify(if sa.horizontal() {
        vec![pa, (pb.0, pa.1), pb]
    } else {
        vec![pa, (pa.0, pb.1), pb]
    })
}

/// One piece of a route drawn with rounded corners (radius 10): a straight
/// line, or a quadratic curve around a corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    Line(Pt),
    Quad { control: Pt, to: Pt },
}

/// A route as a start point and segments with rounded corners
/// (`roundedPath`).
pub fn rounded(points: &[Pt], radius: f64) -> Option<(Pt, Vec<Segment>)> {
    if points.len() < 2 {
        return None;
    }
    let start = points[0];
    let mut segments = Vec::with_capacity(points.len() * 2);
    for index in 1..points.len() - 1 {
        let (px, py) = points[index - 1];
        let (x, y) = points[index];
        let (nx, ny) = points[index + 1];
        let l1 = (x - px).hypot(y - py);
        let l2 = (nx - x).hypot(ny - y);
        if l1 <= 0.0 || l2 <= 0.0 {
            segments.push(Segment::Line((x, y)));
            continue;
        }
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let a = (x - (x - px) / l1 * r, y - (y - py) / l1 * r);
        let b = (x + (nx - x) / l2 * r, y + (ny - y) / l2 * r);
        segments.push(Segment::Line(a));
        segments.push(Segment::Quad {
            control: (x, y),
            to: b,
        });
    }
    segments.push(Segment::Line(points[points.len() - 1]));
    Some((start, segments))
}

// ── Live state ────────────────────────────────────────────────────────────

/// How a tag's value stands against its alarm limits (the server's status).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Normal,
    Warning,
    Critical,
}

/// A bound tag as the mimic sees it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// `None` until a value arrives.
    pub value: Option<f64>,
    pub level: Level,
    pub min: f64,
    pub max: f64,
}

/// Where the live values come from: the console's store, a test's map.
pub trait Values {
    /// The tag, `None` when the console does not know it.
    fn reading(&self, tag: i64) -> Option<Reading>;

    fn value(&self, tag: i64) -> Option<f64> {
        self.reading(tag).and_then(|reading| reading.value)
    }
}

impl<F: Fn(i64) -> Option<Reading>> Values for F {
    fn reading(&self, tag: i64) -> Option<Reading> {
        self(tag)
    }
}

/// An equipment state (ISA-101: colour only for running, fault, stale).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EquipState {
    Idle,
    Running,
    Stopped,
    Fault,
    /// Bound, but no value yet.
    Stale,
}

fn tag_state(value: Option<f64>, threshold: f64) -> EquipState {
    match value {
        None => EquipState::Stale,
        Some(value) if value > threshold => EquipState::Running,
        Some(_) => EquipState::Stopped,
    }
}

/// A symbol's live state from its bound tags (`symbolLiveState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Live {
    pub state: EquipState,
    /// 0 to 1 for level-bound symbols, when known.
    pub level: Option<f64>,
    /// The run tag's state (agitator, rake, pipe flow).
    pub run_state: EquipState,
    /// The run tag alone, ignoring a fault: a tripped pump still running
    /// must offer Stop.
    pub running: bool,
    pub alarm: Option<Level>,
}

impl Live {
    /// Anything that moves continuously (impeller, agitator, flow).
    pub fn moving(&self) -> bool {
        self.state == EquipState::Running || self.run_state == EquipState::Running
    }

    /// Anything that blinks (fault, critical alarm).
    pub fn blinking(&self) -> bool {
        self.state == EquipState::Fault || self.alarm == Some(Level::Critical)
    }
}

/// The value above which a state-bound symbol runs.
pub fn threshold(widget: &Widget, def: &SymbolDef) -> f64 {
    widget
        .number("onThreshold")
        .unwrap_or(if def.binding == Binding::None {
            0.0
        } else {
            0.5
        })
}

pub fn symbol_live(widget: &Widget, def: &SymbolDef, values: &impl Values) -> Live {
    let tag = widget.tag();
    let reading = tag.and_then(|tag| values.reading(tag));
    let data = reading.and_then(|reading| reading.value);
    let bound = tag.is_some();
    let threshold = threshold(widget, def);
    let fault = widget
        .tag_setting("faultSensorId")
        .and_then(|tag| values.value(tag));
    let state = if fault.is_some_and(|value| value > 0.5) {
        EquipState::Fault
    } else if bound && def.binding == Binding::State {
        tag_state(data, threshold)
    } else if bound && data.is_none() {
        EquipState::Stale
    } else {
        EquipState::Idle
    };
    let level = match (def.binding, data) {
        (Binding::Level, Some(value)) => {
            let (min, max) = reading.map_or((0.0, 100.0), |r| (r.min, r.max));
            Some(fraction(value, min, max))
        }
        _ => None,
    };
    let run_state = match widget.tag_setting("runSensorId") {
        Some(run) => tag_state(values.value(run), 0.5),
        None if def.binding == Binding::None && bound => tag_state(data, threshold),
        None => EquipState::Idle,
    };
    let alarm = match (data, reading.map(|r| r.level)) {
        (Some(_), Some(Level::Critical)) => Some(Level::Critical),
        (Some(_), Some(Level::Warning)) => Some(Level::Warning),
        _ => None,
    };
    Live {
        state,
        level,
        run_state,
        running: bound && tag_state(data, threshold) == EquipState::Running,
        alarm,
    }
}

/// Whether a connection end counts as running: an explicit run tag, or the
/// primary tag of a state-bound symbol (`isRunning`).
pub fn is_running(widget: &Widget, values: &impl Values) -> bool {
    if let Some(run) = widget.tag_setting("runSensorId") {
        return values.value(run).unwrap_or(0.0) > 0.5;
    }
    let Some(resolved) = resolve(widget) else {
        return false;
    };
    if resolved.def.binding != Binding::State {
        return false;
    }
    let threshold = widget.number("onThreshold").unwrap_or(0.5);
    widget
        .tag()
        .and_then(|tag| values.value(tag))
        .is_some_and(|value| value > threshold)
}

/// A pipe flows when the equipment at either end runs.
pub fn flowing(connection: &Connection, layout: &Layout, values: &impl Values) -> bool {
    connection.line() == LineKind::Flow
        && [&connection.from, &connection.to]
            .into_iter()
            .filter_map(|id| layout.widget(id))
            .any(|widget| is_running(widget, values))
}

/// The words for a state: valves open, breakers close, rotating kit runs.
pub fn state_text(def: &SymbolDef, state: EquipState) -> &'static str {
    match (def.category, def.key, state) {
        (Category::Valves, _, EquipState::Running) => "OPEN",
        (Category::Valves, _, EquipState::Stopped) => "CLOSED",
        (_, "breaker", EquipState::Running) => "CLOSED",
        (_, "breaker", EquipState::Stopped) => "OPEN",
        (_, _, EquipState::Idle) => "—",
        (_, _, EquipState::Running) => "RUNNING",
        (_, _, EquipState::Stopped) => "STOPPED",
        (_, _, EquipState::Fault) => "FAULT",
        (_, _, EquipState::Stale) => "NO DATA",
    }
}

// ── Widget state ──────────────────────────────────────────────────────────

/// Where `value` sits between `min` and `max`, 0 to 1 (an empty range
/// counts as 1 wide, as the web client's `range || 1`).
pub fn fraction(value: f64, min: f64, max: f64) -> f64 {
    let range = if max - min == 0.0 { 1.0 } else { max - min };
    ((value - min) / range).clamp(0.0, 1.0)
}

/// Which colour band a gauge, bar or faceplate value falls in: at or past
/// the critical limit's fraction, at or past the warning one, or neither
/// (the web client's gauge, bar, PID faceplate).
pub fn zone(value: f64, min: f64, max: f64, warn: f64, crit: f64) -> Level {
    let at = fraction(value, min, max);
    if at >= fraction(crit, min, max) {
        Level::Critical
    } else if at >= fraction(warn, min, max) {
        Level::Warning
    } else {
        Level::Normal
    }
}

/// On above the threshold (on/off, toggle, status light on a digital tag).
pub fn is_on(value: Option<f64>, threshold: f64) -> bool {
    value.is_some_and(|value| value > threshold)
}

/// What a status light shows (`StatusLightWidget`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    /// No value: muted, no glow.
    Dark,
    /// A digital tag off.
    Off,
    On,
    Warning,
    Critical,
}

pub fn status_light(reading: Option<Reading>, digital: bool, threshold: f64) -> Light {
    let Some(reading) = reading else {
        return Light::Dark;
    };
    let Some(value) = reading.value else {
        return Light::Dark;
    };
    if digital {
        return if value > threshold {
            Light::On
        } else {
            Light::Off
        };
    }
    match reading.level {
        Level::Critical => Light::Critical,
        Level::Warning => Light::Warning,
        Level::Normal => Light::On,
    }
}

/// The tags a multi-tag widget shows (`data_table`, `annunciator`,
/// `bar_chart`, `pie_chart`, `multi_trend`): its bound tags, or the plant's
/// first ones when it has none. `all` is every tag in the plant's order.
/// The data table keeps the binding order; the others, like the web
/// client, the plant's.
pub fn shown_tags(widget: &Widget, all: &[i64]) -> Vec<i64> {
    let count = |key: &str, default: f64| -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = widget.number(key).unwrap_or(default).max(0.0) as usize;
        n
    };
    let (limit, binding_order) = match widget.kind {
        WidgetKind::DataTable => (usize::MAX, true),
        WidgetKind::Annunciator => (count("maxItems", 12.0), false),
        WidgetKind::BarChart => (count("maxBars", 8.0), false),
        WidgetKind::PieChart => (count("maxSlices", 8.0), false),
        WidgetKind::MultiTrend => (count("maxTags", 4.0), false),
        _ => return widget.tags.clone(),
    };
    let bound: Vec<i64> = if binding_order {
        widget
            .tags
            .iter()
            .copied()
            .filter(|tag| all.contains(tag))
            .collect()
    } else {
        all.iter()
            .copied()
            .filter(|tag| widget.tags.contains(tag))
            .collect()
    };
    let fallback = if widget.kind == WidgetKind::DataTable {
        6
    } else {
        limit
    };
    if bound.is_empty() {
        all.iter().copied().take(fallback).collect()
    } else {
        bound.into_iter().take(limit).collect()
    }
}

// ── The faceplate (`hmi/Faceplate.tsx`) ───────────────────────────────────

/// An armed command waits this long for its confirmation.
pub const ARM_TIMEOUT: Duration = Duration::from_secs(6);

/// How the console sends an operator command: an admin over the
/// WebSocket, other operators through the REST write (policy-checked and
/// recorded); viewers cannot.
pub fn write_route(admin: bool, operator: bool) -> Option<WriteMode> {
    WriteMode::allowed(admin, operator)
        .iter()
        .copied()
        .find(|mode| *mode == WriteMode::Direct || *mode == WriteMode::Immediate)
}

/// One command button: its word and the value it writes.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: f64,
    pub enabled: bool,
}

/// A setpoint the faceplate adjusts in steps before arming it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Setpoint {
    pub step: f64,
    pub min: f64,
    pub max: f64,
}

impl Setpoint {
    /// `value` moved by `steps` steps, kept in range.
    pub fn stepped(&self, value: f64, steps: f64) -> f64 {
        let next = value + steps * self.step;
        if self.min < self.max {
            next.clamp(self.min, self.max)
        } else {
            next
        }
    }
}

/// What a faceplate offers to command.
#[derive(Debug, Clone, PartialEq)]
pub struct Commands {
    pub on: Choice,
    pub off: Choice,
    pub setpoint: Option<Setpoint>,
}

/// The commands for `widget`, if it takes any: a state-bound symbol that
/// opted in (legacy pumps, valves and motors by default) gets START/STOP
/// (OPEN/CLOSE for valves), enabled by its run feedback; an operator
/// control gets its own words and values. Nothing until the tag has a
/// value, as on the web.
pub fn commands(widget: &Widget, values: &impl Values) -> Option<Commands> {
    let tag = widget.tag()?;
    let reading = values.reading(tag)?;
    let value = reading.value?;
    if let Some(resolved) = resolve(widget) {
        let def = resolved.def;
        let writable = def.binding == Binding::State
            && widget.flag("writable").unwrap_or(resolved.default_writable);
        if !writable {
            return None;
        }
        let running = value > threshold(widget, def);
        let (on, off) = if def.category == Category::Valves {
            ("OPEN", "CLOSE")
        } else {
            ("START", "STOP")
        };
        return Some(Commands {
            on: Choice {
                label: on.to_owned(),
                value: 1.0,
                enabled: !running,
            },
            off: Choice {
                label: off.to_owned(),
                value: 0.0,
                enabled: running,
            },
            setpoint: None,
        });
    }
    let word = |key: &str, default: &str| widget.text(key).unwrap_or(default).to_owned();
    let on_value = widget.number("onValue").unwrap_or(1.0);
    let off_value = widget.number("offValue").unwrap_or(0.0);
    match widget.kind {
        WidgetKind::Toggle => {
            let on = is_on(Some(value), widget.number("onThreshold").unwrap_or(0.5));
            Some(Commands {
                on: Choice {
                    label: word("onLabel", "ON"),
                    value: on_value,
                    enabled: !on,
                },
                off: Choice {
                    label: word("offLabel", "OFF"),
                    value: off_value,
                    enabled: on,
                },
                setpoint: None,
            })
        }
        WidgetKind::PushButton => {
            #[allow(clippy::float_cmp)]
            let active = value == on_value;
            Some(Commands {
                on: Choice {
                    label: word("onLabel", "START"),
                    value: on_value,
                    enabled: !active,
                },
                off: Choice {
                    label: word("offLabel", "STOP"),
                    value: off_value,
                    enabled: active,
                },
                setpoint: None,
            })
        }
        WidgetKind::Slider | WidgetKind::NumberInput => Some(Commands {
            on: Choice {
                label: "SET".to_owned(),
                value,
                enabled: true,
            },
            off: Choice {
                label: "SET".to_owned(),
                value,
                enabled: false,
            },
            setpoint: Some(Setpoint {
                step: widget
                    .number("step")
                    .filter(|step| *step > 0.0)
                    .unwrap_or(1.0),
                min: reading.min,
                max: reading.max,
            }),
        }),
        _ => None,
    }
}

/// A command waiting for its confirmation.
#[derive(Debug, Clone, PartialEq)]
pub struct Armed {
    /// The widget it was armed on.
    pub widget: String,
    pub tag: i64,
    pub label: String,
    pub value: f64,
    pub at: Instant,
}

impl Armed {
    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.at) >= ARM_TIMEOUT
    }

    /// How long it still waits.
    pub fn left(&self, now: Instant) -> Duration {
        ARM_TIMEOUT.saturating_sub(now.duration_since(self.at))
    }
}

/// A faceplate's related tag row: label and tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    pub label: &'static str,
    pub tag: i64,
}

/// The related tags a symbol's faceplate lists: its run feedback, its run
/// tag and its fault tag.
pub fn related(widget: &Widget) -> Vec<Related> {
    let mut rows = Vec::new();
    let Some(resolved) = resolve(widget) else {
        return rows;
    };
    let valve = resolved.def.category == Category::Valves;
    if resolved.def.binding == Binding::State
        && let Some(tag) = widget.tag()
    {
        rows.push(Related {
            label: if valve {
                "Open feedback"
            } else {
                "Run feedback"
            },
            tag,
        });
    }
    if let Some(tag) = widget.tag_setting("runSensorId") {
        rows.push(Related {
            label: if resolved.def.key == "gravity_filter" {
                "Backwash"
            } else {
                "Run"
            },
            tag,
        });
    }
    if let Some(tag) = widget.tag_setting("faultSensorId") {
        rows.push(Related {
            label: "Fault",
            tag,
        });
    }
    rows
}

/// What a related tag's value reads as.
pub fn related_text(label: &str, on: bool) -> &'static str {
    let (yes, no) = match label {
        "Fault" => ("TRIPPED", "HEALTHY"),
        "Run feedback" | "Run" => ("RUNNING", "STOPPED"),
        "Open feedback" => ("OPEN", "CLOSED"),
        "Backwash" => ("IN PROGRESS", "IDLE"),
        _ => ("ON", "OFF"),
    };
    if on { yes } else { no }
}

/// A limit counts when it sits inside the tag's range; 0 means "not
/// configured".
pub fn limit(value: f64, min: f64, max: f64) -> Option<f64> {
    (value != 0.0 && value > min && value <= max).then_some(value)
}

/// The low warning and critical limits in a tag's `source_config`.
pub fn low_limits(source_config: &str) -> (Option<f64>, Option<f64>) {
    let config: Value = serde_json::from_str(source_config).unwrap_or(Value::Null);
    let get = |key: &str| config.get(key).and_then(Value::as_f64);
    (get("low_warn"), get("low_crit"))
}

// ── The alarm banner (`AlarmBanner.tsx`) ──────────────────────────────────

/// The rows the banner shows.
pub const BANNER_ROWS: usize = 3;

/// The banner's alarms: active (not cleared, not shelved), unacknowledged
/// first, critical before warning, newest first; and their counts.
#[derive(Debug, Clone, Default)]
pub struct Banner<'a> {
    pub alarms: Vec<&'a Alarm>,
    pub critical: usize,
    pub warning: usize,
    pub unacknowledged: usize,
}

pub fn banner(alarms: &[Alarm]) -> Banner<'_> {
    let mut active: Vec<&Alarm> = Vec::new();
    for alarm in alarms {
        if alarm.resolved_at.is_none()
            && alarm.shelved_until.is_none()
            && !active.iter().any(|seen| seen.id == alarm.id)
        {
            active.push(alarm);
        }
    }
    let rank = |alarm: &Alarm| {
        u8::from(alarm.acknowledged_at.is_none()) * 2 + u8::from(alarm.severity == "critical")
    };
    active.sort_by(|a, b| {
        rank(b)
            .cmp(&rank(a))
            .then(b.triggered_at.cmp(&a.triggered_at))
    });
    let critical = active.iter().filter(|a| a.severity == "critical").count();
    Banner {
        critical,
        warning: active.len() - critical,
        unacknowledged: active
            .iter()
            .filter(|a| a.acknowledged_at.is_none())
            .count(),
        alarms: active,
    }
}

// ── The camera ────────────────────────────────────────────────────────────

/// The zoom steps of ⌘+ and ⌘−.
pub const ZOOM_STEPS: [f64; 12] = [
    0.1, 0.25, 0.33, 0.5, 0.67, 0.75, 0.9, 1.0, 1.25, 1.5, 2.0, 3.0,
];
pub const MIN_ZOOM: f64 = 0.05;
pub const MAX_ZOOM: f64 = 6.0;

/// How the stage maps onto the viewport: a stage point `p` is drawn at
/// `offset + p * zoom` (viewport pixels from its corner).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub zoom: f64,
    pub x: f64,
    pub y: f64,
}

impl Camera {
    /// The whole stage, centred, `pad` pixels clear of the edges.
    pub fn fit(width: f64, height: f64, pad: f64) -> Self {
        let zoom = ((width - pad) / STAGE_W)
            .min((height - pad) / STAGE_H)
            .max(MIN_ZOOM);
        Self {
            zoom,
            x: (width - STAGE_W * zoom) / 2.0,
            y: (height - STAGE_H * zoom) / 2.0,
        }
    }

    pub fn to_stage(&self, x: f64, y: f64) -> (f64, f64) {
        ((x - self.x) / self.zoom, (y - self.y) / self.zoom)
    }

    pub fn to_view(&self, x: f64, y: f64) -> (f64, f64) {
        (self.x + x * self.zoom, self.y + y * self.zoom)
    }

    /// Zoomed to `zoom` (kept in range) about the viewport point `(ax, ay)`,
    /// which stays over the same stage point.
    #[must_use]
    pub fn zoomed(&self, zoom: f64, ax: f64, ay: f64) -> Self {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let (sx, sy) = self.to_stage(ax, ay);
        Self {
            zoom,
            x: ax - sx * zoom,
            y: ay - sy * zoom,
        }
    }

    /// The next zoom step up (`in`) or down from the current zoom.
    pub fn step(&self, zoom_in: bool) -> f64 {
        if zoom_in {
            ZOOM_STEPS
                .into_iter()
                .find(|step| *step > self.zoom + 0.001)
                .unwrap_or(MAX_ZOOM.min(self.zoom * 1.25))
        } else {
            ZOOM_STEPS
                .into_iter()
                .rev()
                .find(|step| *step < self.zoom - 0.001)
                .unwrap_or(MIN_ZOOM.max(self.zoom / 1.25))
        }
    }

    /// Panned by a viewport delta, keeping at least `keep` pixels of the
    /// stage on screen.
    #[must_use]
    pub fn panned(&self, dx: f64, dy: f64, width: f64, height: f64, keep: f64) -> Self {
        let (w, h) = (STAGE_W * self.zoom, STAGE_H * self.zoom);
        let clamp = |at: f64, size: f64, view: f64| {
            let keep = keep.min(size).min(view);
            at.clamp(keep - size, view - keep)
        };
        Self {
            zoom: self.zoom,
            x: clamp(self.x + dx, w, width),
            y: clamp(self.y + dy, h, height),
        }
    }

    /// The least pan that shows the stage rectangle (with `margin` pixels)
    /// in a viewport, or `None` when it is in view already.
    pub fn reveal(
        &self,
        rect: (f64, f64, f64, f64),
        width: f64,
        height: f64,
        margin: f64,
    ) -> Option<Self> {
        let (x0, y0) = self.to_view(rect.0, rect.1);
        let (x1, y1) = self.to_view(rect.0 + rect.2, rect.1 + rect.3);
        let shift = |lo: f64, hi: f64, view: f64| {
            if hi - lo > view - 2.0 * margin {
                // Bigger than the view: centre it.
                view / 2.0 - f64::midpoint(lo, hi)
            } else if lo < margin {
                margin - lo
            } else if hi > view - margin {
                view - margin - hi
            } else {
                0.0
            }
        };
        let (dx, dy) = (shift(x0, x1, width), shift(y0, y1, height));
        (dx != 0.0 || dy != 0.0).then_some(Self {
            zoom: self.zoom,
            x: self.x + dx,
            y: self.y + dy,
        })
    }

    /// Part of the way from `self` to `to`, 0 to 1.
    #[must_use]
    pub fn toward(&self, to: &Self, t: f64) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            zoom: self.zoom + (to.zoom - self.zoom) * t,
            x: self.x + (to.x - self.x) * t,
            y: self.y + (to.y - self.y) * t,
        }
    }
}

// ── Keyboard order ────────────────────────────────────────────────────────

/// The widgets the keyboard visits (those with a faceplate and a tag), in
/// reading order: rows of about 80 stage pixels, left to right.
pub fn tab_order(layout: &Layout) -> Vec<usize> {
    let mut order: Vec<usize> = layout
        .widgets
        .iter()
        .enumerate()
        .filter(|(_, widget)| widget.kind.has_faceplate() && widget.tag().is_some())
        .map(|(index, _)| index)
        .collect();
    #[allow(clippy::cast_possible_truncation)]
    let key = |index: &usize| {
        let (cx, cy) = layout.widgets[*index].center();
        ((cy / 80.0).floor() as i64, cx.round() as i64)
    };
    order.sort_by_key(key);
    order
}

/// The direction an arrow key moves the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// The nearest of `candidates` from widget `from` in `direction`: ahead of
/// it, weighing sideways distance double.
pub fn neighbor(
    layout: &Layout,
    from: usize,
    direction: Direction,
    candidates: &[usize],
) -> Option<usize> {
    let (fx, fy) = layout.widgets.get(from)?.center();
    candidates
        .iter()
        .copied()
        .filter(|index| *index != from)
        .filter_map(|index| {
            let (x, y) = layout.widgets[index].center();
            let (ahead, aside) = match direction {
                Direction::Up => (fy - y, (x - fx).abs()),
                Direction::Down => (y - fy, (x - fx).abs()),
                Direction::Left => (fx - x, (y - fy).abs()),
                Direction::Right => (x - fx, (y - fy).abs()),
            };
            (ahead > 1.0).then_some((index, ahead + 2.0 * aside))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

pub mod edit;

#[cfg(test)]
mod tests;
