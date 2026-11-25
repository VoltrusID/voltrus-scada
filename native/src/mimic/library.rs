//! The shared symbol library as data: `symbols/<key>.json` (format
//! `voltrus-symbol/1`, schema `symbols/symbol.schema.json`) and the ISA-101
//! state styles of `symbols/_styles.json`, the same files the web client
//! draws (`client/src/scada2/hmi/dataSymbols.tsx`, `symbolStyle.ts`).
//!
//! The files are embedded at build time (`build.rs`), so the console needs
//! nothing beside its binary. Each file becomes a [`SymbolDef`] (key, view
//! box, size, binding, ports: what placing, ports and the faceplate need)
//! that points back at its [`SymbolFile`] for the drawing: layers of SVG path
//! data with a paint role, filtered by `if` against the widget's options, a
//! level area, upright text. [`symbol_style`] is `symbolStyle`: what each
//! paint looks like in a live state, as colour names the console resolves
//! per theme ([`color`]).

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::{Binding, Category, EquipState, Library, Ports, SymbolDef};
use crate::designer::expr::js_number;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/symbol_files.rs"));
}

/// The format the files declare.
pub const FORMAT: &str = "voltrus-symbol/1";

/// The ISA-101 categories, in palette order (`ISA_CATEGORIES`).
pub const ISA_CATEGORIES: [&str; 7] = [
    "Rotating equipment",
    "Valves",
    "Vessels",
    "Heat transfer",
    "Separation",
    "Instruments",
    "Piping",
];

// ── The file format ───────────────────────────────────────────────────────

/// One symbol file (`SymbolFile` in `symbolFile.ts`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SymbolFile {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    pub key: String,
    pub name: String,
    pub category: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub view_box: [f64; 2],
    pub size: [f64; 2],
    pub binding: FileBinding,
    #[serde(default)]
    pub stretch: bool,
    /// False: kept so saved screens draw, not offered in the palette.
    #[serde(default = "yes")]
    pub palette: bool,
    #[serde(default)]
    pub ports: FilePorts,
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub level: Option<LevelArea>,
    #[serde(default)]
    pub text: Vec<TextSpec>,
    #[serde(default)]
    pub options: Vec<SymbolOption>,
    /// Label of the secondary run tag (`config.runSensorId`).
    #[serde(default)]
    pub run_tag: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileBinding {
    State,
    Level,
    Value,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilePorts {
    pub left: Option<[f64; 2]>,
    pub right: Option<[f64; 2]>,
    pub top: Option<[f64; 2]>,
    pub bottom: Option<[f64; 2]>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub dir: String,
}

/// How a layer is painted (`Paint` in `symbolStyle.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaintRole {
    /// Main body: filled when running, hollow when stopped, red on fault.
    Body,
    /// Vessel walls, actuators, bubbles: always hollow, outline follows the state.
    Shell,
    /// Stroke only (nozzles, stands, stems).
    Line,
    /// Thin stroke only (ticks, media, internals).
    Detail,
    /// Stroke over a body, in the stage colour while the body is filled.
    Inner,
    /// Filled with the line colour (arrow heads, junction dots).
    Solid,
    /// Moving flow: only while the run tag is on, dashed and animated.
    Flow,
    /// Secondary drive: thin when stopped, bold while the run tag is on.
    Run,
}

impl PaintRole {
    pub const ALL: [Self; 8] = [
        Self::Body,
        Self::Shell,
        Self::Line,
        Self::Detail,
        Self::Inner,
        Self::Solid,
        Self::Flow,
        Self::Run,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Body => "body",
            Self::Shell => "shell",
            Self::Line => "line",
            Self::Detail => "detail",
            Self::Inner => "inner",
            Self::Solid => "solid",
            Self::Flow => "flow",
            Self::Run => "run",
        }
    }
}

/// A condition's wanted value: text or a flag.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Flag(bool),
    Text(String),
}

impl Scalar {
    /// JavaScript's `String(value)`.
    pub fn js(&self) -> String {
        match self {
            Self::Flag(flag) => flag.to_string(),
            Self::Text(text) => text.clone(),
        }
    }

