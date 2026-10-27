//! Data Manager › Sources without a window: the source types and their
//! settings (after `client/src/SourceConfigFields.tsx` and `EditPanel.tsx`),
//! the form's draft and the bodies it sends, row order, sorting and search,
//! and how per-tag shortcut combos read and are sent. A recorded keystroke
//! becomes a combo in the web client's format with
//! `console::shortcuts::from_keystroke`, the console's one normaliser.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Map, Value, json};
use voltrus_scada_native::api::DataSource;

/// How the web client groups the source types (`types.ts::SOURCE_CATEGORY`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Live data from a device.
    Real,
    /// Generated on the server.
    Simulated,
    /// Polled over HTTP.
    Api,
    /// No data: image and text cards.
    Static,
}

impl Category {
    /// The short tag the table and the form show.
    pub fn tag(self) -> &'static str {
        match self {
            Self::Real => "REAL",
            Self::Simulated => "SIM",
            Self::Api => "API",
            Self::Static => "NONE",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Real => "Real data",
            Self::Simulated => "Simulate",
            Self::Api => "Other",
            Self::Static => "Other",
        }
    }
}

pub struct SourceType {
    pub id: &'static str,
    pub label: &'static str,
    pub category: Category,
}

/// Every source type the server generates or reads, in the web client's
/// order: real data, then simulations, then the rest.
pub const SOURCE_TYPES: &[SourceType] = &[
    SourceType {
        id: "modbus",
        label: "Modbus",
        category: Category::Real,
    },
    SourceType {
        id: "random_walk",
        label: "Random walk",
        category: Category::Simulated,
    },
    SourceType {
        id: "sine",
        label: "Sine wave",
        category: Category::Simulated,
    },
    SourceType {
        id: "step",
        label: "Step / spike",
        category: Category::Simulated,
    },
    SourceType {
        id: "sawtooth",
        label: "Sawtooth",
        category: Category::Simulated,
    },
    SourceType {
        id: "constant",
        label: "Constant",
        category: Category::Simulated,
    },
    SourceType {
        id: "noise",
        label: "Noise",
        category: Category::Simulated,
    },
    SourceType {
        id: "api",
        label: "API (HTTP poll)",
        category: Category::Api,
    },
    SourceType {
        id: "none",
        label: "None (static)",
        category: Category::Static,
    },
];

pub fn source_type(id: &str) -> Option<&'static SourceType> {
    SOURCE_TYPES.iter().find(|kind| kind.id == id)
}

/// A source's label for the table: its type's name, or the raw id of a
/// type this console does not know (an edition's protocol, say).
pub fn type_label(id: &str) -> &str {
    source_type(id).map_or(id, |kind| kind.label)
}

/// The card kinds a source can draw as (`types.ts::CHART_TYPES`).
pub const CHART_TYPES: [&str; 5] = ["sparkline", "bignumber", "gauge", "image", "text"];

/// Image and text cards show no number: the form hides the unit, limits and
/// delays for them, as the web client does.
pub fn shows_numbers(chart_type: &str) -> bool {
    !matches!(chart_type, "image" | "text")
}

pub const API_METHODS: [&str; 2] = ["GET", "POST"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Number,
    Integer,
    Text,
    /// JSON text, stored as the value it parses to.
    Json,
}

/// One per-type setting, stored under `key` in `source_config`.
pub struct ConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    /// What the server uses when the setting is absent; shown as the
    /// placeholder, so an untouched field stays absent like on the web.
    pub placeholder: &'static str,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

const fn number(
    key: &'static str,
    label: &'static str,
    placeholder: &'static str,
    min: Option<f64>,
    max: Option<f64>,
) -> ConfigField {
    ConfigField {
        key,
        label,
        kind: FieldKind::Number,
        placeholder,
        min,
        max,
    }
}

const RANDOM_WALK: &[ConfigField] = &[
    number("volatility", "Volatility", "0.06", Some(0.01), Some(0.5)),
    number("reversion", "Reversion", "0.03", Some(0.01), Some(0.2)),
];
const SINE: &[ConfigField] = &[
    number("amplitude", "Amplitude", "5", None, None),
    number("frequency", "Frequency (Hz)", "0.5", None, None),
    number("offset", "Phase offset", "0", None, None),
];
const STEP: &[ConfigField] = &[
    number(
        "jump_probability",
        "Jump probability",
        "0.02",
        Some(0.0),
        Some(1.0),
    ),
    number("settle_time", "Settle time (s)", "5", Some(1.0), None),
];
const SAWTOOTH: &[ConfigField] = &[number("period", "Period (s)", "10", Some(1.0), None)];
const NOISE: &[ConfigField] = &[number(
    "noise_amplitude",
    "Noise amplitude",
    "3",
    None,
    None,
)];
const API: &[ConfigField] = &[
    ConfigField {
        key: "url",
        label: "URL",
        kind: FieldKind::Text,
        placeholder: "https://example.com/api/v1/sensor",
        min: None,
        max: None,
    },
    ConfigField {
        key: "path",
        label: "JSON path",
        kind: FieldKind::Text,
        placeholder: "data.value",
        min: None,
        max: None,
    },
    ConfigField {
        key: "poll_interval",
        label: "Poll interval (ms)",
        kind: FieldKind::Integer,
        placeholder: "5000",
        min: Some(1000.0),
        max: None,
    },
    ConfigField {
        key: "headers",
        label: "Headers (JSON)",
        kind: FieldKind::Json,
        placeholder: "{}",
        min: None,
        max: None,
    },
];

