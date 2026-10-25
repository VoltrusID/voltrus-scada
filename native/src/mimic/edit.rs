//! The mimic editor's rules, with no UI: what the web client's SCADA editor
//! does (`ScadaView.tsx`, `engine/widgetState.ts`, `WidgetShell.tsx`,
//! `ScadaPalette.tsx`, `ScadaInspector.tsx`), on the plain [`Layout`].
//!
//! - **What can be placed**: [`MANIFESTS`] (`widgets/registry.ts`: display
//!   names, default sizes, bindings, the inspector's `configSchema`) and
//!   the library's symbols ([`SYMBOLS`], [`symbol_fields`] after
//!   `symbolSchema` in `SymbolWidget.tsx`), as a [`Placing`].
//! - **Edits**: [`Edit`] (the reducer's actions) applied by a [`History`]
//!   with undo and redo, consecutive edits of one field or one nudge run
//!   coalesced into one step, at most [`HISTORY_LIMIT`] steps.
//! - **Geometry**: the 10-pixel grid ([`snap`]), keeping boxes on the
//!   stage ([`clamp_pos`]), click-to-add's cascade ([`place_at`]), moves
//!   and resizes that snap to the grid or line up with other widgets
//!   ([`snap_move`], [`snap_resize`], with [`Guide`]s to draw), hit tests
//!   for widgets, resize [`Handle`]s and connections, the rubber band
//!   ([`enclosed`]), z-order, copy, paste and duplicate ([`Clip`]) with
//!   fresh `w_<ms>_<rand>` ids ([`fresh_id`]).
//! - **Saving**: [`named`] fills `equipment_name` as the web's `toAPI`
//!   does; [`super::to_rows`] makes the `PUT` bodies.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use super::{
    Binding, Category, Connection, Layout, STAGE_H, STAGE_W, SYMBOLS, SymbolDef, Widget,
    WidgetKind, quarter_turns, resolve, route, symbol_def,
};

/// Positions and sizes snap to this (`GRID` in `ScadaCanvas.tsx`).
pub const GRID: f64 = 10.0;
/// The editing stage shows a dot every this many stage pixels.
pub const DOTS: f64 = 20.0;
/// Paste and duplicate put the copy this far right and down.
pub const PASTE_OFFSET: f64 = 20.0;
/// Undo steps kept.
pub const HISTORY_LIMIT: usize = 100;
/// The smallest box a resize leaves (`minWidth` / `minHeight`).
pub const MIN_WIDTH: f64 = 16.0;
pub const MIN_HEIGHT: f64 = 10.0;

/// The connection types new connections take (`CONNECTION_TYPES`).
pub const LINE_TYPES: [(&str, &str); 3] = [
    ("flow", "Process pipe"),
    ("power", "Power line"),
    ("data", "Signal / data"),
];

// ── The inspector's fields ────────────────────────────────────────────────

/// A field's default (`ConfigFieldDef.default`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Init {
    None,
    Text(&'static str),
    Number(f64),
    Flag(bool),
}

impl Init {
    pub fn value(self) -> Value {
        match self {
            Self::None => Value::Null,
            Self::Text(text) => Value::from(text),
            Self::Number(number) => Value::from(number),
            Self::Flag(flag) => Value::Bool(flag),
        }
    }
}

/// What a field edits (`ConfigFieldType`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FieldKind {
    Text,
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
    Flag,
    /// Values and their labels.
    Select(&'static [(&'static str, &'static str)]),
    /// A colour: a theme variable (`var(--ok)`) or a literal.
    Color,
    /// A data source id.
    Tag,
}

/// One inspector field (`ConfigFieldDef`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Field {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub init: Init,
}

const fn text(key: &'static str, label: &'static str, init: &'static str) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Text,
        init: Init::Text(init),
    }
}

const fn number(key: &'static str, label: &'static str, init: f64) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Number {
            min: None,
            max: None,
        },
        init: Init::Number(init),
    }
}

const fn ranged(key: &'static str, label: &'static str, init: f64, min: f64, max: f64) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Number {
            min: Some(min),
            max: Some(max),
        },
        init: Init::Number(init),
    }
}

const fn flag(key: &'static str, label: &'static str, init: bool) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Flag,
        init: Init::Flag(init),
    }
}

const fn color(key: &'static str, label: &'static str, init: &'static str) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Color,
        init: Init::Text(init),
    }
}

const fn select(
    key: &'static str,
    label: &'static str,
    init: &'static str,
    options: &'static [(&'static str, &'static str)],
) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Select(options),
        init: Init::Text(init),
    }
}

const fn tag(key: &'static str, label: &'static str) -> Field {
    Field {
        key,
        label,
        kind: FieldKind::Tag,
        init: Init::None,
    }
}

/// The colours a colour field offers besides typing one: the theme
/// variables the widgets draw with.
pub const COLORS: [&str; 6] = [
    "var(--ok)",
    "var(--cyan)",
    "var(--warn)",
    "var(--crit)",
    "var(--muted)",
    "var(--hmi-run)",
];

// ── The widget types (`widgets/registry.ts`) ─────────────────────────────

/// A widget type as the palette and the inspector know it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Manifest {
    /// The wire name (`equipment_type`).
    pub kind: &'static str,
    pub name: &'static str,
    /// The size a new one is placed at.
    pub size: (f64, f64),
    /// How many tags it binds: 0, 1, or a list.
    pub max_tags: usize,
    /// The kinds of tag it shows, as the palette lists them.
    pub data: &'static str,
    pub fields: &'static [Field],
}

const ANALOG: &str = "analog";
const BOTH: &str = "analog · digital";
const ANY: &str = "any";