    pub fn as_json(&self) -> Value {
        match self {
            Self::Flag(flag) => Value::Bool(*flag),
            Self::Text(text) => Value::from(text.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Wanted {
    One(Scalar),
    Any(Vec<Scalar>),
}

/// `{option: value}` or `{option: [values]}`: every one must hold.
pub type Condition = BTreeMap<String, Wanted>;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub d: String,
    pub paint: PaintRole,
    /// Base stroke width instead of the paint's.
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(rename = "if", default)]
    pub when: Option<Condition>,
}

/// The level fill area; fills from the bottom, clipped to `clip`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LevelArea {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    #[serde(default)]
    pub clip: Option<String>,
}

impl LevelArea {
    /// The filled rectangle at `level` (0 to 1): `x, y, w, h`.
    pub fn filled(&self, level: f64) -> (f64, f64, f64, f64) {
        let level = level.clamp(0.0, 1.0);
        (
            self.x,
            self.y + self.h * (1.0 - level),
            self.w,
            self.h * level,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Font {
    Ui,
    Mono,
}

/// Centred text, kept upright when the symbol turns or mirrors.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextSpec {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub text: Option<String>,
    /// An option key, or `$level`.
    #[serde(default)]
    pub from: Option<String>,
    pub size: f64,
    #[serde(default)]
    pub weight: Option<f64>,
    #[serde(default)]
    pub font: Option<Font>,
    #[serde(default)]
    pub on_body: bool,
    #[serde(rename = "if", default)]
    pub when: Option<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OptionKind {
    Select,
    Boolean,
    String,
}

/// A per-widget setting, kept in the widget's config under its key.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolOption {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: OptionKind,
    #[serde(default)]
    pub values: Vec<String>,
    pub default: Scalar,
}

/// A config value a condition compares with.
enum Have<'a> {
    Text(&'a str),
    Flag(bool),
    Other(String),
}

/// JavaScript's `String(value)` of a config value.
fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number
            .as_f64()
            .map_or_else(|| number.to_string(), js_number),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

impl SymbolFile {
    /// An option's value in `config`, its default when the config has none
    /// (`config[k] ?? option.default`).
    pub fn option_value(&self, config: &Map<String, Value>, key: &str) -> Option<Value> {
        match config.get(key) {
            Some(value) if !value.is_null() => Some(value.clone()),
            _ => self
                .options
                .iter()
                .find(|option| option.key == key)
                .map(|option| option.default.as_json()),
        }
    }

    /// Whether a layer's or text's condition holds for `config` (`matches`:
    /// `String(wanted) === String(have)`). Compared without allocating for
    /// text and flags, which is what options hold: the drawing asks every
    /// frame.
    pub fn matches(&self, when: Option<&Condition>, config: &Map<String, Value>) -> bool {
        let Some(when) = when else {
            return true;
        };
        when.iter().all(|(key, wanted)| {
            let have: Option<Have<'_>> = match config.get(key) {
                Some(Value::String(text)) => Some(Have::Text(text)),
                Some(Value::Bool(flag)) => Some(Have::Flag(*flag)),
                Some(Value::Null) | None => {
                    self.options.iter().find(|option| option.key == *key).map(
                        |option| match &option.default {
                            Scalar::Text(text) => Have::Text(text),
                            Scalar::Flag(flag) => Have::Flag(*flag),
                        },
                    )
                }
                Some(other) => Some(Have::Other(js_string(other))),
            };
            let one = |w: &Scalar| match (w, &have) {
                (Scalar::Text(w), Some(Have::Text(h))) => w == h,
                (Scalar::Flag(w), Some(Have::Flag(h))) => w == h,
                (Scalar::Text(w), Some(Have::Flag(h))) => *w == h.to_string(),
                (Scalar::Flag(w), Some(Have::Text(h))) => w.to_string() == *h,
                (w, Some(Have::Other(h))) => w.js() == *h,
                (w, None) => w.js() == "undefined",
            };
            match wanted {
                Wanted::One(w) => one(w),
                Wanted::Any(any) => any.iter().any(one),
            }
        })
    }

    /// The layers drawn for `config`, in order.
    pub fn layers_for<'a>(
        &'a self,
        config: &'a Map<String, Value>,
    ) -> impl Iterator<Item = &'a Layer> + 'a {
        self.layers
            .iter()
            .filter(move |layer| self.matches(layer.when.as_ref(), config))
    }