/// The settings a source type has (the API method and the Modbus device,
/// register and expression are choices of their own).
pub fn config_fields(source_type: &str) -> &'static [ConfigField] {
    match source_type {
        "random_walk" => RANDOM_WALK,
        "sine" => SINE,
        "step" => STEP,
        "sawtooth" => SAWTOOTH,
        "noise" => NOISE,
        "api" => API,
        _ => &[],
    }
}

/// Every setting of every type, once: the form makes one field each.
pub fn all_config_fields() -> impl Iterator<Item = &'static ConfigField> {
    [RANDOM_WALK, SINE, STEP, SAWTOOTH, NOISE, API]
        .into_iter()
        .flatten()
}

/// What the form says for a type without settings.
pub fn type_note(source_type: &str) -> Option<&'static str> {
    match source_type {
        "constant" => Some("Always returns the base value."),
        "none" => Some("No data source. Use it for image cards or static displays."),
        _ => None,
    }
}

/// A Modbus source reads one register, or computes an expression over the
/// device's registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModbusMode {
    Single,
    Expression,
}

/// The form's values as typed. Numbers stay text until saving, so a field
/// can be half-typed; [`Draft::validate`] says what is wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub name: String,
    pub key: String,
    pub unit: String,
    pub color: String,
    pub min: String,
    pub max: String,
    pub warn: String,
    pub crit: String,
    pub base: String,
    pub decimals: String,
    pub low_warn: String,
    pub low_crit: String,
    pub deadband: String,
    pub on_delay: String,
    pub off_delay: String,
    pub chart_type: String,
    pub source_type: String,
    /// Per-type settings by `ConfigField::key`, as typed.
    pub settings: BTreeMap<&'static str, String>,
    pub method: String,
    pub modbus_device: Option<String>,
    pub modbus_mode: ModbusMode,
    pub register: Option<String>,
    pub expression: String,
    pub image_url: String,
    pub text_content: String,
    /// `source_config` keys the form does not show, kept as they were.
    pub extra: Map<String, Value>,
    /// The device id of a source that is not a Modbus one, kept.
    pub device_id: String,
}

/// A new source's defaults: the web client's "Create source" row.
pub const NEW_COLOR: &str = "#8b5cf6";

/// Colours offered next to the colour field.
pub const COLOR_PRESETS: [&str; 8] = [
    "#8b5cf6", "#3b82f6", "#06b6d4", "#22c55e", "#f59e0b", "#f97316", "#ef4444", "#ec4899",
];

/// A stored value as the form shows it: `5`, `0.06`, `GET`, `{"a":1}`.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Number(number) => number
            .as_i64()
            .map(|integer| integer.to_string())
            .or_else(|| number.as_f64().map(format_number))
            .unwrap_or_default(),
        other => other.to_string(),
    }
}

/// `5` for 5.0, `0.06` for 0.06.
pub fn format_number(value: f64) -> String {
    format!("{value}")
}

/// A configured value for the table, plain (150000, never 1.5e5) and at
/// most `decimals` places with trailing zeros dropped: `932.63`, `61000`.
pub fn format_limit(value: f64, decimals: i64) -> String {
    let places = usize::try_from(decimals.clamp(0, 6)).unwrap_or(0);
    let text = format!("{value:.places$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// The JSON object in a `source_config` text, or an empty one.
pub fn parse_config(text: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(map)) => map,
        _ => Map::new(),
    }
}