/// Every widget type, in [`WidgetKind::ALL`]'s order.
pub static MANIFESTS: &[Manifest] = &[
    Manifest {
        kind: "symbol",
        name: "Symbol",
        size: (90.0, 90.0),
        max_tags: 1,
        data: "",
        fields: &[],
    },
    Manifest {
        kind: "area",
        name: "Process Area",
        size: (480.0, 320.0),
        max_tags: 0,
        data: "",
        fields: &[
            text("title", "Title", "Process Area"),
            text("subtitle", "Subtitle", ""),
        ],
    },
    Manifest {
        kind: "gauge",
        name: "Gauge",
        size: (180.0, 180.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            flag("showLabels", "Show Labels", true),
            color("arcColor", "Arc Color", "var(--ok)"),
        ],
    },
    Manifest {
        kind: "indicator",
        name: "Indicator",
        size: (140.0, 80.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            ranged("fontSize", "Font Size", 28.0, 12.0, 72.0),
            flag("showUnit", "Show Unit", true),
            flag("showStatus", "Show Status", true),
        ],
    },
    Manifest {
        kind: "bar",
        name: "Bar",
        size: (200.0, 60.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            color("fillColor", "Fill Color", "var(--ok)"),
            flag("showValue", "Show Value", true),
        ],
    },
    Manifest {
        kind: "status_light",
        name: "Status Light",
        size: (60.0, 60.0),
        max_tags: 1,
        data: BOTH,
        fields: &[number("onThreshold", "On Threshold", 0.5)],
    },
    Manifest {
        kind: "tank_level",
        name: "Tank Level",
        size: (100.0, 160.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[flag("showPercentage", "Show %", true)],
    },
    Manifest {
        kind: "on_off",
        name: "On/Off",
        size: (100.0, 60.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            number("onThreshold", "On Threshold", 0.5),
            text("onLabel", "On Label", "ON"),
            text("offLabel", "Off Label", "OFF"),
        ],
    },
    Manifest {
        kind: "label",
        name: "Label",
        size: (160.0, 40.0),
        max_tags: 99,
        data: ANY,
        fields: &[
            text("text", "Text", "Label"),
            ranged("fontSize", "Font Size", 14.0, 8.0, 48.0),
            select(
                "align",
                "Align",
                "center",
                &[("left", "Left"), ("center", "Center"), ("right", "Right")],
            ),
            flag("bold", "Bold", false),
            flag("muted", "Muted", false),
        ],
    },
    Manifest {
        kind: "trend",
        name: "Trend",
        size: (240.0, 100.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            color("lineColor", "Line Color", "var(--cyan)"),
            ranged("pointsCount", "Points", 180.0, 10.0, 180.0),
            flag("showName", "Show Tag Name", true),
        ],
    },
    Manifest {
        kind: "big_number",
        name: "Big Number",
        size: (180.0, 120.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            ranged("fontSize", "Font Size", 48.0, 20.0, 120.0),
            flag("showUnit", "Show Unit", true),
            flag("showName", "Show Name", true),
            ranged("decimals", "Decimals", 1.0, 0.0, 6.0),
        ],
    },
    Manifest {
        kind: "data_table",
        name: "Data Table",
        size: (320.0, 200.0),
        max_tags: 99,
        data: ANY,
        fields: &[
            flag("showStatus", "Show Status", true),
            flag("compact", "Compact", false),
        ],
    },
    Manifest {
        kind: "pipe",
        name: "Pipe",
        size: (200.0, 30.0),
        max_tags: 1,
        data: ANY,
        fields: &[select(
            "direction",
            "Direction",
            "horizontal",
            &[("horizontal", "Horizontal"), ("vertical", "Vertical")],
        )],
    },
    Manifest {
        kind: "slider",
        name: "Slider",
        size: (220.0, 80.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            flag("showValue", "Show Value", true),
            ranged("step", "Step", 1.0, 0.01, 1000.0),
            color("sliderColor", "Slider Color", "var(--cyan)"),
        ],
    },
    Manifest {
        kind: "number_input",
        name: "Number Input",
        size: (180.0, 80.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            flag("showButtons", "Show +/- Buttons", true),
            ranged("step", "Step", 1.0, 0.01, 1000.0),
        ],
    },
    Manifest {
        kind: "toggle",
        name: "Toggle",
        size: (120.0, 80.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            number("onThreshold", "On Threshold", 0.5),
            number("onValue", "Write On Value", 1.0),
            number("offValue", "Write Off Value", 0.0),
            text("onLabel", "On Label", "ON"),
            text("offLabel", "Off Label", "OFF"),
        ],
    },
    Manifest {
        kind: "meter",
        name: "Meter",
        size: (200.0, 100.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            color("fillColor", "Fill Color", "var(--ok)"),
            flag("showTicks", "Show Ticks", true),
        ],
    },
    Manifest {
        kind: "semicircle_gauge",
        name: "Half Gauge",
        size: (200.0, 120.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            flag("showTicks", "Show Ticks", true),
            ranged("thickness", "Arc Width", 14.0, 4.0, 30.0),
        ],
    },
    Manifest {
        kind: "donut",
        name: "Donut",
        size: (160.0, 160.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            color("fillColor", "Fill Color", "var(--cyan)"),
            ranged("thickness", "Ring Width", 14.0, 4.0, 30.0),
            flag("showPercentage", "Show %", true),
            flag("showName", "Show Name", true),
        ],
    },
    Manifest {
        kind: "led_display",
        name: "LED Display",
        size: (200.0, 80.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            color("color", "LED Color", "var(--cyan)"),
            flag("showName", "Show Name", true),
        ],
    },
    Manifest {
        kind: "motor",
        name: "Motor",
        size: (100.0, 100.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            number("onThreshold", "On Threshold", 0.5),
            flag("showValue", "Show Status", true),
        ],
    },
    Manifest {
        kind: "valve",
        name: "Valve",
        size: (100.0, 80.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            number("onThreshold", "Open Threshold", 0.5),
            flag("showValue", "Show Status", true),
            color("failColor", "Fault Color", "var(--crit)"),
        ],
    },
    Manifest {
        kind: "pump",
        name: "Pump",
        size: (100.0, 100.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            number("onThreshold", "On Threshold", 0.5),
            flag("showValue", "Show Status", true),
        ],
    },
    Manifest {
        kind: "push_button",
        name: "Push Button",
        size: (100.0, 100.0),
        max_tags: 1,
        data: BOTH,
        fields: &[
            select(
                "mode",
                "Mode",
                "maintained",
                &[
                    ("maintained", "Maintained (Latching)"),
                    ("momentary", "Momentary"),
                ],
            ),
            text("onLabel", "On Label", "START"),
            text("offLabel", "Off Label", "STOP"),
            number("onValue", "On Value", 1.0),
            number("offValue", "Off Value", 0.0),
            color("buttonColor", "Active Color", "var(--ok)"),
            flag("showLabel", "Show Label", true),
        ],
    },
    Manifest {
        kind: "bar_chart",
        name: "Bar Chart",
        size: (300.0, 180.0),
        max_tags: 99,
        data: ANALOG,
        fields: &[
            ranged("maxBars", "Max Bars", 8.0, 2.0, 16.0),
            flag("showValues", "Show Values", true),
        ],
    },
    Manifest {
        kind: "pie_chart",
        name: "Pie Chart",
        size: (240.0, 200.0),
        max_tags: 99,
        data: ANALOG,
        fields: &[ranged("maxSlices", "Max Slices", 8.0, 2.0, 12.0)],
    },
    Manifest {
        kind: "multi_trend",
        name: "Multi-Trend",
        size: (320.0, 140.0),
        max_tags: 99,
        data: ANALOG,
        fields: &[
            ranged("maxTags", "Max Tags", 4.0, 1.0, 8.0),
            ranged("pointsCount", "Points", 60.0, 10.0, 300.0),
        ],
    },
    Manifest {
        kind: "annunciator",
        name: "Annunciator",
        size: (280.0, 160.0),
        max_tags: 99,
        data: BOTH,
        fields: &[ranged("maxItems", "Max Items", 12.0, 4.0, 24.0)],
    },
    Manifest {
        kind: "pid_faceplate",
        name: "PID Faceplate",
        size: (180.0, 160.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            number("setpoint", "Setpoint", 50.0),
            number("output", "Output %", 0.0),
            select(
                "mode",
                "Mode",
                "AUTO",
                &[("AUTO", "Auto"), ("MAN", "Manual"), ("CAS", "Cascade")],
            ),
            number("pGain", "P Gain", 1.0),
            number("iGain", "I Gain", 0.1),
            number("dGain", "D Gain", 0.0),
            flag("showOutput", "Show Output", true),
            flag("showBars", "Show Bars", true),
        ],
    },
    Manifest {
        kind: "horizontal_tank",
        name: "H-Tank",
        size: (200.0, 120.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[flag("showPercentage", "Show %", true)],
    },
    Manifest {
        kind: "progress_bar",
        name: "Progress Bar",
        size: (200.0, 180.0),
        max_tags: 1,
        data: ANALOG,
        fields: &[
            color("fillColor", "Fill Color", "var(--ok)"),
            select(
                "direction",
                "Direction",
                "up",
                &[
                    ("up", "Up"),
                    ("down", "Down"),
                    ("right", "Right"),
                    ("left", "Left"),
                ],
            ),
            flag("showTicks", "Show Ticks", true),
        ],
    },
];

/// The manifest of a widget type, `None` for one this console does not
/// know.
pub fn manifest(kind: &WidgetKind) -> Option<&'static Manifest> {
    let wire = kind.as_str();
    MANIFESTS.iter().find(|manifest| manifest.kind == wire)
}

/// The widget types the palette's Widgets list offers: the library draws
/// the equipment types, offered as symbols instead (`SYMBOL_BACKED`).
pub fn palette_widgets() -> impl Iterator<Item = &'static Manifest> {
    MANIFESTS
        .iter()
        .filter(|manifest| !WidgetKind::parse(manifest.kind).is_equipment())
}