    /// The text shown by `spec` (`textOf`): fixed text, an option's value,
    /// or the level in % (hidden when unknown or `showPercent` is false).
    pub fn text_of(
        &self,
        spec: &TextSpec,
        config: &Map<String, Value>,
        level: Option<f64>,
    ) -> Option<String> {
        if let Some(text) = &spec.text {
            return Some(text.clone());
        }
        let from = spec.from.as_deref()?;
        if from == "$level" {
            let shown = config.get("showPercent") != Some(&Value::Bool(false));
            return level
                .filter(|_| shown)
                .map(|level| format!("{}%", (level.clamp(0.0, 1.0) * 100.0).round()));
        }
        match self.option_value(config, from)? {
            Value::Null => None,
            Value::String(text) if text.is_empty() => None,
            value => Some(js_string(&value)),
        }
    }

    /// Problems with the file, each naming its field (`validateSymbolFile`).
    pub fn problems(&self, file: &str) -> Vec<String> {
        let mut out = Vec::new();
        let key_ok = self.key.len() >= 2
            && self.key.len() <= 64
            && self.key.starts_with(|c: char| c.is_ascii_lowercase())
            && self
                .key
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !key_ok {
            out.push(format!(
                "key '{}' must be lowercase letters, digits and '_'",
                self.key
            ));
        }
        if file != self.key {
            out.push(format!(
                "key '{}' must equal the file name '{file}'",
                self.key
            ));
        }
        if self
            .format
            .as_deref()
            .is_some_and(|format| format != FORMAT)
        {
            out.push(format!("format must be {FORMAT}"));
        }
        if self.name.trim().is_empty() {
            out.push("name is empty".into());
        }
        if self.category.trim().is_empty() {
            out.push("category is empty".into());
        }
        let [vw, vh] = self.view_box;
        if !(vw > 0.0 && vh > 0.0) {
            out.push("viewBox must be two positive numbers".into());
        }
        if !(self.size[0] > 0.0 && self.size[1] > 0.0) {
            out.push("size must be two positive numbers".into());
        }
        let inside = |[x, y]: [f64; 2]| x >= 0.0 && y >= 0.0 && x <= vw && y <= vh;
        let ports = self.ports;
        for (side, port) in [
            ("left", ports.left),
            ("right", ports.right),
            ("top", ports.top),
            ("bottom", ports.bottom),
        ] {
            if port.is_some_and(|p| !inside(p)) {
                out.push(format!("ports.{side} is outside the viewBox"));
            }
        }
        for (i, anchor) in self.anchors.iter().enumerate() {
            if !inside([anchor.x, anchor.y]) {
                out.push(format!(
                    "anchors[{i}] '{}' is outside the viewBox",
                    anchor.id
                ));
            }
            if !matches!(anchor.dir.as_str(), "left" | "right" | "top" | "bottom") {
                out.push(format!("anchors[{i}].dir '{}' is not a side", anchor.dir));
            }
        }
        if self.layers.is_empty() {
            out.push("layers is empty".into());
        }
        let has_option = |key: &str| self.options.iter().any(|option| option.key == key);
        let check = |at: String, when: Option<&Condition>, out: &mut Vec<String>| {
            for key in when.into_iter().flat_map(|when| when.keys()) {
                if !has_option(key) {
                    out.push(format!("{at}.if names '{key}', which is not an option"));
                }
            }
        };
        for (i, layer) in self.layers.iter().enumerate() {
            if let Err(at) = super::svg_path::parse_svg_strict(&layer.d) {
                out.push(format!("layers[{i}].d: unreadable at {at}"));
            }
            check(format!("layers[{i}]"), layer.when.as_ref(), &mut out);
        }
        match (&self.level, self.binding) {
            (Some(level), binding) => {
                if binding != FileBinding::Level {
                    out.push("level is only for binding level".into());
                }
                if let Some(clip) = &level.clip
                    && let Err(at) = super::svg_path::parse_svg_strict(clip)
                {
                    out.push(format!("level.clip: unreadable at {at}"));
                }
            }
            (None, FileBinding::Level) => out.push("binding level needs a level area".into()),
            (None, _) => {}
        }
        for (i, text) in self.text.iter().enumerate() {
            if text.text.is_none() && text.from.is_none() {
                out.push(format!("text[{i}] needs text or from"));
            }
            if let Some(from) = text.from.as_deref()
                && from != "$level"
                && !has_option(from)
            {
                out.push(format!("text[{i}].from '{from}' is not an option"));
            }
            check(format!("text[{i}]"), text.when.as_ref(), &mut out);
        }
        for (i, option) in self.options.iter().enumerate() {
            match option.kind {
                OptionKind::Select if option.values.is_empty() => {
                    out.push(format!("options[{i}] '{}' has no values", option.key));
                }
                OptionKind::Select if !option.values.contains(&option.default.js()) => {
                    out.push(format!(
                        "options[{i}] '{}' default '{}' is not one of its values",
                        option.key,
                        option.default.js()
                    ));
                }
                OptionKind::Boolean if !matches!(option.default, Scalar::Flag(_)) => {
                    out.push(format!(
                        "options[{i}] '{}' default must be true or false",
                        option.key
                    ));
                }
                _ => {}
            }
        }
        out
    }