impl Draft {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            key: String::new(),
            unit: String::new(),
            color: NEW_COLOR.into(),
            min: "0".into(),
            max: "100".into(),
            warn: "80".into(),
            crit: "90".into(),
            base: "50".into(),
            decimals: "1".into(),
            low_warn: String::new(),
            low_crit: String::new(),
            deadband: String::new(),
            on_delay: "0".into(),
            off_delay: "0".into(),
            chart_type: "sparkline".into(),
            source_type: "random_walk".into(),
            settings: BTreeMap::new(),
            method: "GET".into(),
            modbus_device: None,
            modbus_mode: ModbusMode::Single,
            register: None,
            expression: String::new(),
            image_url: String::new(),
            text_content: String::new(),
            extra: Map::new(),
            device_id: String::new(),
        }
    }

    pub fn from_source(source: &DataSource) -> Self {
        let mut config = parse_config(&source.source_config);
        let mut take = |key: &str| config.remove(key).map(|value| text_of(&value));
        let low_warn = take("low_warn").unwrap_or_default();
        let low_crit = take("low_crit").unwrap_or_default();
        let deadband = take("deadband").unwrap_or_default();
        let image_url = take("image_url").unwrap_or_default();
        let text_content = take("text_content").unwrap_or_default();
        let method = take("method").filter(|method| !method.is_empty());
        let config_device = take("modbus_device_id").filter(|id| !id.is_empty());
        let stored_mode = take("modbus_mode");
        let register = take("modbus_register_name").filter(|name| !name.is_empty());
        let expression = take("expression").unwrap_or_default();
        let modbus = source.source_type == "modbus";
        // The web client's rule: the stored mode, else an expression if
        // there is one.
        let modbus_mode = match stored_mode.as_deref() {
            Some("expression") => ModbusMode::Expression,
            Some("single") => ModbusMode::Single,
            _ if !expression.is_empty() => ModbusMode::Expression,
            _ => ModbusMode::Single,
        };
        let mut settings = BTreeMap::new();
        for field in config_fields(&source.source_type) {
            if let Some(value) = config.remove(field.key) {
                settings.insert(field.key, text_of(&value));
            }
        }
        let modbus_device = if modbus {
            config_device.or_else(|| {
                (!source.device_id.is_empty() && source.device_id != "default")
                    .then(|| source.device_id.clone())
            })
        } else {
            None
        };
        Self {
            name: source.name.clone(),
            key: source.key.clone(),
            unit: source.unit.clone(),
            color: source.color.clone(),
            min: format_number(source.min_val),
            max: format_number(source.max_val),
            warn: format_number(source.warn_val),
            crit: format_number(source.crit_val),
            base: format_number(source.base_val),
            decimals: source.decimals.to_string(),
            low_warn,
            low_crit,
            deadband,
            on_delay: source.on_delay_secs.to_string(),
            off_delay: source.off_delay_secs.to_string(),
            chart_type: if source.chart_type.is_empty() {
                "sparkline".into()
            } else {
                source.chart_type.clone()
            },
            source_type: source.source_type.clone(),
            settings,
            method: method.unwrap_or_else(|| "GET".into()),
            modbus_device,
            modbus_mode,
            register,
            expression,
            image_url,
            text_content,
            extra: config,
            device_id: if modbus {
                String::new()
            } else {
                source.device_id.clone()
            },
        }
    }

    /// Switch the source type: the old type's settings go, as on the web;
    /// the limits, deadband and card content stay (they are not per type).
    pub fn set_source_type(&mut self, id: &str) {
        if self.source_type != id {
            self.source_type = id.to_owned();
            self.settings.clear();
            self.extra.clear();
            self.modbus_device = None;
            self.register = None;
            self.expression.clear();
            self.modbus_mode = ModbusMode::Single;
        }
    }

    /// Pick a Modbus register: like the web form, the key follows it.
    pub fn pick_register(&mut self, name: &str) {
        self.register = Some(name.to_owned());
        self.key = name.to_owned();
    }

    /// The `source_config` object the draft stands for.
    pub fn config(&self) -> Result<Map<String, Value>, String> {
        let mut config = self.extra.clone();
        for field in config_fields(&self.source_type) {
            let text = self.settings.get(field.key).map_or("", |text| text.trim());
            if text.is_empty() {
                continue;
            }
            let value = match field.kind {
                FieldKind::Number => json!(parse_number(field.label, text, field.min, field.max)?),
                FieldKind::Integer => {
                    let value: i64 = text
                        .parse()
                        .map_err(|_| format!("{} must be a whole number", field.label))?;
                    #[allow(clippy::cast_precision_loss)]
                    in_range(field.label, value as f64, field.min, field.max)?;
                    json!(value)
                }
                FieldKind::Text => json!(text),
                FieldKind::Json => serde_json::from_str::<Value>(text)
                    .map_err(|_| format!("{} must be JSON", field.label))?,
            };
            config.insert(field.key.into(), value);
        }
        match self.source_type.as_str() {
            "api" => {
                config.insert("method".into(), json!(self.method));
            }
            "modbus" if self.modbus_device.is_some() => {
                let expression = self.modbus_mode == ModbusMode::Expression;
                config.insert(
                    "modbus_mode".into(),
                    json!(if expression { "expression" } else { "single" }),
                );
                config.insert(
                    "modbus_register_name".into(),
                    json!(if expression {
                        ""
                    } else {
                        self.register.as_deref().unwrap_or("")
                    }),
                );
                config.insert(
                    "expression".into(),
                    json!(if expression {
                        self.expression.trim()
                    } else {
                        ""
                    }),
                );
            }
            _ => {}
        }
        for (key, text, min) in [
            ("low_warn", &self.low_warn, None),
            ("low_crit", &self.low_crit, None),
            ("deadband", &self.deadband, Some(0.0)),
        ] {
            let text = text.trim();
            if !text.is_empty() {
                let label = match key {
                    "low_warn" => "Low warning",
                    "low_crit" => "Low critical",
                    _ => "Deadband",
                };
                config.insert(key.into(), json!(parse_number(label, text, min, None)?));
            }
        }
        for (key, text) in [
            ("image_url", &self.image_url),
            ("text_content", &self.text_content),
        ] {
            if !text.trim().is_empty() {
                config.insert(key.into(), json!(text.trim()));
            }
        }
        Ok(config)
    }

    /// The fields every body carries, checked.
    fn common(&self) -> Result<Map<String, Value>, String> {
        let key = self.key.trim();
        if key.is_empty() {
            return Err("The key is required".into());
        }
        let name = match self.name.trim() {
            "" => key,
            name => name,
        };
        if parse_hex(&self.color).is_none() {
            return Err("The colour must look like #8b5cf6".into());
        }
        let decimals: i64 = self
            .decimals
            .trim()
            .parse()
            .ok()
            .filter(|decimals| (0..=10).contains(decimals))
            .ok_or("Decimals must be a whole number from 0 to 10")?;
        let mut body = Map::new();
        body.insert("key".into(), json!(key));
        body.insert("name".into(), json!(name));
        body.insert("unit".into(), json!(self.unit.trim()));
        body.insert("color".into(), json!(self.color.trim().to_lowercase()));
        for (field, label, text) in [
            ("min_val", "Minimum", &self.min),
            ("max_val", "Maximum", &self.max),
            ("warn_val", "Warning", &self.warn),
            ("crit_val", "Critical", &self.crit),
            ("base_val", "Base", &self.base),
        ] {
            body.insert(field.into(), json!(parse_number(label, text, None, None)?));
        }
        body.insert("decimals".into(), json!(decimals));
        body.insert("source_type".into(), json!(self.source_type));
        body.insert("chart_type".into(), json!(self.chart_type));
        let device = if self.source_type == "modbus" {
            self.modbus_device.clone().unwrap_or_default()
        } else {
            self.device_id.clone()
        };
        if !device.is_empty() {
            body.insert("device_id".into(), json!(device));
        }
        Ok(body)
    }

    /// The ISA-18.2 alarm delays, in whole seconds.
    pub fn delays(&self) -> Result<(i64, i64), String> {
        let parse = |label: &str, text: &str| {
            text.trim()
                .parse::<i64>()
                .ok()
                .filter(|seconds| *seconds >= 0)
                .ok_or_else(|| format!("{label} must be whole seconds, 0 or more"))
        };
        Ok((
            parse("The on delay", &self.on_delay)?,
            parse("The off delay", &self.off_delay)?,
        ))
    }

    /// Everything wrong with the draft, first problem first.
    pub fn validate(&self) -> Result<(), String> {
        self.common()?;
        self.config()?;
        self.delays()?;
        Ok(())
    }

    /// `POST /api/v1/data-sources`. The server stores an object
    /// `source_config` as its JSON text (a string as-is). The server ignores
    /// the delays here: they follow with [`Draft::delays_body`].
    pub fn create_body(&self) -> Result<Value, String> {
        let mut body = self.common()?;
        body.insert("source_config".into(), Value::Object(self.config()?));
        Ok(Value::Object(body))
    }

    /// `PUT /api/v1/data-sources/{id}` for an edit of `original`, with the
    /// row's text form of `source_config`. The settings and the device go
    /// only when they changed; the sort order and tag path never, so an edit
    /// does not undo a reorder or a tag tree move made meanwhile.
    pub fn update_body(&self, original: &DataSource) -> Result<Value, String> {
        let mut body = self.common()?;
        let config = self.config()?;
        let before = Draft::from_source(original).config().unwrap_or_default();
        if config != before || self.source_type != original.source_type {
            body.insert(
                "source_config".into(),
                json!(Value::Object(config).to_string()),
            );
        }
        if body.get("device_id").and_then(Value::as_str) == Some(original.device_id.as_str()) {
            body.remove("device_id");
        }
        let (on, off) = self.delays()?;
        body.insert("on_delay_secs".into(), json!(on));
        body.insert("off_delay_secs".into(), json!(off));
        Ok(Value::Object(body))
    }

    /// The delays for a `PUT` after creating, if they are not the default.
    pub fn delays_body(&self) -> Result<Option<Value>, String> {
        let (on, off) = self.delays()?;
        Ok((on != 0 || off != 0).then(|| json!({ "on_delay_secs": on, "off_delay_secs": off })))
    }
}