/// The library's categories in the palette's order (`SYMBOL_CATEGORIES`).
pub const CATEGORIES: [Category; 7] = [
    Category::Rotating,
    Category::Valves,
    Category::Vessels,
    Category::Process,
    Category::Electrical,
    Category::Instruments,
    Category::Piping,
];

pub fn category_name(category: Category) -> &'static str {
    match category {
        Category::Rotating => "Rotating",
        Category::Valves => "Valves",
        Category::Vessels => "Vessels",
        Category::Process => "Process",
        Category::Electrical => "Electrical",
        Category::Instruments => "Instruments",
        Category::Piping => "Piping",
    }
}

/// Case-insensitive substring search, as the palette filters: an empty
/// query matches everything.
pub fn matches(query: &str, texts: &[&str]) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || texts
            .iter()
            .any(|text| text.to_lowercase().contains(&query))
}

/// The symbols the palette lists for `query`: by name or category.
pub fn palette_symbols(query: &str) -> impl Iterator<Item = &'static SymbolDef> + '_ {
    SYMBOLS
        .iter()
        .filter(move |def| matches(query, &[def.name, category_name(def.category)]))
}

// ── The symbols' fields (`symbolSchema`, `hmi/symbols.tsx`) ──────────────

const VESSEL: &[Field] = &[flag("showPercent", "Show %", true)];
const MIXER: &[Field] = &[
    flag("showPercent", "Show %", true),
    tag("runSensorId", "Agitator Run Tag"),
];
const CLARIFIER: &[Field] = &[tag("runSensorId", "Rake Drive Run Tag")];
const GRAVITY_FILTER: &[Field] = &[tag("runSensorId", "Backwash Tag")];
const INSTRUMENT: &[Field] = &[
    text("tagFunction", "Function (e.g. PT)", "PT"),
    text("loopNumber", "Loop number", "101"),
    select(
        "mounting",
        "Mounting",
        "field",
        &[
            ("field", "Field"),
            ("panel", "Control room"),
            ("dcs", "DCS / shared display"),
        ],
    ),
];
const PIPE: &[Field] = &[tag("runSensorId", "Flow Tag (animates)")];
const ROTATIONS: &[(&str, &str)] = &[("0", "0°"), ("90", "90°"), ("180", "180°"), ("270", "270°")];

/// A symbol's own fields (`SymbolDef.fields`).
fn own_fields(key: &str) -> &'static [Field] {
    match key {
        "tank" | "tank_horizontal" | "silo" => VESSEL,
        "mixer" => MIXER,
        "clarifier" => CLARIFIER,
        "gravity_filter" => GRAVITY_FILTER,
        "instrument" => INSTRUMENT,
        "pipe" | "pipe_vertical" | "pipe_elbow" | "pipe_tee" => PIPE,
        _ => &[],
    }
}

/// The inspector's fields for a symbol: the label, the binding's, the
/// symbol's own, and its rotation and mirror unless it stretches.
/// `legacy`: a pump, motor or valve widget from before the library, which
/// has no value badge and takes commands unless turned off.
pub fn symbol_fields(def: &SymbolDef, legacy: bool, default_writable: bool) -> Vec<Field> {
    let mut fields = vec![text("label", "Label", "")];
    if !(legacy && def.binding == Binding::State) {
        fields.push(flag(
            "showValue",
            "Show Value",
            matches!(def.binding, Binding::Level | Binding::Value),
        ));
    }
    if def.binding == Binding::State {
        fields.extend([
            flag("showStatus", "Show Status Text", false),
            number("onThreshold", "Running Above", 0.5),
            tag("faultSensorId", "Fault Tag"),
            flag("writable", "Operator Start/Stop", default_writable),
        ]);
    }
    fields.extend_from_slice(own_fields(def.key));
    if !def.stretch {
        fields.extend([
            select("rotation", "Rotation", "0", ROTATIONS),
            flag("flipX", "Mirror", false),
        ]);
    }
    fields
}