    /// Whether any layer moves while the run tag is on.
    pub fn has_flow(&self) -> bool {
        self.layers
            .iter()
            .any(|layer| layer.paint == PaintRole::Flow)
    }

    fn def(&'static self) -> SymbolDef {
        let port = |p: Option<[f64; 2]>| p.map(|[x, y]| (x, y));
        SymbolDef {
            key: &self.key,
            name: &self.name,
            category: Category::Isa(&self.category),
            library: Library::Isa,
            view_box: (self.view_box[0], self.view_box[1]),
            size: (self.size[0], self.size[1]),
            binding: match self.binding {
                FileBinding::State => Binding::State,
                FileBinding::Level => Binding::Level,
                FileBinding::Value => Binding::Value,
                FileBinding::None => Binding::None,
            },
            stretch: self.stretch,
            ports: Ports {
                left: port(self.ports.left),
                right: port(self.ports.right),
                top: port(self.ports.top),
                bottom: port(self.ports.bottom),
            },
            palette: self.palette,
            data: Some(self),
        }
    }
}

/// Every embedded file parsed, by key; a file that does not parse is
/// skipped (the tests make sure none does).
fn parse_files() -> Vec<SymbolFile> {
    embedded::FILES
        .iter()
        .filter_map(
            |(key, text)| match serde_json::from_str::<SymbolFile>(text) {
                Ok(file) if file.key == *key => Some(file),
                Ok(file) => {
                    eprintln!("symbols/{key}.json: its key is '{}'", file.key);
                    None
                }
                Err(error) => {
                    eprintln!("symbols/{key}.json: {error}");
                    None
                }
            },
        )
        .collect()
}

static FILES: LazyLock<Vec<SymbolFile>> = LazyLock::new(parse_files);

/// The symbol files, by key.
pub fn files() -> &'static [SymbolFile] {
    &FILES
}

/// Every symbol file as a library symbol, in the files' order.
pub(super) fn defs() -> Vec<SymbolDef> {
    let mut files: Vec<&'static SymbolFile> = FILES.iter().collect();
    let rank = |category: &str| {
        ISA_CATEGORIES
            .iter()
            .position(|c| *c == category)
            .unwrap_or(ISA_CATEGORIES.len())
    };
    files.sort_by(|a, b| {
        rank(&a.category)
            .cmp(&rank(&b.category))
            .then_with(|| js_compare(&a.name, &b.name))
    });
    files.into_iter().map(SymbolFile::def).collect()
}

/// `localeCompare` closely enough for symbol names: case-insensitive
/// first, then as written.
fn js_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let fold = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect()
    };
    fold(a).cmp(&fold(b)).then_with(|| a.cmp(b))
}

/// The keys of the embedded files, as the build found them.
pub fn embedded_keys() -> impl Iterator<Item = &'static str> {
    embedded::FILES.iter().map(|(key, _)| *key)
}

/// The text of an embedded file.
pub fn embedded_text(key: &str) -> Option<&'static str> {
    embedded::FILES
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, text)| *text)
}