impl Default for Draft {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_number(
    label: &str,
    text: &str,
    min: Option<f64>,
    max: Option<f64>,
) -> Result<f64, String> {
    let value: f64 = text
        .trim()
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite())
        .ok_or_else(|| format!("{label} must be a number"))?;
    in_range(label, value, min, max)
}

fn in_range(label: &str, value: f64, min: Option<f64>, max: Option<f64>) -> Result<f64, String> {
    if let Some(min) = min
        && value < min
    {
        return Err(format!("{label} must be {} or more", format_number(min)));
    }
    if let Some(max) = max
        && value > max
    {
        return Err(format!("{label} must be {} or less", format_number(max)));
    }
    Ok(value)
}

/// `#rrggbb` (or `rrggbb`) as a number for `gpui::rgb`.
pub fn parse_hex(text: &str) -> Option<u32> {
    let hex = text.trim().trim_start_matches('#');
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

/// The web client's duplicate (`App.tsx::handleDuplicateSensor`): the same
/// source under `<key>_copy`, named `<name> copy` (the server's default
/// "permissive" name rule allows only letters, digits, space, `_`, `.` and
/// `-`, so no parentheses; the web names its duplicates the same way). The config goes as an object
/// (see [`Draft::create_body`]); the delays follow separately.
pub fn duplicate_body(source: &DataSource) -> Value {
    let mut body = json!({
        "key": format!("{}_copy", source.key),
        "name": format!("{} copy", source.name),
        "unit": source.unit,
        "color": source.color,
        "min_val": source.min_val,
        "max_val": source.max_val,
        "warn_val": source.warn_val,
        "crit_val": source.crit_val,
        "base_val": source.base_val,
        "decimals": source.decimals,
        "source_type": source.source_type,
        "source_config": Value::Object(parse_config(&source.source_config)),
        "chart_type": source.chart_type,
    });
    if !source.device_id.is_empty() {
        body["device_id"] = json!(source.device_id);
    }
    body
}

/// The delays of `source` for a `PUT` after duplicating, if any.
pub fn duplicate_delays(source: &DataSource) -> Option<Value> {
    (source.on_delay_secs != 0 || source.off_delay_secs != 0).then(|| {
        json!({
            "on_delay_secs": source.on_delay_secs,
            "off_delay_secs": source.off_delay_secs,
        })
    })
}

// ── Order, sorting, search ───────────────────────────────────────────────

/// What the table sorts by. `Order` is the server's `sort_order`, the order
/// every page shows the tags in; only it can be rearranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Order,
    Name,
    Key,
    Unit,
    Source,
    Chart,
}