/// The inspector's fields for `widget` (`WidgetInspector`'s schema).
pub fn schema(widget: &Widget) -> Vec<Field> {
    let manifest = manifest(&widget.kind);
    if let Some(resolved) = resolve(widget) {
        let mut fields = symbol_fields(
            resolved.def,
            widget.kind != WidgetKind::Symbol,
            resolved.default_writable,
        );
        // A legacy pipe keeps its orientation in its own setting.
        if widget.kind == WidgetKind::Pipe {
            fields.extend(
                manifest
                    .iter()
                    .flat_map(|manifest| manifest.fields.iter())
                    .filter(|field| field.key == "direction")
                    .copied(),
            );
        }
        return fields;
    }
    manifest.map_or_else(Vec::new, |manifest| manifest.fields.to_vec())
}

/// What the inspector shows for a field: the setting as drawn (legacy
/// widgets map their old keys, as `resolveSymbol` does), else its default.
pub fn field_value(widget: &Widget, field: &Field) -> Value {
    let legacy_drive = matches!(
        widget.kind,
        WidgetKind::Pump | WidgetKind::Motor | WidgetKind::Valve
    );
    let legacy_vessel = matches!(
        widget.kind,
        WidgetKind::TankLevel | WidgetKind::HorizontalTank
    );
    let shown = match field.key {
        "showValue" if legacy_drive => Some(Value::Bool(false)),
        "showStatus" if legacy_drive => Some(
            widget
                .get("showStatus")
                .or_else(|| widget.get("showValue"))
                .cloned()
                .unwrap_or(Value::Bool(true)),
        ),
        "showPercent" if legacy_vessel => Some(
            widget
                .get("showPercent")
                .or_else(|| widget.get("showPercentage"))
                .cloned()
                .unwrap_or(Value::Bool(true)),
        ),
        key => widget.get(key).cloned(),
    };
    shown.unwrap_or_else(|| field.init.value())
}

/// A setting as text for a field or a select: numbers without needless
/// decimals (`90`, not `90.0`), text as is, nothing for null.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        Value::Number(number) => match number.as_f64() {
            Some(n) if n.fract() == 0.0 && n.abs() < 1e15 => format!("{n:.0}"),
            Some(n) => n.to_string(),
            None => number.to_string(),
        },
        other => other.to_string(),
    }
}