// ── State styles (`_styles.json`, `symbolStyle.ts`) ──────────────────────

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateRule {
    pub stroke: String,
    pub width: f64,
    pub fill: String,
    #[serde(default)]
    pub dash: Option<String>,
    #[serde(default)]
    pub blink: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaintRule {
    pub fill: String,
    #[serde(default)]
    pub width: Option<f64>,
    #[serde(default)]
    pub stroke: Option<String>,
    #[serde(default)]
    pub dash: Option<String>,
    #[serde(default)]
    pub animate: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub stroke_on_filled: Option<String>,
    #[serde(default)]
    pub width_running: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TextRule {
    pub color: String,
    pub on_body_filled: String,
    pub on_body_hollow: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BadgeShape {
    Box,
    Square,
    Triangle,
    Diamond,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeRule {
    pub text: String,
    pub shape: BadgeShape,
    pub fill: String,
    pub stroke: String,
    pub color: String,
    #[serde(default)]
    pub blink: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BadgeRules {
    pub manual: BadgeRule,
    pub interlock: BadgeRule,
    pub alarm: BTreeMap<String, BadgeRule>,
}

/// `_styles.json`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Styles {
    #[serde(rename = "$comment", default)]
    pub comment: Option<String>,
    pub format: String,
    pub states: BTreeMap<String, StateRule>,
    pub paints: BTreeMap<String, PaintRule>,
    pub text: TextRule,
    pub badges: BadgeRules,
    /// Colour names to `#rrggbb`, per theme (`dark`, `light`).
    pub colors: BTreeMap<String, BTreeMap<String, String>>,
}

static STYLES: LazyLock<Styles> = LazyLock::new(|| {
    serde_json::from_str(embedded::STYLES).unwrap_or_else(|error| {
        // Embedded at build time and checked by the tests.
        panic!("symbols/_styles.json: {error}")
    })
});

pub fn styles() -> &'static Styles {
    &STYLES
}

/// A theme colour by name (`isa-line`, `hmi-fault`, …) as `0xRRGGBB`.
pub fn color(dark: bool, name: &str) -> Option<u32> {
    let theme = if dark { "dark" } else { "light" };
    let hex = STYLES.colors.get(theme)?.get(name)?;
    u32::from_str_radix(hex.strip_prefix('#')?, 16).ok()
}

/// What a paint fills with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    None,
    Color(&'static str),
}

/// One paint in one state (`PaintStyle`): colour names, widths in stage
/// pixels (strokes do not scale with the symbol).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaintStyle {
    pub stroke: &'static str,
    pub width: f64,
    pub fill: Fill,
    /// Dash and gap.
    pub dash: Option<(f64, f64)>,
    /// Drawn in this state at all.
    pub visible: bool,
    /// The dashes run (`hmi-flow`).
    pub animate: bool,
}

/// A state indicator beside the symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeKind {
    Alarm,
    Manual,
    Interlock,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Badge {
    pub kind: BadgeKind,
    pub text: &'static str,
    pub shape: BadgeShape,
    pub fill: &'static str,
    pub stroke: &'static str,
    pub color: &'static str,
    pub blink: bool,
}

/// What modifies the main state's look (`StyleModifiers`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Modifiers {
    /// The run tag on (pipe flow, agitator).
    pub run_on: bool,
    pub manual: bool,
    pub interlocked: bool,
    /// 1 to 3.
    pub alarm_priority: Option<u8>,
}

/// A symbol's look in one live state (`SymbolStyle`).
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolStyle {
    /// By [`PaintRole`], in [`PaintRole::ALL`] order.
    pub paints: [PaintStyle; 8],
    /// The whole symbol blinks (fault).
    pub blink: bool,
    /// Plain text's colour.
    pub text: &'static str,
    /// Text on a body.
    pub text_on_body: &'static str,
    /// Alarm, manual, interlock, top to bottom.
    pub badges: Vec<Badge>,
}

impl SymbolStyle {
    pub fn paint(&self, role: PaintRole) -> PaintStyle {
        self.paints[role as usize]
    }
}

/// Every state's style, made once: [`symbol_style`] by index.
static STYLE_TABLE: LazyLock<Vec<SymbolStyle>> = LazyLock::new(|| {
    let mut out = Vec::with_capacity(STATES.len() * 2 * 2 * 2 * 4);
    for state in STATES {
        for run_on in [false, true] {
            for manual in [false, true] {
                for interlocked in [false, true] {
                    for alarm_priority in [None, Some(1), Some(2), Some(3)] {
                        let mods = Modifiers {
                            run_on,
                            manual,
                            interlocked,
                            alarm_priority,
                        };
                        out.push(symbol_style(state, mods));
                    }
                }
            }
        }
    }
    out
});

const STATES: [EquipState; 5] = [
    EquipState::Idle,
    EquipState::Running,
    EquipState::Stopped,
    EquipState::Fault,
    EquipState::Stale,
];

/// [`symbol_style`] without making it: the drawing asks every frame.
pub fn style_of(state: EquipState, mods: Modifiers) -> &'static SymbolStyle {
    let state_index = STATES.iter().position(|s| *s == state).unwrap_or(0);
    let alarm = match mods.alarm_priority {
        Some(p @ 1..=3) => usize::from(p),
        _ => 0,
    };
    let index = (((state_index * 2 + usize::from(mods.run_on)) * 2 + usize::from(mods.manual)) * 2
        + usize::from(mods.interlocked))
        * 4
        + alarm;
    &STYLE_TABLE[index]
}