impl SortKey {
    pub const ALL: [Self; 6] = [
        Self::Order,
        Self::Name,
        Self::Key,
        Self::Unit,
        Self::Source,
        Self::Chart,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Order => "order",
            Self::Name => "name",
            Self::Key => "key",
            Self::Unit => "unit",
            Self::Source => "source",
            Self::Chart => "chart",
        }
    }

    /// The next key, for the `s` shortcut.
    #[must_use]
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|key| *key == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
}

fn compare_text(a: &str, b: &str) -> std::cmp::Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// The table's rows: `filtered` (positions in the server's order, already
/// matching the search) sorted. `order`, while a reorder waits for the
/// server, is the order the operator asked for and replaces the server's.
/// `info` gives the source at a position.
pub fn sort_rows<'a>(
    filtered: &[usize],
    info: impl Fn(usize) -> &'a DataSource,
    sort: SortKey,
    descending: bool,
    order: Option<&[i64]>,
) -> Vec<usize> {
    let mut rows = filtered.to_vec();
    if let Some(order) = order {
        let at: HashMap<i64, usize> = order
            .iter()
            .enumerate()
            .map(|(position, id)| (*id, position))
            .collect();
        rows.sort_by_key(|index| at.get(&info(*index).id).copied().unwrap_or(usize::MAX));
    }
    let by = |a: &usize, b: &usize| {
        let (a, b) = (info(*a), info(*b));
        match sort {
            SortKey::Order => std::cmp::Ordering::Equal,
            SortKey::Name => compare_text(&a.name, &b.name),
            SortKey::Key => compare_text(&a.key, &b.key),
            SortKey::Unit => compare_text(&a.unit, &b.unit),
            SortKey::Source => compare_text(&a.source_type, &b.source_type),
            SortKey::Chart => compare_text(&a.chart_type, &b.chart_type),
        }
    };
    // Stable, so equal names keep the server's order.
    rows.sort_by(by);
    if descending {
        rows.reverse();
    }
    rows
}