/// JavaScript truthiness of a setting (a checkbox's `!!value`).
pub fn value_flag(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// A data source id in a setting (a tag field).
#[allow(clippy::cast_possible_truncation)]
pub fn value_tag(value: &Value) -> Option<i64> {
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

/// A number as the web client stores it: a whole one without a fraction
/// (`90`, not `90.0`).
#[allow(clippy::cast_possible_truncation)]
pub fn number_value(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9e15 {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

/// A number typed into a field, kept in the field's range; `None` when it
/// is not a number (the field then keeps its value, as the web's
/// `NumberInput` does).
pub fn typed_number(text: &str, kind: FieldKind) -> Option<f64> {
    let value: f64 = text.trim().parse().ok().filter(|n: &f64| n.is_finite())?;
    Some(match kind {
        FieldKind::Number { min, max } => {
            let value = min.map_or(value, |min| value.max(min));
            max.map_or(value, |max| value.min(max))
        }
        _ => value,
    })
}

/// The inspector's title for a widget (`widgetTitle`).
pub fn title(widget: &Widget) -> String {
    if widget.kind == WidgetKind::Symbol {
        return widget
            .text("symbol")
            .and_then(symbol_def)
            .map_or_else(|| "Symbol".to_owned(), |def| def.name.to_owned());
    }
    manifest(&widget.kind).map_or_else(
        || widget.kind.as_str().to_owned(),
        |manifest| manifest.name.to_owned(),
    )
}

/// How many tags `widget` binds: 0, 1, or a list.
pub fn max_tags(widget: &Widget) -> usize {
    manifest(&widget.kind).map_or(1, |manifest| manifest.max_tags)
}

/// What the binding of a symbol means (`bindingHint`).
pub fn binding_hint(binding: Binding) -> &'static str {
    match binding {
        Binding::State => "Running when the tag is above the threshold.",
        Binding::Level => "Fill level follows the tag between its min and max.",
        Binding::Value => "Shows the live tag value.",
        Binding::None => "Optional. A bound tag above zero animates flow.",
    }
}

// ── Placing ───────────────────────────────────────────────────────────────

/// Something the palette places: a widget type, or a library symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placing {
    /// A widget type by its wire name.
    Widget(&'static str),
    /// A library symbol by its key.
    Symbol(&'static str),
}

impl Placing {
    /// The size a new one takes.
    pub fn size(self) -> (f64, f64) {
        match self {
            Self::Widget(kind) => MANIFESTS
                .iter()
                .find(|manifest| manifest.kind == kind)
                .map_or((100.0, 100.0), |manifest| manifest.size),
            Self::Symbol(key) => symbol_def(key).map_or((90.0, 90.0), |def| def.size),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Widget(kind) => MANIFESTS
                .iter()
                .find(|manifest| manifest.kind == kind)
                .map_or(kind, |manifest| manifest.name),
            Self::Symbol(key) => symbol_def(key).map_or(key, |def| def.name),
        }
    }

    /// A new widget with its corner at `(x, y)` (kept on the stage), no
    /// tag and no settings but the symbol's key (`createWidget`).
    pub fn widget(self, id: String, x: f64, y: f64) -> Widget {
        let (width, height) = self.size();
        let (x, y) = clamp_pos(x, y, width, height);
        let (kind, config) = match self {
            Self::Widget(kind) => (WidgetKind::parse(kind), Map::new()),
            Self::Symbol(key) => {
                let mut config = Map::new();
                config.insert("symbol".to_owned(), Value::from(key));
                (WidgetKind::Symbol, config)
            }
        };
        Widget {
            id,
            kind,
            name: String::new(),
            tags: Vec::new(),
            x,
            y,
            width,
            height,
            config,
        }
    }
}

/// A position on the grid.
pub fn snap(value: f64) -> f64 {
    (value / GRID).round() * GRID
}

/// A box's corner kept on the stage (`clampPos`).
pub fn clamp_pos(x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    (
        x.min(STAGE_W - width).max(0.0),
        y.min(STAGE_H - height).max(0.0),
    )
}

/// Where click-to-add puts a new box: the stage's centre, each next one
/// 20 pixels further, eight steps round (`placeAt`).
#[allow(clippy::cast_precision_loss)]
pub fn place_at(count: usize, width: f64, height: f64) -> (f64, f64) {
    let step = (count % 8) as f64 * 20.0;
    clamp_pos(
        snap(STAGE_W / 2.0 - width / 2.0 + step),
        snap(STAGE_H / 2.0 - height / 2.0 + step),
        width,
        height,
    )
}

/// Where a drop at a stage point puts a box's corner: on the grid, on the
/// stage.
pub fn drop_at(x: f64, y: f64, width: f64, height: f64) -> (f64, f64) {
    clamp_pos(snap(x), snap(y), width, height)
}

/// A fresh id: `w_<ms>_<5 base-36 characters>` (`newId`).
pub fn fresh_id(prefix: &str) -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    // A new `RandomState` has fresh keys: its hash of the time is random.
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(ms);
    id_from(prefix, ms, hasher.finish())
}

/// The id made of a prefix, a time and random bits.
pub fn id_from(prefix: &str, ms: u128, random: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut rest = random;
    let mut tail = String::with_capacity(5);
    for _ in 0..5 {
        #[allow(clippy::cast_possible_truncation)]
        tail.push(char::from(DIGITS[(rest % 36) as usize]));
        rest /= 36;
    }
    format!("{prefix}_{ms}_{tail}")
}

/// A new connection (`createConnection`).
pub fn connection(id: String, from: &str, to: &str, kind: &str) -> Connection {
    Connection {
        id,
        from: from.to_owned(),
        to: to.to_owned(),
        kind: kind.to_owned(),
        waypoints: Vec::new(),
    }
}

// ── Edits (`layoutReducer`) ───────────────────────────────────────────────

/// One change to a layout.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    /// New widgets (on top) and connections.
    Add {
        widgets: Vec<Widget>,
        connections: Vec<Connection>,
    },
    /// Remove widgets (and every connection to them) and connections.
    Remove {
        widgets: Vec<String>,
        connections: Vec<String>,
    },
    /// Put widgets' corners at these points.
    Move(Vec<(String, f64, f64)>),
    Resize {
        id: String,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
    /// Set one setting (`UPDATE_WIDGET_CONFIG`); null clears it.
    Config {
        id: String,
        key: String,
        value: Value,
    },
    /// Rebind a widget's tags; the first is the primary one.
    Bind { id: String, tags: Vec<i64> },
    /// Change a connection's type.
    Line { id: String, kind: String },
    /// Bring widgets to the front or send them to the back, keeping their
    /// order among themselves (`REORDER_WIDGET`).
    Reorder { ids: Vec<String>, front: bool },
    /// Several edits as one step.
    Many(Vec<Edit>),
}

impl Edit {
    /// Change `layout`; `false`, leaving it as it was, when this changes
    /// nothing.
    #[allow(clippy::float_cmp)]
    pub fn apply(&self, layout: &mut Layout) -> bool {
        match self {
            Self::Add {
                widgets,
                connections,
            } => {
                layout.widgets.extend(widgets.iter().cloned());
                layout.connections.extend(connections.iter().cloned());
                !widgets.is_empty() || !connections.is_empty()
            }
            Self::Remove {
                widgets,
                connections,
            } => {
                let before = (layout.widgets.len(), layout.connections.len());
                let gone: HashSet<&str> = widgets.iter().map(String::as_str).collect();
                layout
                    .widgets
                    .retain(|widget| !gone.contains(widget.id.as_str()));
                layout.connections.retain(|connection| {
                    !gone.contains(connection.from.as_str())
                        && !gone.contains(connection.to.as_str())
                        && !connections.contains(&connection.id)
                });
                before != (layout.widgets.len(), layout.connections.len())
            }
            Self::Move(moves) => {
                let mut changed = false;
                for (id, x, y) in moves {
                    if let Some(widget) = layout.widgets.iter_mut().find(|w| w.id == *id)
                        && (widget.x != *x || widget.y != *y)
                    {
                        widget.x = *x;
                        widget.y = *y;
                        changed = true;
                    }
                }
                changed
            }
            Self::Resize {
                id,
                x,
                y,
                width,
                height,
            } => {
                let Some(widget) = layout.widgets.iter_mut().find(|w| w.id == *id) else {
                    return false;
                };
                if (widget.x, widget.y, widget.width, widget.height) == (*x, *y, *width, *height) {
                    return false;
                }
                widget.x = *x;
                widget.y = *y;
                widget.width = *width;
                widget.height = *height;
                true
            }
            Self::Config { id, key, value } => {
                let Some(widget) = layout.widgets.iter_mut().find(|w| w.id == *id) else {
                    return false;
                };
                let current = widget.config.get(key.as_str()).unwrap_or(&Value::Null);
                if current == value {
                    return false;
                }
                if value.is_null() {
                    widget.config.remove(key.as_str());
                } else {
                    widget.config.insert(key.clone(), value.clone());
                }
                true
            }
            Self::Bind { id, tags } => {
                let Some(widget) = layout.widgets.iter_mut().find(|w| w.id == *id) else {
                    return false;
                };
                if widget.tags == *tags {
                    return false;
                }
                widget.tags.clone_from(tags);
                true
            }
            Self::Line { id, kind } => {
                let Some(line) = layout.connections.iter_mut().find(|c| c.id == *id) else {
                    return false;
                };
                if line.kind == *kind {
                    return false;
                }
                line.kind.clone_from(kind);
                true
            }
            Self::Reorder { ids, front } => {
                let before: Vec<String> = layout.widgets.iter().map(|w| w.id.clone()).collect();
                let (picked, rest): (Vec<Widget>, Vec<Widget>) =
                    std::mem::take(&mut layout.widgets)
                        .into_iter()
                        .partition(|widget| ids.contains(&widget.id));
                layout.widgets = if *front {
                    rest.into_iter().chain(picked).collect()
                } else {
                    picked.into_iter().chain(rest).collect()
                };
                layout
                    .widgets
                    .iter()
                    .map(|widget| &widget.id)
                    .ne(before.iter())
            }
            Self::Many(edits) => edits
                .iter()
                .fold(false, |changed, edit| edit.apply(layout) || changed),
        }
    }
}

// ── Undo and redo (`historyReducer`) ──────────────────────────────────────

/// The layout being edited, the steps before and after it, and the copy
/// last loaded from or saved to the server.
#[derive(Debug, Clone, Default)]
pub struct History {
    past: Vec<Layout>,
    present: Layout,
    future: Vec<Layout>,
    /// Consecutive edits with this key (typing in one field, one drag, one
    /// nudge run) are one step.
    last_key: Option<String>,
    saved: Layout,
}

impl History {
    /// Start from the server's copy.
    pub fn new(layout: Layout) -> Self {
        Self {
            saved: layout.clone(),
            present: layout,
            ..Self::default()
        }
    }

    pub fn present(&self) -> &Layout {
        &self.present
    }

    pub fn saved(&self) -> &Layout {
        &self.saved
    }

    /// Apply `edit`; with the key of the edit before it, it joins that
    /// step. Whether it changed anything.
    pub fn apply(&mut self, edit: &Edit, coalesce: Option<&str>) -> bool {
        if coalesce.is_some() && coalesce == self.last_key.as_deref() {
            if edit.apply(&mut self.present) {
                self.future.clear();
                return true;
            }
            return false;
        }
        let before = self.present.clone();
        if !edit.apply(&mut self.present) {
            return false;
        }
        self.past.push(before);
        if self.past.len() > HISTORY_LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
        self.last_key = coalesce.map(str::to_owned);
        true
    }

    /// End the running step: the next edit starts a new one.
    pub fn seal(&mut self) {
        self.last_key = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.past.pop() else {
            return false;
        };
        let present = std::mem::replace(&mut self.present, previous);
        self.future.insert(0, present);
        self.last_key = None;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.future.is_empty() {
            return false;
        }
        let next = self.future.remove(0);
        let present = std::mem::replace(&mut self.present, next);
        self.past.push(present);
        self.last_key = None;
        true
    }

    /// Unsaved changes: the layout differs from the server's copy.
    pub fn dirty(&self) -> bool {
        self.present != self.saved
    }

    /// The server now has `snapshot` (what was sent; edits made while the
    /// save was on its way stay unsaved).
    pub fn mark_saved(&mut self, snapshot: Layout) {
        self.saved = snapshot;
        self.last_key = None;
    }
}

// ── Geometry ──────────────────────────────────────────────────────────────

/// A box on the stage.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn of(widget: &Widget) -> Self {
        Self {
            x: widget.x,
            y: widget.y,
            width: widget.width,
            height: widget.height,
        }
    }

    /// The box between two corners, in any order.
    pub fn between(a: (f64, f64), b: (f64, f64)) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            width: (a.0 - b.0).abs(),
            height: (a.1 - b.1).abs(),
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    /// `other` lies wholly inside.
    pub fn encloses(&self, other: &Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    #[must_use]
    pub fn moved(&self, dx: f64, dy: f64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    fn xs(&self) -> [f64; 3] {
        [self.x, self.center_x(), self.right()]
    }

    fn ys(&self) -> [f64; 3] {
        [self.y, self.center_y(), self.bottom()]
    }
}

/// The box around some widgets.
pub fn bounds<'a>(widgets: impl IntoIterator<Item = &'a Widget>) -> Option<Rect> {
    widgets
        .into_iter()
        .map(Rect::of)
        .reduce(|all, rect| all.union(&rect))
}

/// The widgets with these ids, in stacking order.
pub fn picked<'a>(layout: &'a Layout, ids: &'a [String]) -> impl Iterator<Item = &'a Widget> {
    layout
        .widgets
        .iter()
        .filter(move |widget| ids.contains(&widget.id))
}

/// A line drawn while a box lines up with another: vertical at `x = at`
/// (or horizontal at `y = at`) from `from` to `to`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guide {
    pub vertical: bool,
    pub at: f64,
    pub from: f64,
    pub to: f64,
}

/// How a move or resize snaps: to the grid, and to other widgets' edges
/// and centres within `threshold` stage pixels (0: not at all).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapping {
    pub grid: bool,
    pub threshold: f64,
}