pub fn state_name(state: EquipState) -> &'static str {
    match state {
        EquipState::Idle => "idle",
        EquipState::Running => "running",
        EquipState::Stopped => "stopped",
        EquipState::Fault => "fault",
        EquipState::Stale => "stale",
    }
}

fn dash(text: Option<&str>) -> Option<(f64, f64)> {
    let mut parts = text?.split_whitespace().filter_map(|p| p.parse().ok());
    Some((parts.next()?, parts.next()?))
}

/// The look of every paint, the badges and the blink in `state`
/// (`symbolStyle`), from `_styles.json`.
pub fn symbol_style(state: EquipState, mods: Modifiers) -> SymbolStyle {
    let rules: &'static Styles = &STYLES;
    let s = &rules.states[state_name(state)];
    let stroke: &'static str = &s.stroke;
    let filled = matches!(state, EquipState::Running | EquipState::Fault);
    let fault = state == EquipState::Fault;
    let thin = |w: f64| if fault { w + 0.75 } else { w };
    let base = PaintStyle {
        stroke,
        width: s.width,
        fill: Fill::None,
        dash: dash(s.dash.as_deref()),
        visible: true,
        animate: false,
    };
    let color = |name: &'static str| {
        if name == "none" {
            Fill::None
        } else {
            Fill::Color(name)
        }
    };
    let rule = |role: PaintRole| &rules.paints[role.name()];
    let paints = PaintRole::ALL.map(|role| {
        let r = rule(role);
        match role {
            PaintRole::Body => PaintStyle {
                fill: color(&s.fill),
                ..base
            },
            PaintRole::Shell => PaintStyle {
                fill: color(&r.fill),
                ..base
            },
            PaintRole::Line => base,
            PaintRole::Detail => PaintStyle {
                width: thin(r.width.unwrap_or(1.0)),
                ..base
            },
            PaintRole::Inner => PaintStyle {
                stroke: match (&r.stroke_on_filled, filled) {
                    (Some(on), true) => on,
                    _ => stroke,
                },
                width: thin(r.width.unwrap_or(1.25)),
                ..base
            },
            PaintRole::Solid => PaintStyle {
                fill: Fill::Color(stroke),
                ..base
            },
            PaintRole::Flow => PaintStyle {
                stroke: r.stroke.as_deref().unwrap_or("isa-on"),
                width: s.width,
                fill: Fill::None,
                dash: dash(r.dash.as_deref()),
                visible: r.when.as_deref() != Some("run") || mods.run_on,
                animate: r.animate.is_some(),
            },
            PaintRole::Run => PaintStyle {
                width: if mods.run_on {
                    r.width_running.unwrap_or(2.5)
                } else {
                    thin(r.width.unwrap_or(1.0))
                },
                ..base
            },
        }
    });

    let badge = |kind: BadgeKind, b: &'static BadgeRule| Badge {
        kind,
        text: &b.text,
        shape: b.shape,
        fill: &b.fill,
        stroke: &b.stroke,
        color: &b.color,
        blink: b.blink,
    };
    let mut badges = Vec::new();
    if let Some(b) = mods
        .alarm_priority
        .and_then(|p| rules.badges.alarm.get(&p.to_string()))
    {
        badges.push(badge(BadgeKind::Alarm, b));
    }
    if mods.manual {
        badges.push(badge(BadgeKind::Manual, &rules.badges.manual));
    }
    if mods.interlocked {
        badges.push(badge(BadgeKind::Interlock, &rules.badges.interlock));
    }
    SymbolStyle {
        paints,
        blink: s.blink,
        text: &rules.text.color,
        text_on_body: if filled {
            &rules.text.on_body_filled
        } else {
            &rules.text.on_body_hollow
        },
        badges,
    }
}

/// A layer's stroke width in a state: its own width (heavier on a fault)
/// or the paint's.
pub fn layer_width(layer: &Layer, style: &PaintStyle, state: EquipState) -> f64 {
    match layer.width {
        Some(width) if state == EquipState::Fault => width + 0.75,
        Some(width) => width,
        None => style.width,
    }
}