/// The web client's drag-and-drop reorder (`SensorManager.tsx`): move the
/// row at `from` to `to` among the `visible` rows (the search may hide
/// some), keeping hidden rows where they are in `all`. The new order of
/// every id, or `None` when nothing moves.
pub fn reordered(all: &[i64], visible: &[i64], from: usize, to: usize) -> Option<Vec<i64>> {
    if from == to || from >= visible.len() || to >= visible.len() {
        return None;
    }
    let mut moved = visible.to_vec();
    let id = moved.remove(from);
    moved.insert(to, id);
    let shown: std::collections::HashSet<i64> = visible.iter().copied().collect();
    let mut next = moved.into_iter();
    Some(
        all.iter()
            .map(|id| {
                if shown.contains(id) {
                    next.next().unwrap_or(*id)
                } else {
                    *id
                }
            })
            .collect(),
    )
}

// ── Per-tag shortcuts ────────────────────────────────────────────────────

/// A combo as the operator reads it: `⇧⌘K` on macOS, `Ctrl+Shift+K`
/// elsewhere.
pub fn display_combo(combo: &str) -> String {
    let mac = cfg!(target_os = "macos");
    // `Mod+ ` (Space) splits into an empty last part.
    let (modifiers, key) = match combo.rsplit_once('+') {
        Some((modifiers, "")) => (modifiers.trim_end_matches('+'), "+"),
        Some((modifiers, key)) => (modifiers, key),
        None => ("", combo),
    };
    let key = match key {
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "Enter" => "↵",
        " " => "Space",
        "Escape" => "Esc",
        other => other,
    };
    let has = |name: &str| modifiers.split('+').any(|part| part == name);
    if mac {
        let mut out = String::new();
        for (name, symbol) in [("Alt", "⌥"), ("Shift", "⇧"), ("Mod", "⌘")] {
            if has(name) {
                out.push_str(symbol);
            }
        }
        out.push_str(key);
        out
    } else {
        let mut parts: Vec<&str> = Vec::new();
        for (name, label) in [("Mod", "Ctrl"), ("Alt", "Alt"), ("Shift", "Shift")] {
            if has(name) {
                parts.push(label);
            }
        }
        parts.push(key);
        parts.join("+")
    }
}

/// `PUT /api/v1/shortcuts` replaces the user's whole map (a tag left out or
/// sent as `null` has no shortcut), so the body is the current map with `id`
/// set to `combo` (or left out), empty ones dropped.
pub fn shortcuts_body(current: &HashMap<i64, String>, id: i64, combo: Option<&str>) -> Value {
    let mut map = Map::new();
    let mut ids: Vec<&i64> = current.keys().collect();
    ids.sort_unstable();
    for other in ids {
        if *other != id && !current[other].is_empty() {
            map.insert(other.to_string(), json!(current[other]));
        }
    }
    if let Some(combo) = combo.filter(|combo| !combo.is_empty()) {
        map.insert(id.to_string(), json!(combo));
    }
    Value::Object(map)
}

// ── Modbus expressions ───────────────────────────────────────────────────

/// The expression with `operand` appended after `op`: the web builder's
/// `+ {Reg_B}` terms. An empty expression takes the operand alone.
pub fn append_term(expression: &str, op: char, operand: &str) -> String {
    let expression = expression.trim_end();
    if expression.is_empty() {
        operand.to_owned()
    } else {
        format!("{expression} {op} {operand}")
    }
}