/// The boxes a moving one lines up with: the other widgets, and the
/// stage's centre lines.
fn targets(layout: &Layout, moving: &[&str]) -> Vec<Rect> {
    let mut rects: Vec<Rect> = layout
        .widgets
        .iter()
        .filter(|widget| !moving.contains(&widget.id.as_str()))
        .map(Rect::of)
        .collect();
    rects.push(Rect {
        x: STAGE_W / 2.0,
        y: 0.0,
        width: 0.0,
        height: STAGE_H,
    });
    rects.push(Rect {
        x: 0.0,
        y: STAGE_H / 2.0,
        width: STAGE_W,
        height: 0.0,
    });
    rects
}

/// The smallest shift within `threshold` that puts one of `edges` on one
/// of `lines`.
fn nearest(edges: &[f64], lines: impl Iterator<Item = f64> + Clone, threshold: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for edge in edges {
        for line in lines.clone() {
            let shift = line - edge;
            if shift.abs() <= threshold && best.is_none_or(|best| shift.abs() < best.abs()) {
                best = Some(shift);
            }
        }
    }
    best
}

/// The guides of `rect` against `others`: each of its edges and centres
/// that one of theirs lines up with.
fn guides_for(rect: &Rect, others: &[Rect]) -> Vec<Guide> {
    let mut guides: Vec<Guide> = Vec::new();
    for at in rect.xs() {
        let lined: Vec<&Rect> = others
            .iter()
            .filter(|other| other.xs().iter().any(|x| (x - at).abs() < 0.5))
            .collect();
        if !lined.is_empty() && !guides.iter().any(|g| g.vertical && (g.at - at).abs() < 0.5) {
            guides.push(Guide {
                vertical: true,
                at,
                from: lined.iter().map(|o| o.y).fold(rect.y, f64::min),
                to: lined
                    .iter()
                    .map(|o| o.bottom())
                    .fold(rect.bottom(), f64::max),
            });
        }
    }
    for at in rect.ys() {
        let lined: Vec<&Rect> = others
            .iter()
            .filter(|other| other.ys().iter().any(|y| (y - at).abs() < 0.5))
            .collect();
        if !lined.is_empty()
            && !guides
                .iter()
                .any(|g| !g.vertical && (g.at - at).abs() < 0.5)
        {
            guides.push(Guide {
                vertical: false,
                at,
                from: lined.iter().map(|o| o.x).fold(rect.x, f64::min),
                to: lined.iter().map(|o| o.right()).fold(rect.right(), f64::max),
            });
        }
    }
    guides
}

/// Where a move lands.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapped {
    pub dx: f64,
    pub dy: f64,
    pub guides: Vec<Guide>,
}

/// The shift that snaps a moving box along one axis: lined up with
/// another's edge or centre, else its corner on the grid.
fn shift_for(edges: [f64; 3], lines: &[f64], corner: f64, snapping: Snapping) -> f64 {
    let lined = if snapping.threshold > 0.0 {
        nearest(&edges, lines.iter().copied(), snapping.threshold)
    } else {
        None
    };
    lined.unwrap_or(if snapping.grid {
        snap(corner) - corner
    } else {
        0.0
    })
}