/// Alarm priority of a tag status: critical is 1, warning 2
/// (`alarmPriorityOf`).
pub fn alarm_priority(alarm: Option<super::Level>) -> Option<u8> {
    match alarm {
        Some(super::Level::Critical) => Some(1),
        Some(super::Level::Warning) => Some(2),
        Some(super::Level::Normal) | None => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::mimic::edit::palette_symbols;
    use crate::mimic::{SYMBOLS, symbol_def};

    /// What the web reads from the same files (`symbols.json`, written by
    /// `tests/fixtures/designer/symbols.mts` from the web's own code).
    fn web() -> Value {
        serde_json::from_str(include_str!("../../tests/fixtures/designer/symbols.json"))
            .expect("symbols.json")
    }

    fn pair(value: &Value) -> (f64, f64) {
        (value[0].as_f64().unwrap(), value[1].as_f64().unwrap())
    }

    #[test]
    fn every_file_parses_and_is_valid() {
        let keys: Vec<&str> = embedded_keys().collect();
        assert!(keys.len() >= 38, "{keys:?}");
        assert_eq!(files().len(), keys.len(), "every embedded file parses");
        for (key, text) in embedded::FILES {
            let file: SymbolFile =
                serde_json::from_str(text).unwrap_or_else(|e| panic!("symbols/{key}.json: {e}"));
            assert_eq!(file.problems(key), Vec::<String>::new(), "{key}");
            assert!(
                symbol_def(key).is_some_and(|def| def.data.is_some()),
                "{key}"
            );
        }
        assert_eq!(styles().format, "voltrus-symbol-styles/1");
        for theme in ["dark", "light"] {
            for name in styles().colors[theme].keys() {
                assert!(color(theme == "dark", name).is_some(), "{theme} {name}");
            }
        }
    }

    /// The same keys, and per key the same view box, size, binding,
    /// stretch, palette flag, category and ports as the web.
    #[test]
    fn native_and_web_agree_on_keys_ports_and_boxes() {
        let web = web();
        let web_keys: Vec<&str> = web["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let native_keys: Vec<&str> = embedded_keys().collect();
        assert_eq!(native_keys, web_keys);
        for symbol in web["symbols"].as_array().unwrap() {
            let key = symbol["key"].as_str().unwrap();
            assert_eq!(symbol["problems"], json!([]), "{key} on the web");
            let def = symbol_def(key).unwrap();
            assert_eq!(def.name, symbol["name"].as_str().unwrap(), "{key}");
            assert_eq!(
                def.category_name(),
                symbol["category"].as_str().unwrap(),
                "{key}"
            );
            assert_eq!(def.view_box, pair(&symbol["viewBox"]), "{key}");
            assert_eq!(def.size, pair(&symbol["size"]), "{key}");
            let binding = match def.binding {
                Binding::State => "state",
                Binding::Level => "level",
                Binding::Value => "value",
                Binding::None => "none",
            };
            assert_eq!(binding, symbol["binding"].as_str().unwrap(), "{key}");
            assert_eq!(def.stretch, symbol["stretch"].as_bool().unwrap(), "{key}");
            assert_eq!(def.palette, symbol["palette"].as_bool().unwrap(), "{key}");
            let port = |side: &str| symbol["ports"].get(side).map(pair);
            assert_eq!(
                (
                    def.ports.left,
                    def.ports.right,
                    def.ports.top,
                    def.ports.bottom
                ),
                (port("left"), port("right"), port("top"), port("bottom")),
                "{key} ports"
            );
            // The layers and texts each option value shows.
            let file = def.data.unwrap();
            for case in symbol["shown"].as_array().unwrap() {
                let config = case["config"].as_object().unwrap();
                let layers: Vec<usize> = file
                    .layers
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| file.matches(l.when.as_ref(), config))
                    .map(|(i, _)| i)
                    .collect();
                let texts: Vec<usize> = file
                    .text
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| file.matches(t.when.as_ref(), config))
                    .map(|(i, _)| i)
                    .collect();
                assert_eq!(json!(layers), case["layers"], "{key} {config:?} layers");
                assert_eq!(json!(texts), case["text"], "{key} {config:?} text");
            }
        }
        // The library lists the files in the web's order.
        let order: Vec<&str> = SYMBOLS
            .iter()
            .filter(|def| def.data.is_some())
            .map(|def| def.key)
            .collect();
        assert_eq!(json!(order), web["order"]);
    }

    #[test]
    fn the_palette_searches_as_the_web() {
        let web = web();
        for search in web["searches"].as_array().unwrap() {
            let query = search["query"].as_str().unwrap();
            let isa: Vec<&str> = palette_symbols(query)
                .iter()
                .filter(|def| def.data.is_some())
                .map(|def| def.key)
                .collect();
            assert_eq!(json!(isa), search["keys"], "{query:?}");
        }
        let groups: Vec<String> = crate::mimic::edit::palette_categories()
            .into_iter()
            .filter(|c| matches!(c, crate::mimic::Category::Isa(_)))
            .map(crate::mimic::edit::category_heading)
            .collect();
        let web_groups: Vec<&str> = web["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g.as_str().unwrap())
            .filter(|g| *g != "ISA-101: all")
            .collect();
        assert_eq!(groups, web_groups);
    }

    /// `symbolStyle` and the native style agree for every state, run tag,
    /// mode, interlock and alarm priority the fixture lists.
    #[test]
    fn styles_match_the_web() {
        let web = web();
        for case in web["styles"].as_array().unwrap() {
            let state = match case["state"].as_str().unwrap() {
                "idle" => EquipState::Idle,
                "running" => EquipState::Running,
                "stopped" => EquipState::Stopped,
                "fault" => EquipState::Fault,
                _ => EquipState::Stale,
            };
            #[allow(clippy::cast_possible_truncation)]
            let mods = Modifiers {
                run_on: case["runOn"].as_bool().unwrap(),
                manual: case["manual"].as_bool().unwrap(),
                interlocked: case["interlocked"].as_bool().unwrap(),
                alarm_priority: case["alarmPriority"].as_u64().map(|p| p as u8),
            };
            let style = style_of(state, mods);
            assert_eq!(style, &symbol_style(state, mods));
            for role in PaintRole::ALL {
                let p = style.paint(role);
                let native = json!({
                    "stroke": p.stroke,
                    "width": crate::mimic::edit::number_value(p.width),
                    "fill": match p.fill { Fill::None => "none", Fill::Color(c) => c },
                    "dash": p.dash.map(|(on, off)| format!("{} {}", js_number(on), js_number(off))),
                    "visible": p.visible,
                    "animate": p.animate,
                });
                assert_eq!(
                    native,
                    case["paints"][role.name()],
                    "{case} {}",
                    role.name()
                );
            }
            assert_eq!(json!(style.blink), case["blink"]);
            assert_eq!(json!(style.text), case["text"]);
            assert_eq!(json!(style.text_on_body), case["textOnBody"], "{case}");
            let badges: Vec<Value> = style
                .badges
                .iter()
                .map(|b| {
                    json!({
                        "kind": match b.kind {
                            BadgeKind::Alarm => "alarm",
                            BadgeKind::Manual => "manual",
                            BadgeKind::Interlock => "interlock",
                        },
                        "text": b.text,
                        "shape": match b.shape {
                            BadgeShape::Box => "box",
                            BadgeShape::Square => "square",
                            BadgeShape::Triangle => "triangle",
                            BadgeShape::Diamond => "diamond",
                        },
                        "fill": b.fill,
                        "stroke": b.stroke,
                        "color": b.color,
                        "blink": b.blink,
                    })
                })
                .collect();
            assert_eq!(json!(badges), case["badges"], "{case}");
        }
    }

    #[test]
    fn texts_come_from_options_and_the_level() {
        let tank = symbol_def("isa_tank").unwrap().data.unwrap();
        let spec = &tank.text[0];
        let none = Map::new();
        assert_eq!(
            tank.text_of(spec, &none, Some(0.625)).as_deref(),
            Some("63%")
        );
        assert_eq!(tank.text_of(spec, &none, None), None);
        let hidden = json!({ "showPercent": false });
        assert_eq!(
            tank.text_of(spec, hidden.as_object().unwrap(), Some(0.5)),
            None
        );
        let bubble = symbol_def("isa_instrument").unwrap().data.unwrap();
        let shown: Vec<String> = bubble
            .text
            .iter()
            .filter(|t| bubble.matches(t.when.as_ref(), &none))
            .filter_map(|t| bubble.text_of(t, &none, None))
            .collect();
        assert!(!shown.is_empty(), "the bubble's defaults show");
        let level = LevelArea {
            x: 7.0,
            y: 16.0,
            w: 86.0,
            h: 134.0,
            clip: None,
        };
        assert_eq!(level.filled(0.5), (7.0, 83.0, 86.0, 67.0));
        assert_eq!(level.filled(1.5), level.filled(1.0));
    }
}