/// A register as an expression operand: `{name}`.
pub fn register_operand(name: &str) -> String {
    format!("{{{name}}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: i64, key: &str, name: &str) -> DataSource {
        DataSource {
            id,
            key: key.into(),
            name: name.into(),
            ..DataSource::default()
        }
    }

    #[test]
    fn a_new_draft_posts_the_web_defaults_with_the_config_as_an_object() {
        let mut draft = Draft::new();
        draft.key = "  line_temp ".into();
        draft.settings.insert("volatility", "0.1".into());
        let body = draft.create_body().unwrap();
        assert_eq!(body["key"], "line_temp");
        assert_eq!(body["name"], "line_temp", "an empty name takes the key");
        assert_eq!(body["color"], "#8b5cf6");
        assert_eq!(body["warn_val"], 80.0);
        assert_eq!(body["decimals"], 1);
        assert_eq!(body["source_type"], "random_walk");
        assert_eq!(body["source_config"], json!({ "volatility": 0.1 }));
        assert!(
            body.get("device_id").is_none(),
            "the server picks the default"
        );
        assert_eq!(draft.delays_body().unwrap(), None);
        draft.on_delay = "5".into();
        assert_eq!(
            draft.delays_body().unwrap(),
            Some(json!({ "on_delay_secs": 5, "off_delay_secs": 0 }))
        );
    }

    #[test]
    fn untouched_settings_stay_absent_and_bad_ones_are_named() {
        let mut draft = Draft::new();
        draft.key = "k".into();
        assert_eq!(draft.config().unwrap(), Map::new());
        draft.settings.insert("volatility", "0.9".into());
        assert_eq!(
            draft.validate(),
            Err("Volatility must be 0.5 or less".into())
        );
        draft.settings.insert("volatility", "lots".into());
        assert_eq!(draft.validate(), Err("Volatility must be a number".into()));
        draft.settings.clear();
        draft.key = " ".into();
        assert_eq!(draft.validate(), Err("The key is required".into()));
        draft.key = "k".into();
        draft.color = "purple".into();
        assert!(draft.validate().unwrap_err().contains("colour"));
        draft.color = "#22C55E".into();
        draft.off_delay = "-1".into();
        assert!(draft.validate().unwrap_err().contains("off delay"));
        draft.off_delay = "0".into();
        draft.set_source_type("api");
        draft.settings.insert("headers", "{not json".into());
        assert_eq!(draft.validate(), Err("Headers (JSON) must be JSON".into()));
        draft
            .settings
            .insert("headers", r#"{"Authorization":"Bearer x"}"#.into());
        draft.settings.insert("poll_interval", "500".into());
        assert_eq!(
            draft.validate(),
            Err("Poll interval (ms) must be 1000 or more".into())
        );
        draft.settings.insert("poll_interval", "2000".into());
        draft.method = "POST".into();
        assert_eq!(
            draft.config().unwrap(),
            json!({
                "headers": { "Authorization": "Bearer x" },
                "poll_interval": 2000,
                "method": "POST",
            })
            .as_object()
            .unwrap()
            .clone()
        );
    }

    #[test]
    fn an_edit_keeps_unknown_settings_and_sends_the_row_form() {
        let stored = DataSource {
            id: 7,
            key: "ac_current_l1".into(),
            name: "AC current L1".into(),
            color: "#f97316".into(),
            max_val: 120.0,
            warn_val: 80.0,
            crit_val: 85.0,
            decimals: 2,
            source_type: "modbus".into(),
            source_config:
                r#"{"address":40001,"low_warn":2,"modbus_register_name":"ac_current_l1"}"#.into(),
            device_id: "inverter-1".into(),
            chart_type: "gauge".into(),
            on_delay_secs: 3,
            ..DataSource::default()
        };
        let draft = Draft::from_source(&stored);
        assert_eq!(draft.max, "120");
        assert_eq!(draft.low_warn, "2");
        assert_eq!(draft.modbus_device.as_deref(), Some("inverter-1"));
        assert_eq!(draft.register.as_deref(), Some("ac_current_l1"));
        assert_eq!(draft.modbus_mode, ModbusMode::Single);
        let untouched = draft.update_body(&stored).unwrap();
        assert_eq!(untouched["on_delay_secs"], 3);
        assert!(
            untouched.get("sort_order").is_none(),
            "a reorder is not undone"
        );
        assert!(
            untouched.get("source_config").is_none() && untouched.get("device_id").is_none(),
            "settings nobody touched are not sent"
        );
        let mut edited = draft.clone();
        edited.low_warn = "3".into();
        edited.modbus_device = Some("inverter-2".into());
        let body = edited.update_body(&stored).unwrap();
        assert_eq!(body["device_id"], "inverter-2");
        let config = parse_config(body["source_config"].as_str().unwrap());
        assert_eq!(config["address"], 40001, "unknown keys survive");
        assert_eq!(config["low_warn"], 3.0);
        assert_eq!(config["modbus_mode"], "single");
        assert_eq!(config["modbus_register_name"], "ac_current_l1");
        assert_eq!(config["expression"], "");
    }

    #[test]
    fn switching_type_drops_its_settings_but_keeps_the_limits() {
        let stored = DataSource {
            source_type: "sine".into(),
            source_config: r#"{"amplitude":4,"low_crit":1,"deadband":0.5}"#.into(),
            key: "s".into(),
            color: "#000000".into(),
            ..DataSource::default()
        };
        let mut draft = Draft::from_source(&stored);
        assert_eq!(
            draft.settings.get("amplitude").map(String::as_str),
            Some("4")
        );
        draft.set_source_type("noise");
        let config = draft.config().unwrap();
        assert!(config.get("amplitude").is_none());
        assert_eq!(config["low_crit"], 1.0);
        assert_eq!(config["deadband"], 0.5);
    }

    #[test]
    fn modbus_expressions_follow_the_web_rule() {
        let stored = DataSource {
            source_type: "modbus".into(),
            source_config: r#"{"modbus_device_id":"plc","expression":"{A} + {B}"}"#.into(),
            device_id: "default".into(),
            ..DataSource::default()
        };
        let mut draft = Draft::from_source(&stored);
        assert_eq!(draft.modbus_mode, ModbusMode::Expression);
        assert_eq!(draft.modbus_device.as_deref(), Some("plc"));
        draft.pick_register("Reg_C");
        assert_eq!(draft.key, "Reg_C", "the key follows the register");
        assert_eq!(
            append_term(&draft.expression, '*', &register_operand("Reg_C")),
            "{A} + {B} * {Reg_C}"
        );
        assert_eq!(append_term("  ", '+', "{A}"), "{A}");
    }

    #[test]
    fn a_duplicate_is_the_web_copy_with_an_object_config() {
        let mut original = source(3, "flow", "Flow");
        original.source_config = r#"{"amplitude":2}"#.into();
        original.device_id = "plc".into();
        let body = duplicate_body(&original);
        assert_eq!(body["key"], "flow_copy");
        assert_eq!(body["name"], "Flow copy");
        assert_eq!(body["source_config"], json!({ "amplitude": 2 }));
        assert_eq!(body["device_id"], "plc");
        assert_eq!(duplicate_delays(&original), None);
        original.off_delay_secs = 4;
        assert_eq!(
            duplicate_delays(&original),
            Some(json!({ "on_delay_secs": 0, "off_delay_secs": 4 }))
        );
    }

    #[test]
    fn rows_sort_stably_and_show_a_pending_reorder() {
        let list = [
            source(1, "b_key", "beta"),
            source(2, "a_key", "Alpha"),
            source(3, "c_key", "alpha"),
        ];
        let info = |index: usize| &list[index];
        let all = [0, 1, 2];
        assert_eq!(
            sort_rows(&all, info, SortKey::Order, false, None),
            [0, 1, 2]
        );
        assert_eq!(sort_rows(&all, info, SortKey::Name, false, None), [1, 2, 0]);
        assert_eq!(sort_rows(&all, info, SortKey::Key, true, None), [2, 0, 1]);
        assert_eq!(
            sort_rows(&[1, 2], info, SortKey::Order, false, None),
            [1, 2],
            "only the search's rows"
        );
        assert_eq!(
            sort_rows(&all, info, SortKey::Order, false, Some(&[3, 1, 2])),
            [2, 0, 1],
            "a pending reorder shows at once"
        );
        assert_eq!(SortKey::Chart.next(), SortKey::Order);
    }

    #[test]
    fn reordering_moves_among_visible_rows_and_keeps_hidden_ones() {
        // The web client's handleDragEnd, with 20 hidden by the search.
        let all = [10, 20, 30, 40];
        let visible = [10, 30, 40];
        assert_eq!(reordered(&all, &visible, 2, 0), Some(vec![40, 20, 10, 30]));
        assert_eq!(reordered(&all, &all, 0, 1), Some(vec![20, 10, 30, 40]));
        assert_eq!(reordered(&all, &visible, 1, 1), None);
        assert_eq!(reordered(&all, &visible, 0, 3), None);
    }

    #[test]
    fn combos_read_as_keys() {
        if cfg!(target_os = "macos") {
            assert_eq!(display_combo("Mod+Shift+ArrowUp"), "⇧⌘↑");
            assert_eq!(display_combo("Mod+Alt+K"), "⌥⌘K");
            assert_eq!(display_combo("Mod+ "), "⌘Space");
        } else {
            assert_eq!(display_combo("Mod+Shift+ArrowUp"), "Ctrl+Shift+↑");
        }
        assert_eq!(display_combo("F2"), "F2");
    }

    #[test]
    fn a_shortcut_change_sends_the_whole_map_without_empty_entries() {
        let current = HashMap::from([
            (1, "Mod+1".to_owned()),
            (2, String::new()),
            (3, "F3".into()),
        ]);
        assert_eq!(
            shortcuts_body(&current, 1, None),
            json!({ "3": "F3" }),
            "clearing leaves the entry out"
        );
        assert_eq!(
            shortcuts_body(&current, 4, Some("Alt+K")),
            json!({ "1": "Mod+1", "3": "F3", "4": "Alt+K" })
        );
    }

    #[test]
    fn categories_colours_and_charts() {
        assert_eq!(format_limit(932.630_380_095_3, 2), "932.63");
        assert_eq!(format_limit(61_000.0, 1), "61000");
        assert_eq!(format_limit(150_000.0, 0), "150000");
        assert_eq!(
            format_limit(0.5, 0),
            "0",
            "rounded to the source's decimals"
        );
        assert_eq!(format_limit(50.15, 2), "50.15");
        assert_eq!(source_type("modbus").unwrap().category, Category::Real);
        assert_eq!(type_label("opcua"), "opcua");
        assert_eq!(parse_hex("#8B5CF6"), Some(0x8b5cf6));
        assert_eq!(parse_hex("#8b5cf"), None);
        assert!(!shows_numbers("image") && shows_numbers("gauge"));
        assert_eq!(all_config_fields().count(), 13);
        assert!(COLOR_PRESETS.iter().all(|color| parse_hex(color).is_some()));
    }
}