/// Move the widgets `moving` (whose box was `start` when the drag began)
/// by `(dx, dy)`: an edge or centre within the threshold of another
/// widget's lines up with it, else the box's corner goes on the grid; the
/// box stays on the stage.
pub fn snap_move(
    layout: &Layout,
    moving: &[String],
    start: Rect,
    dx: f64,
    dy: f64,
    snapping: Snapping,
) -> Snapped {
    let ids: Vec<&str> = moving.iter().map(String::as_str).collect();
    let others = targets(layout, &ids);
    let moved = start.moved(dx, dy);
    let xs: Vec<f64> = others.iter().flat_map(Rect::xs).collect();
    let ys: Vec<f64> = others.iter().flat_map(Rect::ys).collect();
    let sx = shift_for(moved.xs(), &xs, moved.x, snapping);
    let sy = shift_for(moved.ys(), &ys, moved.y, snapping);
    let dx = (dx + sx).min(STAGE_W - start.right()).max(-start.x);
    let dy = (dy + sy).min(STAGE_H - start.bottom()).max(-start.y);
    let guides = if snapping.threshold > 0.0 {
        guides_for(&start.moved(dx, dy), &others)
    } else {
        Vec::new()
    };
    Snapped { dx, dy, guides }
}

/// A resize handle: the edges it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    NorthWest,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NorthWest,
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
    ];

    /// Which edges it moves: -1 the left (top), 1 the right (bottom), 0
    /// neither.
    pub fn sides(self) -> (i8, i8) {
        match self {
            Self::NorthWest => (-1, -1),
            Self::North => (0, -1),
            Self::NorthEast => (1, -1),
            Self::East => (1, 0),
            Self::SouthEast => (1, 1),
            Self::South => (0, 1),
            Self::SouthWest => (-1, 1),
            Self::West => (-1, 0),
        }
    }

    /// Where it sits on a box.
    pub fn point(self, rect: Rect) -> (f64, f64) {
        let (hx, hy) = self.sides();
        let x = match hx {
            -1 => rect.x,
            1 => rect.right(),
            _ => rect.center_x(),
        };
        let y = match hy {
            -1 => rect.y,
            1 => rect.bottom(),
            _ => rect.center_y(),
        };
        (x, y)
    }
}

/// The handle of `rect` within `radius` of a stage point, the nearest.
pub fn hit_handle(rect: Rect, x: f64, y: f64, radius: f64) -> Option<Handle> {
    Handle::ALL
        .into_iter()
        .map(|handle| {
            let (hx, hy) = handle.point(rect);
            (handle, (hx - x).abs().max((hy - y).abs()))
        })
        .filter(|(_, distance)| *distance <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

/// An edge being dragged: lined up with another widget's, else on the
/// grid.
fn place_edge(edge: f64, lines: &[f64], snapping: Snapping) -> f64 {
    let lined = if snapping.threshold > 0.0 {
        nearest(&[edge], lines.iter().copied(), snapping.threshold)
    } else {
        None
    };
    match lined {
        Some(shift) => edge + shift,
        None if snapping.grid => snap(edge),
        None => edge,
    }
}

/// Resize widget `id` (its box `start` when the drag began) by dragging
/// `handle` by `(dx, dy)`: the moving edges line up with other widgets' or
/// go on the grid; the box keeps its least size and stays on the stage.
pub fn snap_resize(
    layout: &Layout,
    id: &str,
    start: Rect,
    handle: Handle,
    dx: f64,
    dy: f64,
    snapping: Snapping,
) -> (Rect, Vec<Guide>) {
    let others = targets(layout, &[id]);
    let xs: Vec<f64> = others.iter().flat_map(Rect::xs).collect();
    let ys: Vec<f64> = others.iter().flat_map(Rect::ys).collect();
    let (hx, hy) = handle.sides();
    let (mut left, mut right) = (start.x, start.right());
    let (mut top, mut bottom) = (start.y, start.bottom());
    if hx < 0 {
        left = place_edge(start.x + dx, &xs, snapping)
            .min(right - MIN_WIDTH)
            .max(0.0);
    } else if hx > 0 {
        right = place_edge(start.right() + dx, &xs, snapping)
            .max(left + MIN_WIDTH)
            .min(STAGE_W);
    }
    if hy < 0 {
        top = place_edge(start.y + dy, &ys, snapping)
            .min(bottom - MIN_HEIGHT)
            .max(0.0);
    } else if hy > 0 {
        bottom = place_edge(start.bottom() + dy, &ys, snapping)
            .max(top + MIN_HEIGHT)
            .min(STAGE_H);
    }
    let rect = Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    };
    let guides = if snapping.threshold > 0.0 {
        guides_for(&rect, &others)
    } else {
        Vec::new()
    };
    (rect, guides)
}

/// The arrow keys' move of the widgets `ids` by `(dx, dy)`, the group kept
/// on the stage.
pub fn nudge(layout: &Layout, ids: &[String], dx: f64, dy: f64) -> Option<Edit> {
    let area = bounds(picked(layout, ids))?;
    let dx = dx.min(STAGE_W - area.right()).max(-area.x);
    let dy = dy.min(STAGE_H - area.bottom()).max(-area.y);
    Some(Edit::Move(
        picked(layout, ids)
            .map(|widget| (widget.id.clone(), widget.x + dx, widget.y + dy))
            .collect(),
    ))
}

/// The topmost widget at a stage point.
pub fn hit_widget(layout: &Layout, x: f64, y: f64) -> Option<usize> {
    layout
        .widgets
        .iter()
        .rposition(|widget| Rect::of(widget).contains(x, y))
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = dx * dx + dy * dy;
    let t = if length <= 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0)
    };
    (p.0 - (a.0 + t * dx)).hypot(p.1 - (a.1 + t * dy))
}

/// The topmost connection whose route passes within `tolerance` of a
/// stage point.
pub fn hit_connection(layout: &Layout, x: f64, y: f64, tolerance: f64) -> Option<usize> {
    layout.connections.iter().rposition(|line| {
        let (Some(a), Some(b)) = (layout.widget(&line.from), layout.widget(&line.to)) else {
            return false;
        };
        route(a, b, &line.waypoints)
            .windows(2)
            .any(|pair| distance_to_segment((x, y), pair[0], pair[1]) <= tolerance)
    })
}

/// The route drawn while the connect tool waits for its target: from the
/// source's nozzle to the pointer.
pub fn pending_route(from: &Widget, x: f64, y: f64) -> Vec<(f64, f64)> {
    let pointer = Widget {
        id: String::new(),
        kind: WidgetKind::Other(String::new()),
        name: String::new(),
        tags: Vec::new(),
        x,
        y,
        width: 0.0,
        height: 0.0,
        config: Map::new(),
    };
    route(from, &pointer, &[])
}

/// The widgets wholly inside a rubber band, in stacking order.
pub fn enclosed(layout: &Layout, band: Rect) -> Vec<String> {
    layout
        .widgets
        .iter()
        .filter(|widget| band.encloses(&Rect::of(widget)))
        .map(|widget| widget.id.clone())
        .collect()
}

/// Every widget in reading order (rows of about 80 stage pixels, left to
/// right), for Tab in the editor.
pub fn reading_order(layout: &Layout) -> Vec<usize> {
    let mut order: Vec<usize> = (0..layout.widgets.len()).collect();
    #[allow(clippy::cast_possible_truncation)]
    order.sort_by_key(|index| {
        let (cx, cy) = layout.widgets[*index].center();
        ((cy / 80.0).floor() as i64, cx.round() as i64)
    });
    order
}

/// The widget after (or before) `current` in reading order, round the end.
pub fn step(layout: &Layout, current: Option<&str>, forward: bool) -> Option<String> {
    let order = reading_order(layout);
    if order.is_empty() {
        return None;
    }
    let at = current.and_then(|id| {
        order
            .iter()
            .position(|index| layout.widgets[*index].id == id)
    });
    let next = match (at, forward) {
        (None, true) => 0,
        (None, false) => order.len() - 1,
        (Some(at), true) => (at + 1) % order.len(),
        (Some(at), false) => (at + order.len() - 1) % order.len(),
    };
    Some(layout.widgets[order[next]].id.clone())
}

/// Whether a quarter-turn rotation applies (a library symbol that does not
/// stretch, as the inspector's Rotation field).
pub fn turns(widget: &Widget) -> bool {
    resolve(widget).is_some_and(|resolved| !resolved.def.stretch)
}

/// Turn the rotatable widgets among `ids` by `quarters` quarter turns
/// (clockwise when positive).
pub fn rotate(layout: &Layout, ids: &[String], quarters: i32) -> Option<Edit> {
    let edits: Vec<Edit> = picked(layout, ids)
        .filter(|widget| turns(widget))
        .map(|widget| {
            let now = i32::from(quarter_turns(widget.number("rotation").unwrap_or(0.0)));
            Edit::Config {
                id: widget.id.clone(),
                key: "rotation".to_owned(),
                value: Value::from((now + quarters).rem_euclid(4) * 90),
            }
        })
        .collect();
    (!edits.is_empty()).then_some(Edit::Many(edits))
}

/// Mirror (or unmirror) the rotatable widgets among `ids`.
pub fn mirror(layout: &Layout, ids: &[String]) -> Option<Edit> {
    let edits: Vec<Edit> = picked(layout, ids)
        .filter(|widget| turns(widget))
        .map(|widget| Edit::Config {
            id: widget.id.clone(),
            key: "flipX".to_owned(),
            value: Value::Bool(!widget.flag("flipX").unwrap_or(false)),
        })
        .collect();
    (!edits.is_empty()).then_some(Edit::Many(edits))
}

// ── Copy and paste ────────────────────────────────────────────────────────

/// Copied widgets and the connections between them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clip {
    pub widgets: Vec<Widget>,
    pub connections: Vec<Connection>,
}

impl Clip {
    pub fn is_empty(&self) -> bool {
        self.widgets.is_empty()
    }

    /// The clip moved by `offset`, so the next paste lands further on
    /// (the web's clipboard steps 20 pixels after each paste).
    pub fn shift(&mut self, offset: f64) {
        for widget in &mut self.widgets {
            widget.x += offset;
            widget.y += offset;
        }
    }
}

/// The widgets `ids` (in stacking order) and the connections among them.
pub fn copy(layout: &Layout, ids: &[String]) -> Clip {
    let widgets: Vec<Widget> = picked(layout, ids).cloned().collect();
    let connections = layout
        .connections
        .iter()
        .filter(|line| {
            widgets.iter().any(|w| w.id == line.from) && widgets.iter().any(|w| w.id == line.to)
        })
        .cloned()
        .collect();
    Clip {
        widgets,
        connections,
    }
}

/// The edit that pastes `clip` `offset` pixels right and down (the group
/// kept on the stage), with fresh ids from `fresh`, its connections
/// rejoined to the copies.
pub fn paste(
    layout: &Layout,
    clip: &Clip,
    offset: f64,
    mut fresh: impl FnMut(&str) -> String,
) -> Option<Edit> {
    let area = bounds(&clip.widgets)?;
    let dx = offset.min(STAGE_W - area.right()).max(-area.x);
    let dy = offset.min(STAGE_H - area.bottom()).max(-area.y);
    let mut taken: HashSet<String> = layout
        .widgets
        .iter()
        .map(|widget| widget.id.clone())
        .chain(layout.connections.iter().map(|line| line.id.clone()))
        .collect();
    let mut unique = |prefix: &str| loop {
        let id = fresh(prefix);
        if taken.insert(id.clone()) {
            break id;
        }
    };
    let mut renamed: Vec<(String, String)> = Vec::with_capacity(clip.widgets.len());
    let mut widgets: Vec<Widget> = Vec::with_capacity(clip.widgets.len());
    for widget in &clip.widgets {
        let id = unique("w");
        renamed.push((widget.id.clone(), id.clone()));
        widgets.push(Widget {
            id,
            x: widget.x + dx,
            y: widget.y + dy,
            ..widget.clone()
        });
    }
    let new_id = |old: &str| {
        renamed
            .iter()
            .find(|(from, _)| from == old)
            .map(|(_, to)| to.clone())
    };
    let mut connections: Vec<Connection> = Vec::with_capacity(clip.connections.len());
    for line in &clip.connections {
        let (Some(from), Some(to)) = (new_id(&line.from), new_id(&line.to)) else {
            continue;
        };
        connections.push(Connection {
            id: unique("c"),
            from,
            to,
            kind: line.kind.clone(),
            waypoints: line
                .waypoints
                .iter()
                .map(|(x, y)| (x + dx, y + dy))
                .collect(),
        });
    }
    Some(Edit::Add {
        widgets,
        connections,
    })
}

// ── Saving ────────────────────────────────────────────────────────────────

/// The layout with each widget's `equipment_name` as the web's `toAPI`
/// writes it: its label, else its primary tag's name, else its type.
pub fn named(layout: &Layout, tag_name: impl Fn(i64) -> Option<String>) -> Layout {
    let mut named = layout.clone();
    for widget in &mut named.widgets {
        let name = widget
            .text("label")
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
            .or_else(|| widget.tag().and_then(&tag_name))
            .unwrap_or_else(|| title(widget));
        widget.name = name;
    }
    named
}

/// What the screen holds: widgets, connections and distinct bound tags
/// (`ScreenInspector`).
pub fn summary(layout: &Layout) -> (usize, usize, usize) {
    let tags: HashSet<i64> = layout
        .widgets
        .iter()
        .flat_map(|widget| widget.tags.iter().copied())
        .collect();
    (layout.widgets.len(), layout.connections.len(), tags.len())
}

#[cfg(test)]
mod tests;
