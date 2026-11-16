//! Data Manager › Flows: the server's flow routes (`src/api/flows.rs`), the
//! stored flow (`models::Flow`, `models::FlowRun`), the node-type catalogue
//! and the branching graph inside a flow, mirrored, with the edits the
//! native canvas editor makes to it.
//!
//! A stored flow carries its trigger and its graph as two JSON strings.
//! The graph is version 2: nodes with positions and type-specific
//! settings, and edges from an output port of one node (or of the virtual
//! `trigger` node) into the single input of another:
//!
//! ```json
//! { "version": 2,
//!   "nodes": [ { "id": "n1", "type": "filter", "x": 120, "y": 80, … } ],
//!   "edges": [ { "from": "trigger", "port": 0, "to": "n1" } ] }
//! ```
//!
//! An older server stores a plain array instead: the process steps run in
//! array order, every output step fed by the last one. [`FlowGraph::parse`]
//! turns that into the same graph, and [`FlowGraph::legacy_json`] writes a
//! graph back as an array when it is such a straight line, for servers
//! without the graph engine.
//!
//! Node settings stay JSON maps in the order the server stored them
//! (serde_json keeps key order here, as GPUI turns on `preserve_order`).

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::Result;
use serde::de::IgnoredAny;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

use super::ops::encode;
use super::{Method, ServerError, VoltrusApi};

/// The list route.
pub const FLOWS: &str = "/api/v1/flows";
/// `GET`: the node-type catalogue (graph servers only).
pub const NODE_TYPES: &str = "/api/v1/flows/node-types";
/// `POST`: a dry run of an unsaved flow (graph servers only).
pub const DRAFT_TEST: &str = "/api/v1/flows/test";
/// The virtual source node of every flow: its trigger.
pub const TRIGGER: &str = "trigger";

/// `GET`, `PUT`, `DELETE /api/v1/flows/{id}`.
pub fn flow_path(id: &str) -> String {
    format!("{FLOWS}/{}", encode(id))
}

/// `POST /api/v1/flows/{id}/start`, `/stop`, `/test`, `/run`; `GET …/runs`.
pub fn flow_action_path(id: &str, action: &str) -> String {
    format!("{}/{action}", flow_path(id))
}

/// `models::Flow`: the request body of `POST` and `PUT` and the rows of
/// `GET`. The server needs every field in a body.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Flow {
    pub id: String,
    pub name: String,
    /// Stored: whether the engine loads it at start-up. `start`/`stop`
    /// change the running engine only.
    pub enabled: bool,
    /// The trigger as JSON text: `{"type": "modbus", …}`.
    pub trigger_config: String,
    /// The graph as JSON text: version 2, or a legacy array of steps.
    pub nodes: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// `GET /api/v1/flows`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FlowList {
    #[serde(default)]
    pub flows: Vec<Flow>,
}

/// `POST /api/v1/flows`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FlowCreated {
    pub id: String,
}

/// One node of a logged run's path.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct RunStep {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub port: Option<usize>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub duration_us: u64,
}

/// A run's trace as the server sends it: a list, a list as JSON text, or
/// nothing (older servers).
fn lenient_trace<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<RunStep>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    let value = match value {
        Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::Null),
        other => other,
    };
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// `models::FlowRun`: one run the engine logged.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FlowRun {
    pub id: i64,
    #[serde(default)]
    pub flow_id: String,
    /// Unix seconds.
    pub started_at: i64,
    pub duration_ms: i64,
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// The last payload as JSON text, when the run went through.
    #[serde(default)]
    pub output: Option<String>,
    /// The nodes the run went through (graph servers).
    #[serde(default, deserialize_with = "lenient_trace")]
    pub trace: Vec<RunStep>,
}

impl FlowRun {
    /// `n1 → n3 (port 1) → n4 ✗`, for the runs list.
    pub fn path_text(&self) -> String {
        self.trace
            .iter()
            .map(|step| {
                let mut text = step.node_id.clone();
                if let Some(port) = step.port.filter(|port| *port > 0) {
                    text.push_str(&format!(" (port {port})"));
                }
                if step.error.is_some() {
                    text.push_str(" ✗");
                }
                text
            })
            .collect::<Vec<_>>()
            .join(" → ")
    }
}

/// `GET /api/v1/flows/{id}/runs`: the latest 20, newest first.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FlowRuns {
    #[serde(default)]
    pub runs: Vec<FlowRun>,
}

/// The legacy `POST /api/v1/flows/{id}/test`: the stored steps run once,
/// in array order, on the payload, outputs included.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct TestResult {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub node_results: Vec<NodeResult>,
}

/// One step of a legacy test run: `continue` with its output,
/// `filter_reject`, or an error (the run stops there).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct NodeResult {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl NodeResult {
    /// `continue`, `filter_reject` or `error`.
    pub fn outcome(&self) -> &str {
        match (&self.error, &self.result) {
            (Some(_), _) => "error",
            (None, Some(result)) => result.as_str(),
            (None, None) => "—",
        }
    }
}

/// One node of a dry run: what came in and went out, on which port.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct TraceStep {
    #[serde(default)]
    pub node_id: String,
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub output: Value,
    /// The output port the message left by; none when it stopped here.
    #[serde(default)]
    pub port: Option<usize>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub duration_us: u64,
    /// An output node that would have acted (write, alarm, HTTP): the dry
    /// run skipped it.
    #[serde(default)]
    pub side_effect_skipped: bool,
}

/// `POST /api/v1/flows/test`: a dry run of the flow sent.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct TestRun {
    #[serde(default)]
    pub ok: bool,
    /// Why the run failed, when it did.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub trace: Vec<TraceStep>,
    #[serde(default)]
    pub outputs_reached: Value,
}

impl TestRun {
    /// A legacy test's results as a trace: a step that continued left by
    /// port 0.
    pub fn from_legacy(result: &TestResult) -> Self {
        Self {
            ok: result.success,
            error: None,
            trace: result
                .node_results
                .iter()
                .map(|step| TraceStep {
                    node_id: step.node_id.clone(),
                    kind: String::new(),
                    input: Value::Null,
                    output: step.output.clone().unwrap_or(Value::Null),
                    port: (step.outcome() == "continue").then_some(0),
                    error: step.error.clone(),
                    duration_us: 0,
                    side_effect_skipped: false,
                })
                .collect(),
            outputs_reached: Value::Null,
        }
    }

    /// How many outputs the run reached, as the server counts them.
    pub fn outputs_text(&self) -> String {
        match &self.outputs_reached {
            Value::Number(number) => number.to_string(),
            Value::Array(items) => items.len().to_string(),
            Value::Null => "—".to_owned(),
            other => other.to_string(),
        }
    }
}

/// The body of the legacy `POST /api/v1/flows/{id}/test`.
pub fn test_body(payload: Value) -> Value {
    json!({ "payload": payload })
}

/// The body of `POST /api/v1/flows/test`.
pub fn draft_test_body(trigger_config: &str, nodes: &str, input: Value) -> Value {
    json!({
        "flow": { "trigger_config": trigger_config, "nodes": nodes },
        "input": input,
    })
}

/// The web client's default test payload.
pub const TEST_PAYLOAD: &str = r#"{"metrics": {"temp": 85}}"#;

/// The default dry-run input: a payload and the trigger's metadata.
pub const TEST_INPUT: &str =
    "{\n  \"payload\": {\"metrics\": {\"temp\": 85}},\n  \"metadata\": {}\n}";

/// A dry run's input from its text: an object with a `payload` (and
/// optional `metadata`).
pub fn parse_input(text: &str) -> Result<Value, String> {
    let value = parse_json(text)?;
    let Some(object) = value.as_object() else {
        return Err("the input must be an object: {\"payload\": …, \"metadata\": …}".into());
    };
    if !object.contains_key("payload") {
        return Err("the input needs a payload: {\"payload\": …}".into());
    }
    let mut input = object.clone();
    input
        .entry("metadata")
        .or_insert_with(|| Value::Object(Map::new()));
    Ok(Value::Object(input))
}

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

impl Flow {
    /// A new flow: the trigger's defaults, no nodes, never created yet.
    /// The empty graph is an empty array, which every server reads.
    pub fn blank(id: &str, name: &str, enabled: bool, trigger: &TriggerType, now: i64) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            enabled,
            trigger_config: pretty(&Value::Object(trigger.defaults())),
            nodes: pretty(&Value::Array(Vec::new())),
            created_at: 0,
            updated_at: now,
        }
    }

    /// The same flow renamed or enabled differently; its trigger and
    /// graph stay the text the server has.
    pub fn with_meta(&self, name: &str, enabled: bool, now: i64) -> Self {
        Self {
            name: name.to_owned(),
            enabled,
            updated_at: now,
            ..self.clone()
        }
    }

    /// The flow with `trigger_config` and `nodes` (the editor's save).
    pub fn with_texts(
        &self,
        name: &str,
        enabled: bool,
        trigger_config: String,
        nodes: String,
        now: i64,
    ) -> Self {
        Self {
            name: name.to_owned(),
            enabled,
            trigger_config,
            nodes,
            updated_at: now,
            ..self.clone()
        }
    }

    /// The trigger's `type`, if the text holds one.
    pub fn trigger_type(&self) -> Option<String> {
        serde_json::from_str::<Value>(&self.trigger_config)
            .ok()?
            .get("type")?
            .as_str()
            .map(str::to_owned)
    }

    /// How many nodes, if the text is a graph or a legacy array.
    pub fn node_count(&self) -> Option<usize> {
        match serde_json::from_str::<Value>(&self.nodes).ok()? {
            Value::Array(items) => Some(items.len()),
            Value::Object(graph) => graph.get("nodes")?.as_array().map(Vec::len),
            _ => None,
        }
    }

    /// The trigger's label for the list (`Modbus`), or its raw type.
    pub fn trigger_label(&self) -> String {
        match self.trigger_type() {
            Some(kind) => trigger_kind(&kind).map_or(kind, |known| known.label.to_owned()),
            None => "Unknown".to_owned(),
        }
    }
}

/// What the new-flow and settings form checks before sending: the web
/// client asks for an ID and a name. A new ID must be free, and usable in
/// the routes' path.
pub fn validate_meta(id: &str, name: &str, taken: Option<&[Flow]>) -> Result<(), String> {
    let id = id.trim();
    if id.is_empty() || name.trim().is_empty() {
        return Err("ID and name required".into());
    }
    if id == "." || id == ".." {
        return Err("The ID can't be . or ..".into());
    }
    if taken.is_some() && (id == "test" || id == "node-types") {
        return Err(format!("The ID {id} is reserved"));
    }
    if let Some(flows) = taken {
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        {
            return Err("The ID may hold letters, digits, - _ and . only".into());
        }
        if flows.iter().any(|flow| flow.id == id) {
            return Err(format!("A flow with the ID {id} exists"));
        }
    }
    Ok(())
}

// ── The catalogue ────────────────────────────────────────────────────────

/// How a setting is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    #[default]
    String,
    Number,
    Bool,
    Select,
    Expression,
    Code,
    Tag,
    Device,
    Json,
    DurationMs,
    Cron,
    Severity,
    /// A kind this console does not know: edited as JSON.
    #[serde(other)]
    Other,
}

impl FieldKind {
    /// Whether the value is text (a line or an area).
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::String
                | Self::Expression
                | Self::Code
                | Self::Tag
                | Self::Device
                | Self::Cron
                | Self::Select
                | Self::Severity
        )
    }

    /// Whether the value is a number.
    pub fn is_number(self) -> bool {
        matches!(self, Self::Number | Self::DurationMs)
    }

    /// Whether the value is edited as JSON text.
    pub fn is_json(self) -> bool {
        matches!(self, Self::Json | Self::Other)
    }
}

/// One choice of a select field.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct FieldOption {
    pub value: Value,
    #[serde(default)]
    pub label: String,
}

impl FieldOption {
    pub fn label(&self) -> String {
        if self.label.is_empty() {
            field_text(&self.value)
        } else {
            self.label.clone()
        }
    }
}

/// One setting of a trigger or a node type.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct FieldDef {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub options: Vec<FieldOption>,
    #[serde(default)]
    pub help: Option<String>,
    /// A `tag` field that takes several tags (a list of ids).
    #[serde(default)]
    pub multiple: bool,
    /// A `json` field holding a list of objects: one item's fields, for a
    /// list form instead of raw JSON.
    #[serde(default)]
    pub item_fields: Vec<FieldDef>,
}

impl FieldDef {
    pub fn label(&self) -> &str {
        if self.label.is_empty() {
            &self.key
        } else {
            &self.label
        }
    }

    /// Its help text, when it has some.
    pub fn help(&self) -> Option<&str> {
        self.help.as_deref().filter(|help| !help.trim().is_empty())
    }

    /// The choices of a select or severity field (severity falls back to
    /// the alarm severities).
    pub fn choices(&self) -> Vec<FieldOption> {
        if !self.options.is_empty() || self.kind != FieldKind::Severity {
            return self.options.clone();
        }
        ["info", "warning", "critical"]
            .iter()
            .map(|value| FieldOption {
                value: Value::String((*value).to_owned()),
                label: String::new(),
            })
            .collect()
    }

    /// Whether `value` leaves a required field unset.
    pub fn is_missing(&self, value: Option<&Value>) -> bool {
        self.required
            && match value {
                None | Some(Value::Null) => true,
                Some(Value::String(text)) => text.trim().is_empty(),
                Some(_) => false,
            }
    }
}

/// A trigger type of the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct TriggerType {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
}

impl TriggerType {
    /// A new trigger's settings: its type, then every field's default.
    pub fn defaults(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("type".into(), Value::String(self.kind.clone()));
        for field in &self.fields {
            if let Some(value) = &field.default {
                map.insert(field.key.clone(), value.clone());
            }
        }
        map
    }

    pub fn label(&self) -> &str {
        if self.label.is_empty() {
            &self.kind
        } else {
            &self.label
        }
    }
}

/// Where a node type sits in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Logic,
    Transform,
    Output,
    Timing,
    Advanced,
    /// A category this console does not know.
    Other,
}

impl Category {
    /// The palette's groups, in order.
    pub const ALL: [Self; 6] = [
        Self::Logic,
        Self::Transform,
        Self::Output,
        Self::Timing,
        Self::Advanced,
        Self::Other,
    ];

    pub fn parse(text: &str) -> Self {
        match text.to_ascii_lowercase().as_str() {
            "logic" => Self::Logic,
            "transform" => Self::Transform,
            "output" => Self::Output,
            "timing" => Self::Timing,
            "advanced" => Self::Advanced,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Logic => "Logic",
            Self::Transform => "Transform",
            Self::Output => "Output",
            Self::Timing => "Timing",
            Self::Advanced => "Advanced",
            Self::Other => "Other",
        }
    }

    /// The theme colour of the category's nodes.
    pub fn tone(self) -> Tone {
        match self {
            Self::Logic => Tone::Info,
            Self::Transform => Tone::Success,
            Self::Output => Tone::Warning,
            Self::Timing => Tone::Series(4),
            Self::Advanced => Tone::Series(2),
            Self::Other => Tone::Series(6),
        }
    }
}

/// A node type of the catalogue.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct NodeType {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    /// The output ports' names. A switch adds one port per rule before
    /// these (see [`FlowGraph::outputs`]).
    #[serde(default)]
    pub outputs: Vec<String>,
    /// It acts on the plant or the world (writes, alarms, HTTP).
    #[serde(default)]
    pub side_effect: bool,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    /// The list field whose items each add an output before `outputs`
    /// (`"rules"` for a switch).
    #[serde(default)]
    pub dynamic_outputs: Option<String>,
}

impl NodeType {
    /// The field whose items add outputs: the catalogue's word, or a
    /// switch's `rules`.
    pub fn dynamic_key(&self) -> Option<&str> {
        self.dynamic_outputs
            .as_deref()
            .or_else(|| (self.kind == "switch").then_some("rules"))
    }

    pub fn category(&self) -> Category {
        Category::parse(&self.category)
    }

    pub fn label(&self) -> &str {
        if self.label.is_empty() {
            &self.kind
        } else {
            &self.label
        }
    }
}

/// `GET /api/v1/flows/node-types`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct Catalogue {
    #[serde(default)]
    pub triggers: Vec<TriggerType>,
    #[serde(default)]
    pub nodes: Vec<NodeType>,
}

impl Catalogue {
    pub fn node(&self, kind: &str) -> Option<&NodeType> {
        self.nodes.iter().find(|node| node.kind == kind)
    }

    pub fn trigger(&self, kind: &str) -> Option<&TriggerType> {
        self.triggers.iter().find(|trigger| trigger.kind == kind)
    }

    /// The palette: the node types matching `filter` (label, type or
    /// description), grouped by category in [`Category::ALL`] order.
    pub fn palette(&self, filter: &str) -> Vec<(Category, Vec<&NodeType>)> {
        let filter = filter.trim().to_lowercase();
        let matches = |node: &NodeType| {
            filter.is_empty()
                || node.label().to_lowercase().contains(&filter)
                || node.kind.to_lowercase().contains(&filter)
                || node.description.to_lowercase().contains(&filter)
        };
        Category::ALL
            .iter()
            .filter_map(|category| {
                let nodes: Vec<&NodeType> = self
                    .nodes
                    .iter()
                    .filter(|node| node.category() == *category && matches(node))
                    .collect();
                (!nodes.is_empty()).then_some((*category, nodes))
            })
            .collect()
    }

    /// The triggers matching `filter`.
    pub fn trigger_palette(&self, filter: &str) -> Vec<&TriggerType> {
        let filter = filter.trim().to_lowercase();
        self.triggers
            .iter()
            .filter(|trigger| {
                filter.is_empty()
                    || trigger.label().to_lowercase().contains(&filter)
                    || trigger.kind.to_lowercase().contains(&filter)
            })
            .collect()
    }

    /// The catalogue of a server without `/node-types`: the steps of the
    /// legacy engine, each process step with one output, the output steps
    /// with none.
    pub fn builtin() -> Self {
        let triggers = TRIGGER_KINDS
            .iter()
            .map(|kind| TriggerType {
                kind: kind.id.to_owned(),
                label: kind.label.to_owned(),
                description: String::new(),
                fields: trigger_fields(kind.id)
                    .iter()
                    .map(|field| FieldDef {
                        key: field.key.to_owned(),
                        label: field.label.to_owned(),
                        kind: field.kind,
                        required: field.required,
                        default: (kind.defaults)().get(field.key).cloned(),
                        ..FieldDef::default()
                    })
                    .collect(),
            })
            .collect();
        let nodes = NODE_KINDS
            .iter()
            .map(|kind| {
                let required = legacy_required(kind.id);
                NodeType {
                    kind: kind.id.to_owned(),
                    label: kind.label.to_owned(),
                    category: kind.category.to_owned(),
                    description: kind.desc.to_owned(),
                    outputs: if kind.category == "output" {
                        Vec::new()
                    } else {
                        vec!["out".to_owned()]
                    },
                    side_effect: kind.category == "output",
                    fields: (kind.defaults)()
                        .into_iter()
                        .map(|(key, value)| FieldDef {
                            label: key.replace('_', " "),
                            kind: if key == "severity" {
                                FieldKind::Severity
                            } else {
                                value_kind(&value)
                            },
                            required: required.contains(&key.as_str()),
                            key,
                            default: Some(value),
                            ..FieldDef::default()
                        })
                        .collect(),
                    dynamic_outputs: None,
                }
            })
            .collect();
        Self { triggers, nodes }
    }
}

/// The field kind a legacy setting's value suggests.
pub fn value_kind(value: &Value) -> FieldKind {
    match value {
        Value::Bool(_) => FieldKind::Bool,
        Value::Number(_) => FieldKind::Number,
        Value::String(_) => FieldKind::String,
        Value::Null | Value::Array(_) | Value::Object(_) => FieldKind::Json,
    }
}

/// The settings the legacy engine needs per step type.
fn legacy_required(kind: &str) -> &'static [&'static str] {
    match kind {
        "filter" => &["condition"],
        "threshold" => &["field", "rules"],
        "lookup" => &["field", "output_field"],
        "expression" => &["output_field", "expr"],
        "data_source_out" => &["value_field", "source_key", "name", "unit", "decimals"],
        "http_out" => &["url", "timeout_ms", "retry_count"],
        "alarm_out" => &["severity", "source_key", "message"],
        "write_out" => &["data_source_id"],
        "script" => &["script_id"],
        _ => &[],
    }
}

// ── The legacy engine's step types (the built-in catalogue) ──────────────

/// The theme colour a node is drawn in, as a theme token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Accent,
    Warning,
    Success,
    Danger,
    /// `theme.series(n)`.
    Series(usize),
}

/// A step type of the legacy `flow_engine::node::NodeConfig`, with the web
/// client's label, description and default settings.
pub struct NodeKind {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    /// Its catalogue category.
    pub category: &'static str,
    /// The settings a new step starts with (`NODE_META[type].defaultConfig`).
    pub defaults: fn() -> Map<String, Value>,
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Every legacy step type, in the web client's `NODE_META` order.
pub static NODE_KINDS: &[NodeKind] = &[
    NodeKind {
        id: "filter",
        label: "Filter",
        desc: "Pass only if condition is true",
        category: "logic",
        defaults: || object(json!({ "condition": "payload.metrics.temp > 0" })),
    },
    NodeKind {
        id: "transform",
        label: "Transform",
        desc: "Map fields to new schema",
        category: "transform",
        defaults: || {
            object(json!({
                "map": { "status": "if(payload.metrics.temp > 80, 'CRITICAL', 'OK')" }
            }))
        },
    },
    NodeKind {
        id: "threshold",
        label: "Threshold",
        desc: "Check values against rules",
        category: "logic",
        defaults: || {
            object(json!({
                "field": "payload.metrics.temp",
                "rules": [{ "gt": 80, "set": { "alarm": "HIGH" } }]
            }))
        },
    },
    NodeKind {
        id: "lookup",
        label: "Lookup",
        desc: "Map values via lookup table",
        category: "transform",
        defaults: || {
            object(json!({
                "field": "payload.machine_code",
                "output_field": "payload.machine_name",
                "table": { "M1": "Cutting" },
                "default": "Unknown"
            }))
        },
    },
    NodeKind {
        id: "expression",
        label: "Expression",
        desc: "Compute new field from formula",
        category: "transform",
        defaults: || {
            object(json!({
                "output_field": "payload.power_kw",
                "expr": "payload.metrics.voltage * payload.metrics.current * 0.001"
            }))
        },
    },
    NodeKind {
        id: "script",
        label: "Script",
        desc: "Run a QuickJS-WASM script",
        category: "advanced",
        defaults: || object(json!({ "script_id": 0, "inputs": [] })),
    },
    NodeKind {
        id: "dsl",
        label: "Transform DSL",
        desc: "Inline transform DSL expression",
        category: "advanced",
        defaults: || object(json!({ "source": "set payload.status = \"OK\"" })),
    },
    NodeKind {
        id: "data_source_out",
        label: "Data Source",
        desc: "Write to virtual sensor",
        category: "output",
        defaults: || {
            object(json!({
                "value_field": "payload.power_kw",
                "source_key": "flow_power",
                "name": "Power",
                "unit": "kW",
                "decimals": 2
            }))
        },
    },
    NodeKind {
        id: "http_out",
        label: "HTTP Webhook",
        desc: "POST to external URL",
        category: "output",
        defaults: || {
            object(json!({
                "url": "https://example.com/webhook",
                "headers": {},
                "timeout_ms": 5000,
                "retry_count": 3
            }))
        },
    },
    NodeKind {
        id: "alarm_out",
        label: "Alarm",
        desc: "Trigger an alarm",
        category: "output",
        defaults: || {
            object(json!({
                "severity": "critical",
                "source_key": "flow_alert",
                "message": "Alert triggered"
            }))
        },
    },
    NodeKind {
        id: "write_out",
        label: "Write",
        desc: "Write a value to a real device/sensor",
        category: "output",
        defaults: || {
            object(json!({
                "data_source_id": 0,
                "value_expr": "payload.value",
                "policy": "immediate",
                "reason": "flow:write"
            }))
        },
    },
];

/// A trigger type of the legacy `flow_engine::trigger::TriggerConfig`.
pub struct TriggerKind {
    pub id: &'static str,
    pub label: &'static str,
    pub tone: Tone,
    /// `TRIGGER_DEFAULTS[type]`, its `type` first.
    pub defaults: fn() -> Map<String, Value>,
}

/// The legacy triggers, in the web client's order.
pub static TRIGGER_KINDS: &[TriggerKind] = &[
    TriggerKind {
        id: "modbus",
        label: "Modbus",
        tone: Tone::Info,
        defaults: || {
            object(json!({
                "type": "modbus",
                "device_id": "*",
                "metric": "",
                "debounce_ms": 1000,
                "on_change_only": false
            }))
        },
    },
    TriggerKind {
        id: "timer",
        label: "Timer",
        tone: Tone::Warning,
        defaults: || object(json!({ "type": "timer", "interval_ms": 5000 })),
    },
    TriggerKind {
        id: "mqtt",
        label: "MQTT",
        tone: Tone::Series(4),
        defaults: || {
            object(json!({
                "type": "mqtt",
                "topic": "sensors/+/temp",
                "debounce_ms": 1000
            }))
        },
    },
];

pub fn node_kind(id: &str) -> Option<&'static NodeKind> {
    NODE_KINDS.iter().find(|kind| kind.id == id)
}

pub fn trigger_kind(id: &str) -> Option<&'static TriggerKind> {
    TRIGGER_KINDS.iter().find(|kind| kind.id == id)
}

/// One field of a legacy trigger's settings.
pub struct TriggerField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub required: bool,
}

pub fn trigger_fields(kind: &str) -> &'static [TriggerField] {
    match kind {
        "modbus" => &[
            TriggerField {
                key: "device_id",
                label: "Device ID",
                kind: FieldKind::Device,
                required: true,
            },
            TriggerField {
                key: "metric",
                label: "Metric (optional)",
                kind: FieldKind::String,
                required: false,
            },
            TriggerField {
                key: "debounce_ms",
                label: "Debounce",
                kind: FieldKind::DurationMs,
                required: false,
            },
            TriggerField {
                key: "on_change_only",
                label: "On change only",
                kind: FieldKind::Bool,
                required: false,
            },
        ],
        "timer" => &[TriggerField {
            key: "interval_ms",
            label: "Interval",
            kind: FieldKind::DurationMs,
            required: true,
        }],
        "mqtt" => &[
            TriggerField {
                key: "topic",
                label: "Topic",
                kind: FieldKind::String,
                required: true,
            },
            TriggerField {
                key: "debounce_ms",
                label: "Debounce",
                kind: FieldKind::DurationMs,
                required: false,
            },
        ],
        _ => &[],
    }
}

// ── Node settings ────────────────────────────────────────────────────────

/// A node's `type`.
pub fn config_type(config: &Map<String, Value>) -> &str {
    config.get("type").and_then(Value::as_str).unwrap_or("")
}

/// A node's `id`.
pub fn config_id(config: &Map<String, Value>) -> &str {
    config.get("id").and_then(Value::as_str).unwrap_or("")
}

/// The keys of a node that are the graph's, not its type's settings.
pub const RESERVED: [&str; 5] = ["id", "type", "x", "y", "name"];

/// The line under a node's label on the canvas.
pub fn preview(config: &Map<String, Value>) -> String {
    for key in [
        "condition",
        "expression",
        "expr",
        "field",
        "output_field",
        "url",
        "message",
        "tag",
        "duration_ms",
    ] {
        match config.get(key) {
            Some(Value::String(text)) if !text.is_empty() => return text.clone(),
            Some(Value::Number(number)) => return format!("{key} {number}"),
            _ => {}
        }
    }
    for key in ["map", "table"] {
        if let Some(value) = config.get(key).filter(|value| !value.is_null()) {
            return value.to_string().chars().take(30).collect();
        }
    }
    String::new()
}

/// The text a field shows for `value`.
pub fn field_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => pretty(value),
    }
}

/// A number field's text as JSON: whole numbers stay integers (`5000`, not
/// `5000.0`).
pub fn parse_number(text: &str) -> Result<Value, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("a number is needed".into());
    }
    if let Ok(whole) = text.parse::<i64>() {
        return Ok(Value::from(whole));
    }
    if let Ok(whole) = text.parse::<u64>() {
        return Ok(Value::from(whole));
    }
    text.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .ok_or_else(|| format!("“{text}” is not a number"))
}

/// A JSON field's text as JSON.
pub fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|error| format!("not JSON: {error}"))
}

/// A field's text as the value its kind stores; an empty optional number
/// is removed (`None`).
pub fn parse_field(kind: FieldKind, text: &str) -> Result<Option<Value>, String> {
    if kind.is_number() {
        if text.trim().is_empty() {
            return Ok(None);
        }
        return parse_number(text).map(Some);
    }
    if kind.is_json() {
        if text.trim().is_empty() {
            return Ok(None);
        }
        return parse_json(text).map(Some);
    }
    Ok(Some(Value::String(text.to_owned())))
}

/// A tag field's text as ids: each comma-separated entry is an id or a tag
/// key `resolve` knows. One id, or a list for a `multiple` field; `None`
/// when empty.
pub fn parse_tags(
    text: &str,
    multiple: bool,
    resolve: impl Fn(&str) -> Option<i64>,
) -> Result<Option<Value>, String> {
    let ids = text
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            entry
                .parse::<i64>()
                .ok()
                .or_else(|| resolve(entry))
                .ok_or_else(|| format!("no tag “{entry}”"))
        })
        .collect::<Result<Vec<i64>, String>>()?;
    Ok(match (ids.as_slice(), multiple) {
        ([], _) => None,
        ([id], false) => Some(Value::from(*id)),
        (_, false) => return Err("one tag only".into()),
        (_, true) => Some(Value::from(ids)),
    })
}

/// A tag field's value as text: its tags' keys (`name`), else their ids.
pub fn tag_text(value: &Value, name: impl Fn(i64) -> Option<String>) -> String {
    let one = |value: &Value| match value.as_i64() {
        Some(id) => name(id).unwrap_or_else(|| id.to_string()),
        None => field_text(value),
    };
    match value {
        Value::Array(items) => items.iter().map(one).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        other => one(other),
    }
}

/// One item of a list field from its fields' texts: numbers as numbers,
/// JSON kinds as JSON, empty optional fields left out.
pub fn parse_item(fields: &[FieldDef], texts: &[String]) -> Result<Value, String> {
    let mut item = Map::new();
    for (field, text) in fields.iter().zip(texts) {
        match parse_field(field.kind, text) {
            Ok(Some(Value::String(text))) if text.is_empty() && !field.required => {}
            Ok(Some(value)) => {
                item.insert(field.key.clone(), value);
            }
            Ok(None) => {}
            Err(error) => return Err(format!("{}: {error}", field.label())),
        }
    }
    Ok(Value::Object(item))
}

/// A switch's rules: `[{name, expression}]`.
pub fn parse_rules(value: &Value) -> Result<Vec<(String, String)>, String> {
    let Some(rules) = value.as_array() else {
        return Err("the rules must be a list: [{\"name\": …, \"expression\": …}]".into());
    };
    rules
        .iter()
        .enumerate()
        .map(|(index, rule)| {
            let expression = rule.get("expression").and_then(Value::as_str);
            match (rule.get("name"), expression) {
                (None | Some(Value::Null | Value::String(_)), Some(expression)) => {
                    Ok((rule_name(rule, index), expression.to_owned()))
                }
                _ => Err(format!(
                    "rule {} needs an expression (text) and, if named, a text name",
                    index + 1
                )),
            }
        })
        .collect()
}

/// A rule's output name: its `name`, or `rule N` as the engine names an
/// unnamed one.
pub fn rule_name(rule: &Value, index: usize) -> String {
    rule.get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map_or_else(|| format!("rule {}", index + 1), str::to_owned)
}

// ── The graph ────────────────────────────────────────────────────────────

/// A wire's end: the trigger or a node by its position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum End {
    Trigger,
    Node(usize),
}

/// One wire: out of `from`'s output `port` into `to`'s input.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Edge {
    pub from: String,
    #[serde(default)]
    pub port: usize,
    pub to: String,
}

/// A save or a check the graph fails: which node, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub at: Option<End>,
    pub message: String,
}

impl Problem {
    fn new(at: Option<End>, message: impl Into<String>) -> Self {
        Self {
            at,
            message: message.into(),
        }
    }
}

/// Where Arrange puts its columns and rows.
pub const LAYOUT_COLUMN: f64 = 280.0;
pub const LAYOUT_ROW: f64 = 110.0;
/// Where a converted legacy chain goes: the graph server's row.
pub const LEGACY_X0: f64 = 120.0;
pub const LEGACY_DX: f64 = 280.0;
pub const LEGACY_Y: f64 = 80.0;

/// A flow's trigger and graph as JSON maps, in the server's key order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowGraph {
    pub trigger: Map<String, Value>,
    pub nodes: Vec<Map<String, Value>>,
    pub edges: Vec<Edge>,
    /// Top-level keys of the stored graph this console does not know,
    /// written back as they were.
    pub extra: Map<String, Value>,
}

impl FlowGraph {
    /// The stored texts: a trigger object, and a v2 graph or a legacy
    /// array (then `true`: converted here).
    pub fn parse(trigger_config: &str, nodes: &str) -> Result<(Self, bool), String> {
        let trigger = match serde_json::from_str::<Value>(trigger_config) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err("The trigger is not a JSON object".into()),
            Err(error) => return Err(format!("The trigger is not JSON: {error}")),
        };
        let objects = |items: Vec<Value>| {
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| match item {
                    Value::Object(map) => Ok(map),
                    _ => Err(format!("Node {} is not a JSON object", index + 1)),
                })
                .collect::<Result<Vec<_>, _>>()
        };
        match serde_json::from_str::<Value>(nodes) {
            Ok(Value::Array(items)) => Ok((Self::from_legacy(trigger, objects(items)?), true)),
            Ok(Value::Object(mut graph)) => {
                let nodes = match graph.shift_remove("nodes") {
                    Some(Value::Array(items)) => objects(items)?,
                    None | Some(Value::Null) => Vec::new(),
                    Some(_) => return Err("The graph's nodes are not a list".into()),
                };
                let edges = match graph.shift_remove("edges") {
                    Some(value @ Value::Array(_)) => serde_json::from_value(value)
                        .map_err(|error| format!("The graph's edges are not readable: {error}"))?,
                    None | Some(Value::Null) => Vec::new(),
                    Some(_) => return Err("The graph's edges are not a list".into()),
                };
                graph.shift_remove("version");
                Ok((
                    Self {
                        trigger,
                        nodes,
                        edges,
                        extra: graph,
                    },
                    false,
                ))
            }
            Ok(_) => Err("The nodes are neither a graph nor a list".into()),
            Err(error) => Err(format!("The nodes are not JSON: {error}")),
        }
    }

    pub fn of(flow: &Flow) -> Result<(Self, bool), String> {
        Self::parse(&flow.trigger_config, &flow.nodes)
    }

    /// The legacy engine's flow as a graph: the process steps chained from
    /// the trigger in order, every output step fed by the chain's end (as
    /// the legacy engine ran it). Steps without a position go left to
    /// right as the graph server lays a converted chain out (x 120, 400,
    /// 680, … at y 80), the outputs stacked in the column after the chain.
    pub fn from_legacy(trigger: Map<String, Value>, nodes: Vec<Map<String, Value>>) -> Self {
        let is_output = |node: &Map<String, Value>| {
            node_kind(config_type(node)).is_some_and(|kind| kind.category == "output")
        };
        let mut graph = Self {
            trigger,
            nodes,
            edges: Vec::new(),
            extra: Map::new(),
        };
        let mut previous = TRIGGER.to_owned();
        let mut x = LEGACY_X0;
        for node in graph.nodes.iter_mut().filter(|node| !is_output(node)) {
            let id = config_id(node).to_owned();
            graph.edges.push(Edge {
                from: previous,
                port: 0,
                to: id.clone(),
            });
            place_if_unplaced(node, x, LEGACY_Y);
            x += LEGACY_DX;
            previous = id;
        }
        let mut y = LEGACY_Y;
        for node in graph.nodes.iter_mut().filter(|node| is_output(node)) {
            graph.edges.push(Edge {
                from: previous.clone(),
                port: 0,
                to: config_id(node).to_owned(),
            });
            place_if_unplaced(node, x, y);
            y += LAYOUT_ROW;
        }
        graph
    }

    /// The trigger as written back.
    pub fn trigger_json(&self) -> String {
        pretty(&Value::Object(self.trigger.clone()))
    }

    /// The graph as v2 JSON text.
    pub fn nodes_json(&self) -> String {
        let mut graph = Map::new();
        graph.insert("version".into(), Value::from(2));
        graph.insert(
            "nodes".into(),
            Value::Array(self.nodes.iter().cloned().map(Value::Object).collect()),
        );
        graph.insert(
            "edges".into(),
            serde_json::to_value(&self.edges).unwrap_or(Value::Array(Vec::new())),
        );
        for (key, value) in &self.extra {
            graph.insert(key.clone(), value.clone());
        }
        pretty(&Value::Object(graph))
    }

    /// The graph as the legacy engine's array, when it is the shape that
    /// engine runs: process steps in one chain from the trigger, every
    /// output step fed by the chain's end. Positions and names ride along
    /// (the legacy engine ignores them).
    pub fn legacy_json(&self, catalogue: &Catalogue) -> Result<String, String> {
        let output = |index: usize| {
            catalogue
                .node(config_type(&self.nodes[index]))
                .is_some_and(|kind| kind.category() == Category::Output)
        };
        let mut chain = Vec::new();
        let mut at = End::Trigger;
        let mut seen = HashSet::new();
        loop {
            let next: Vec<usize> = self
                .outgoing(at)
                .into_iter()
                .filter_map(|edge| self.index_of(&self.edges[edge].to))
                .filter(|index| !output(*index))
                .collect();
            match next.as_slice() {
                [] => break,
                [index] if seen.insert(*index) => {
                    chain.push(*index);
                    at = End::Node(*index);
                }
                _ => return Err(STRAIGHT_LINE.into()),
            }
        }
        let outputs: Vec<usize> = (0..self.nodes.len())
            .filter(|index| output(*index))
            .collect();
        if chain.len() + outputs.len() != self.nodes.len() {
            return Err(STRAIGHT_LINE.into());
        }
        let end_id = self.id_of(at);
        for edge in &self.edges {
            let into_output = self.index_of(&edge.to).is_some_and(output);
            let from_chain = edge.from == TRIGGER
                || self
                    .index_of(&edge.from)
                    .is_some_and(|index| chain.contains(&index));
            if edge.port != 0 || !from_chain || (into_output && edge.from != end_id) {
                return Err(STRAIGHT_LINE.into());
            }
        }
        for index in &outputs {
            let id = config_id(&self.nodes[*index]);
            if !self
                .edges
                .iter()
                .any(|edge| edge.to == id && edge.from == end_id)
            {
                return Err(STRAIGHT_LINE.into());
            }
        }
        let array = chain
            .into_iter()
            .chain(outputs)
            .map(|index| Value::Object(self.nodes[index].clone()))
            .collect();
        Ok(pretty(&Value::Array(array)))
    }

    pub fn trigger_type(&self) -> &str {
        config_type(&self.trigger)
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|node| config_id(node) == id)
    }

    /// The id an edge names `end` by.
    pub fn id_of(&self, end: End) -> String {
        match end {
            End::Trigger => TRIGGER.to_owned(),
            End::Node(index) => self
                .nodes
                .get(index)
                .map(|node| config_id(node).to_owned())
                .unwrap_or_default(),
        }
    }

    /// The end an edge's `from` names.
    pub fn end_of(&self, id: &str) -> Option<End> {
        if id == TRIGGER {
            Some(End::Trigger)
        } else {
            self.index_of(id).map(End::Node)
        }
    }

    /// The output ports' names of `end`: the trigger has one; a switch
    /// one per rule, then the catalogue's (`otherwise`); a node of an
    /// unknown type keeps as many as its wires use.
    pub fn outputs(&self, end: End, catalogue: &Catalogue) -> Vec<String> {
        let index = match end {
            End::Trigger => return vec![String::new()],
            End::Node(index) => index,
        };
        let Some(node) = self.nodes.get(index) else {
            return Vec::new();
        };
        let kind = config_type(node);
        let Some(known) = catalogue.node(kind) else {
            let id = config_id(node);
            let used = self
                .edges
                .iter()
                .filter(|edge| edge.from == id)
                .map(|edge| edge.port + 1)
                .max()
                .unwrap_or(1);
            return (1..=used).map(|port| format!("{port}")).collect();
        };
        let mut names = Vec::new();
        if let Some(key) = known.dynamic_key()
            && let Some(items) = node.get(key).and_then(Value::as_array)
        {
            names.extend(
                items
                    .iter()
                    .enumerate()
                    .map(|(at, item)| rule_name(item, at)),
            );
        }
        names.extend(known.outputs.iter().cloned());
        names
    }

    /// A node's name, else its type's label, else its type.
    pub fn label(&self, end: End, catalogue: &Catalogue) -> String {
        match end {
            End::Trigger => {
                let kind = self.trigger_type();
                catalogue
                    .trigger(kind)
                    .map_or_else(|| kind.to_owned(), |known| known.label().to_owned())
            }
            End::Node(index) => {
                let Some(node) = self.nodes.get(index) else {
                    return String::new();
                };
                if let Some(name) = node.get("name").and_then(Value::as_str)
                    && !name.trim().is_empty()
                {
                    return name.to_owned();
                }
                type_label(config_type(node), catalogue)
            }
        }
    }

    /// Where node `index` sits.
    pub fn position(&self, index: usize) -> (f64, f64) {
        let Some(node) = self.nodes.get(index) else {
            return (0.0, 0.0);
        };
        let get = |key: &str| node.get(key).and_then(Value::as_f64).unwrap_or(0.0);
        (get("x"), get("y"))
    }

    /// Move node `index`, to whole units.
    pub fn set_position(&mut self, index: usize, x: f64, y: f64) {
        if let Some(node) = self.nodes.get_mut(index) {
            set_xy(node, x, y);
        }
    }

    /// The edges out of `end`, by position in `edges`.
    pub fn outgoing(&self, end: End) -> Vec<usize> {
        let id = self.id_of(end);
        (0..self.edges.len())
            .filter(|edge| self.edges[*edge].from == id)
            .collect()
    }

    /// The edges into node `index`.
    pub fn incoming(&self, index: usize) -> Vec<usize> {
        let id = self.id_of(End::Node(index));
        (0..self.edges.len())
            .filter(|edge| self.edges[*edge].to == id)
            .collect()
    }

    /// Add a node of `kind` with `id` at `(x, y)`, with its fields'
    /// defaults. Its position.
    pub fn add(&mut self, kind: &NodeType, id: String, x: f64, y: f64) -> usize {
        let mut node = Map::new();
        node.insert("id".into(), Value::String(id));
        node.insert("type".into(), Value::String(kind.kind.clone()));
        set_xy(&mut node, x, y);
        for field in &kind.fields {
            if matches!(field.key.as_str(), "id" | "type" | "x" | "y") {
                continue;
            }
            if let Some(value) = &field.default {
                node.insert(field.key.clone(), value.clone());
            }
        }
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// A copy of node `index` with `id`, 40 units down and right, unwired.
    pub fn duplicate(&mut self, index: usize, id: String) -> Option<usize> {
        let mut node = self.nodes.get(index)?.clone();
        let (x, y) = self.position(index);
        node.insert("id".into(), Value::String(id));
        set_xy(&mut node, x + 40.0, y + 40.0);
        self.nodes.push(node);
        Some(self.nodes.len() - 1)
    }

    /// Remove node `index` and its wires.
    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.nodes.len() {
            return false;
        }
        let id = config_id(&self.nodes.remove(index)).to_owned();
        self.edges.retain(|edge| edge.from != id && edge.to != id);
        true
    }

    /// Wire `from`'s output `port` into node `to`. `Ok(false)` when that
    /// wire is there already; `Err` with why it cannot be.
    pub fn connect(
        &mut self,
        from: End,
        port: usize,
        to: usize,
        catalogue: &Catalogue,
    ) -> Result<bool, String> {
        if to >= self.nodes.len() {
            return Err("There is no such node".into());
        }
        if let End::Node(source) = from {
            if source >= self.nodes.len() {
                return Err("There is no such node".into());
            }
            if source == to {
                return Err("A node cannot feed itself".into());
            }
        }
        let ports = self.outputs(from, catalogue).len();
        if port >= ports {
            return Err(format!(
                "{} has no output {}",
                self.label(from, catalogue),
                port + 1
            ));
        }
        let edge = Edge {
            from: self.id_of(from),
            port,
            to: self.id_of(End::Node(to)),
        };
        if self.edges.contains(&edge) {
            return Ok(false);
        }
        if let End::Node(source) = from
            && self.reaches(to, source)
        {
            return Err(format!(
                "{} already leads to {}: the wire would make a loop",
                self.label(End::Node(to), catalogue),
                self.label(from, catalogue)
            ));
        }
        self.edges.push(edge);
        Ok(true)
    }

    /// Whether a message from node `from` can reach node `to`.
    pub fn reaches(&self, from: usize, to: usize) -> bool {
        let mut queue = VecDeque::from([from]);
        let mut seen = HashSet::from([from]);
        while let Some(at) = queue.pop_front() {
            if at == to {
                return true;
            }
            for edge in self.outgoing(End::Node(at)) {
                if let Some(next) = self.index_of(&self.edges[edge].to)
                    && seen.insert(next)
                {
                    queue.push_back(next);
                }
            }
        }
        false
    }

    /// Drop the wires whose port the node no longer has (a switch rule
    /// removed). How many went.
    pub fn prune_ports(&mut self, catalogue: &Catalogue) -> usize {
        let before = self.edges.len();
        let ports: HashMap<String, usize> = std::iter::once(End::Trigger)
            .chain((0..self.nodes.len()).map(End::Node))
            .map(|end| (self.id_of(end), self.outputs(end, catalogue).len()))
            .collect();
        self.edges
            .retain(|edge| ports.get(&edge.from).is_none_or(|count| edge.port < *count));
        before - self.edges.len()
    }

    /// Keep a switch's wires on the same outputs when its rules change
    /// from `before` to `after` in number: rule `removed` went (its wires
    /// with it, later ports move up), or rules were added or dropped at
    /// the end (the fixed outputs after them move along).
    pub fn renumber_rules(
        &mut self,
        index: usize,
        before: usize,
        after: usize,
        removed: Option<usize>,
    ) {
        let id = self.id_of(End::Node(index));
        self.edges.retain_mut(|edge| {
            if edge.from != id {
                return true;
            }
            match removed {
                Some(rule) if edge.port == rule => return false,
                Some(rule) if edge.port > rule => edge.port -= 1,
                Some(_) => {}
                None if edge.port >= before => edge.port = edge.port - before + after,
                None if edge.port >= after => return false,
                None => {}
            }
            true
        });
    }

    /// Replace the trigger with `kind`'s defaults.
    pub fn set_trigger_kind(&mut self, kind: &TriggerType) {
        self.trigger = kind.defaults();
    }

    /// An id no node has, from `seed` (`n<ms>`).
    pub fn fresh_id(&self, seed: u64) -> String {
        let mut next = seed;
        loop {
            let id = format!("n{next}");
            if self.index_of(&id).is_none() {
                return id;
            }
            next += 1;
        }
    }

    /// The fields `end`'s type declares that are unset though required.
    pub fn missing(&self, end: End, catalogue: &Catalogue) -> Vec<String> {
        let (config, fields) = match end {
            End::Trigger => (
                &self.trigger,
                catalogue
                    .trigger(self.trigger_type())
                    .map(|kind| &kind.fields),
            ),
            End::Node(index) => match self.nodes.get(index) {
                Some(node) => (
                    node,
                    catalogue.node(config_type(node)).map(|kind| &kind.fields),
                ),
                None => return Vec::new(),
            },
        };
        fields
            .into_iter()
            .flatten()
            .filter(|field| field.is_missing(config.get(&field.key)))
            .map(|field| field.label().to_owned())
            .collect()
    }

    /// What the server would refuse, the first thing found: an unknown
    /// type, a required field unset, duplicate ids, a wire to nowhere.
    pub fn validate(&self, catalogue: &Catalogue) -> Result<(), Problem> {
        let kind = self.trigger_type();
        if catalogue.trigger(kind).is_none() {
            return Err(Problem::new(
                Some(End::Trigger),
                format!("The trigger has the unknown type “{kind}”"),
            ));
        }
        if let Some(field) = self.missing(End::Trigger, catalogue).first() {
            return Err(Problem::new(
                Some(End::Trigger),
                format!("Trigger: {field} is required"),
            ));
        }
        let mut ids = HashSet::new();
        for index in 0..self.nodes.len() {
            let end = End::Node(index);
            let id = config_id(&self.nodes[index]);
            let what = format!("{} ({id})", self.label(end, catalogue));
            if id.is_empty() || id == TRIGGER {
                return Err(Problem::new(Some(end), format!("{what}: needs an id")));
            }
            if !ids.insert(id) {
                return Err(Problem::new(
                    Some(end),
                    format!("{what}: another node has the id {id}"),
                ));
            }
            let kind = config_type(&self.nodes[index]);
            if catalogue.node(kind).is_none() {
                return Err(Problem::new(
                    Some(end),
                    format!("{what}: the server has no node type “{kind}”"),
                ));
            }
            if let Some(field) = self.missing(end, catalogue).first() {
                return Err(Problem::new(
                    Some(end),
                    format!("{what}: {field} is required"),
                ));
            }
            if kind == "switch"
                && let Some(Err(error)) = self.nodes[index].get("rules").map(parse_rules)
            {
                return Err(Problem::new(Some(end), format!("{what}: {error}")));
            }
        }
        for edge in &self.edges {
            let Some(from) = self.end_of(&edge.from) else {
                return Err(Problem::new(
                    None,
                    format!("A wire comes from {}, which is not in the flow", edge.from),
                ));
            };
            let Some(to) = self.index_of(&edge.to) else {
                return Err(Problem::new(
                    Some(from),
                    format!("A wire goes to {}, which is not in the flow", edge.to),
                ));
            };
            if edge.port >= self.outputs(from, catalogue).len() {
                return Err(Problem::new(
                    Some(from),
                    format!(
                        "{}: the wire into {} leaves by an output it does not have",
                        self.label(from, catalogue),
                        self.label(End::Node(to), catalogue)
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Lay the graph out in columns by distance from the trigger (the
    /// longest path), each column top to bottom in wiring order; nodes the
    /// trigger does not reach go in a last column. The trigger's place.
    pub fn arrange(&mut self) -> (f64, f64) {
        let count = self.nodes.len();
        let mut depth = vec![0usize; count];
        let mut reached = vec![false; count];
        // Longest-path layering on the acyclic graph (cycles are refused
        // when wiring; a stored one stops after `count` rounds).
        for _ in 0..=count {
            let mut changed = false;
            for edge in &self.edges {
                let Some(to) = self.index_of(&edge.to) else {
                    continue;
                };
                let next = match self.end_of(&edge.from) {
                    Some(End::Trigger) => 1,
                    Some(End::Node(from)) if reached[from] => depth[from] + 1,
                    _ => continue,
                };
                if !reached[to] || next > depth[to] {
                    depth[to] = next.min(count + 1);
                    reached[to] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let last = depth.iter().copied().max().unwrap_or(0) + 1;
        let mut rows: HashMap<usize, f64> = HashMap::new();
        for index in 0..count {
            let column = if reached[index] { depth[index] } else { last };
            let row = rows.entry(column).or_insert(0.0);
            let y = *row;
            *row += LAYOUT_ROW;
            #[allow(clippy::cast_precision_loss)]
            let x = column as f64 * LAYOUT_COLUMN;
            set_xy(&mut self.nodes[index], x, y);
        }
        (0.0, 0.0)
    }

    /// Where the trigger goes when nothing says: a column left of the
    /// leftmost node, level with the first node it feeds.
    pub fn trigger_spot(&self) -> (f64, f64) {
        if self.nodes.is_empty() {
            return (0.0, 0.0);
        }
        let left = (0..self.nodes.len())
            .map(|index| self.position(index).0)
            .fold(f64::INFINITY, f64::min);
        let y = self
            .outgoing(End::Trigger)
            .first()
            .and_then(|edge| self.index_of(&self.edges[*edge].to))
            .or(Some(0))
            .map_or(0.0, |index| self.position(index).1);
        (left - LAYOUT_COLUMN, y)
    }
}

const STRAIGHT_LINE: &str = "This server runs flows in a straight line only (one chain of steps, the outputs after it): update the server for branches";

fn type_label(kind: &str, catalogue: &Catalogue) -> String {
    match catalogue.node(kind) {
        Some(known) => known.label().to_owned(),
        None if kind.is_empty() => "Unknown".to_owned(),
        None => kind.to_owned(),
    }
}

fn set_xy(node: &mut Map<String, Value>, x: f64, y: f64) {
    #[allow(clippy::cast_possible_truncation)]
    let whole = |value: f64| Value::from(value.round() as i64);
    node.insert("x".into(), whole(x));
    node.insert("y".into(), whole(y));
}

fn place_if_unplaced(node: &mut Map<String, Value>, x: f64, y: f64) {
    let placed =
        node.get("x").is_some_and(Value::is_number) && node.get("y").is_some_and(Value::is_number);
    if !placed {
        set_xy(node, x, y);
    }
}

/// What a run went through, by position: the nodes it reached, the wires
/// it took, the nodes that failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Path {
    pub nodes: HashSet<usize>,
    pub edges: HashSet<usize>,
    pub errors: HashSet<usize>,
}

impl Path {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// The path a trace took through `graph`: a wire is taken when its
/// source left by its port (the trigger always fires) and its target ran.
pub fn taken_path<'a>(
    graph: &FlowGraph,
    steps: impl IntoIterator<Item = (&'a str, Option<usize>, bool)>,
) -> Path {
    let mut path = Path::default();
    let mut ports: HashMap<String, HashSet<usize>> = HashMap::new();
    for (node_id, port, failed) in steps {
        let Some(index) = graph.index_of(node_id) else {
            continue;
        };
        path.nodes.insert(index);
        if failed {
            path.errors.insert(index);
        }
        if let Some(port) = port {
            ports.entry(node_id.to_owned()).or_default().insert(port);
        }
    }
    for (at, edge) in graph.edges.iter().enumerate() {
        let ran = graph
            .index_of(&edge.to)
            .is_some_and(|to| path.nodes.contains(&to));
        let left = edge.from == TRIGGER
            || ports
                .get(&edge.from)
                .is_some_and(|ports| ports.contains(&edge.port));
        if ran && left {
            path.edges.insert(at);
        }
    }
    path
}

/// [`taken_path`] of a dry run.
pub fn test_path(graph: &FlowGraph, run: &TestRun) -> Path {
    taken_path(
        graph,
        run.trace
            .iter()
            .map(|step| (step.node_id.as_str(), step.port, step.error.is_some())),
    )
}

// ── Calls ────────────────────────────────────────────────────────────────

/// Whether `error` is the server saying it has no such route (an older
/// server): 404 or 405, or the 500 "Flow not found" an older server
/// answers when it reads `node-types` as a flow id.
pub fn missing_route(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ServerError>().is_some_and(|error| {
        matches!(
            error.status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
        ) || (error.status == reqwest::StatusCode::INTERNAL_SERVER_ERROR
            && error.message.to_ascii_lowercase().contains("not found"))
    })
}

/// A refusal of a flow: the server's message and the node it names.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Refusal {
    pub message: String,
    pub node_id: Option<String>,
}

/// `error` as a refusal when the server answered 400 about the flow.
pub fn refusal(error: &anyhow::Error) -> Option<Refusal> {
    let error = error.downcast_ref::<ServerError>()?;
    (error.status == reqwest::StatusCode::BAD_REQUEST).then(|| Refusal {
        message: error.message.clone(),
        node_id: error.node_id.clone(),
    })
}

/// `POST /api/v1/flows/{id}/webhook-token`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WebhookToken {
    pub token: String,
}

impl VoltrusApi {
    /// `GET /api/v1/flows`.
    pub async fn flows(&self) -> Result<Vec<Flow>> {
        Ok(self.get_json::<FlowList>(FLOWS).await?.flows)
    }

    /// `GET /api/v1/flows/{id}`.
    pub async fn flow(&self, id: &str) -> Result<Flow> {
        self.get_json(&flow_path(id)).await
    }

    /// `GET /api/v1/flows/node-types`; `None` from a server without the
    /// graph engine.
    pub async fn flow_node_types(&self) -> Result<Option<Catalogue>> {
        match self.get_json::<Catalogue>(NODE_TYPES).await {
            Ok(catalogue) => Ok(Some(catalogue)),
            Err(error) if missing_route(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `POST /api/v1/flows`; the id the server stored.
    pub async fn create_flow(&self, flow: &Flow) -> Result<String> {
        let created: FlowCreated = self.send_json(Method::POST, FLOWS, flow).await?;
        Ok(created.id)
    }

    /// `PUT /api/v1/flows/{id}`.
    pub async fn update_flow(&self, flow: &Flow) -> Result<()> {
        self.send_json::<_, IgnoredAny>(Method::PUT, &flow_path(&flow.id), flow)
            .await?;
        Ok(())
    }

    /// `DELETE /api/v1/flows/{id}`.
    pub async fn delete_flow(&self, id: &str) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::DELETE, &flow_path(id))
            .await?;
        Ok(())
    }

    /// `POST /api/v1/flows/{id}/start` (`running` true) or `/stop`.
    pub async fn run_flow(&self, id: &str, running: bool) -> Result<()> {
        let action = if running { "start" } else { "stop" };
        self.send_empty::<IgnoredAny>(Method::POST, &flow_action_path(id, action))
            .await?;
        Ok(())
    }

    /// `POST /api/v1/flows/{id}/run`: one real run now, on `input` if
    /// given; its trace.
    pub async fn run_flow_now(&self, id: &str, input: Option<Value>) -> Result<TestRun> {
        let body = match input {
            Some(input) => json!({ "input": input }),
            None => json!({}),
        };
        self.send_json(Method::POST, &flow_action_path(id, "run"), &body)
            .await
    }

    /// `POST /api/v1/flows/{id}/webhook-token` (admin): a new token for the
    /// flow's webhook, shown once.
    pub async fn flow_webhook_token(&self, id: &str) -> Result<String> {
        let answer: WebhookToken = self
            .send_empty(Method::POST, &flow_action_path(id, "webhook-token"))
            .await?;
        Ok(answer.token)
    }

    /// The legacy `POST /api/v1/flows/{id}/test`.
    pub async fn test_flow(&self, id: &str, payload: Value) -> Result<TestResult> {
        self.send_json(
            Method::POST,
            &flow_action_path(id, "test"),
            &test_body(payload),
        )
        .await
    }

    /// `POST /api/v1/flows/test`: a dry run of an unsaved flow; `None` from
    /// a server without it.
    pub async fn test_flow_draft(
        &self,
        trigger_config: &str,
        nodes: &str,
        input: Value,
    ) -> Result<Option<TestRun>> {
        let body = draft_test_body(trigger_config, nodes, input);
        match self
            .send_json::<_, TestRun>(Method::POST, DRAFT_TEST, &body)
            .await
        {
            Ok(run) => Ok(Some(run)),
            Err(error) if missing_route(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// `GET /api/v1/flows/{id}/runs`.
    pub async fn flow_runs(&self, id: &str) -> Result<Vec<FlowRun>> {
        Ok(self
            .get_json::<FlowRuns>(&flow_action_path(id, "runs"))
            .await?
            .runs)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `GET /api/v1/flows/node-types` as the graph server sends it.
    pub(crate) const CATALOGUE: &str = r#"{
        "triggers": [
            { "type": "timer", "label": "Timer", "description": "Every interval",
              "fields": [ { "key": "interval_ms", "label": "Interval", "kind": "duration_ms",
                            "required": true, "default": 5000 } ] },
            { "type": "tag_change", "label": "Tag change", "description": "A tag changed",
              "fields": [ { "key": "tag", "label": "Tag", "kind": "tag", "required": true } ] }
        ],
        "nodes": [
            { "type": "filter", "label": "Filter", "category": "logic",
              "description": "Pass or reject", "outputs": ["pass", "reject"], "side_effect": false,
              "fields": [ { "key": "condition", "label": "Condition", "kind": "expression",
                            "required": true, "default": "payload.value > 0" } ] },
            { "type": "switch", "label": "Switch", "category": "logic",
              "description": "Route by rules", "outputs": ["otherwise"], "side_effect": false,
              "fields": [ { "key": "rules", "label": "Rules", "kind": "json", "required": true,
                            "default": [] } ] },
            { "type": "delay", "label": "Delay", "category": "timing", "description": "Wait",
              "outputs": ["out"], "side_effect": false,
              "fields": [ { "key": "duration_ms", "label": "Duration", "kind": "duration_ms",
                            "required": true, "default": 1000 } ] },
            { "type": "alarm_out", "label": "Alarm", "category": "output",
              "description": "Raise an alarm", "outputs": [], "side_effect": true,
              "fields": [
                { "key": "severity", "label": "Severity", "kind": "severity", "required": true,
                  "default": "warning",
                  "options": [ {"value": "warning", "label": "Warning"}, {"value": "critical"} ] },
                { "key": "message", "label": "Message", "kind": "string", "required": true },
                { "key": "colour", "label": "Colour", "kind": "rgb", "required": false } ] }
        ]
    }"#;

    pub(crate) fn catalogue() -> Catalogue {
        serde_json::from_str(CATALOGUE).unwrap()
    }

    /// A v2 graph: trigger → switch; switch rule "hot" → alarm, otherwise
    /// → delay.
    pub(crate) const GRAPH: &str = r#"{
  "version": 2,
  "nodes": [
    {
      "id": "s",
      "type": "switch",
      "x": 260,
      "y": 0,
      "rules": [
        {
          "name": "hot",
          "expression": "payload.temp > 80"
        }
      ]
    },
    {
      "id": "a",
      "type": "alarm_out",
      "x": 520,
      "y": 0,
      "name": "Too hot",
      "severity": "critical",
      "message": "Hot"
    },
    {
      "id": "d",
      "type": "delay",
      "x": 520,
      "y": 110,
      "duration_ms": 1000
    }
  ],
  "edges": [
    {
      "from": "trigger",
      "port": 0,
      "to": "s"
    },
    {
      "from": "s",
      "port": 0,
      "to": "a"
    },
    {
      "from": "s",
      "port": 1,
      "to": "d"
    }
  ],
  "layout_hint": "kept"
}"#;

    pub(crate) const TIMER: &str = r#"{"type":"timer","interval_ms":5000}"#;

    pub(crate) fn graph() -> FlowGraph {
        FlowGraph::parse(TIMER, GRAPH).unwrap().0
    }

    fn ids(graph: &FlowGraph) -> Vec<&str> {
        graph.nodes.iter().map(config_id).collect()
    }

    #[test]
    fn a_v2_graph_parses_and_writes_back_as_stored() {
        let (graph, legacy) = FlowGraph::parse(TIMER, GRAPH).unwrap();
        assert!(!legacy);
        assert_eq!(ids(&graph), ["s", "a", "d"]);
        assert_eq!(graph.edges.len(), 3);
        assert_eq!(graph.extra["layout_hint"], "kept");
        assert_eq!(
            graph.nodes_json(),
            GRAPH,
            "byte for byte, unknown keys kept"
        );
        let flow = Flow {
            nodes: GRAPH.to_owned(),
            ..Flow::blank("f", "F", true, &catalogue().triggers[0], 1)
        };
        assert_eq!(flow.node_count(), Some(3));
        assert_eq!(
            FlowGraph::of(&flow).unwrap().0.trigger_json(),
            "{\n  \"type\": \"timer\",\n  \"interval_ms\": 5000\n}"
        );
        assert_eq!(graph.position(2), (520.0, 110.0));
        let (empty, _) = FlowGraph::parse(TIMER, r#"{"version": 2}"#).unwrap();
        assert!(empty.nodes.is_empty() && empty.edges.is_empty());
        assert!(FlowGraph::parse(TIMER, r#"{"nodes": 3}"#).is_err());
        assert!(FlowGraph::parse(TIMER, r#"{"edges": [{"to": "x"}]}"#).is_err());
        assert!(FlowGraph::parse("[]", "[]").is_err());
        assert!(
            FlowGraph::parse(TIMER, "[1]")
                .unwrap_err()
                .contains("Node 1")
        );
    }

    #[test]
    fn a_legacy_array_becomes_a_chain_with_outputs_off_its_end() {
        let nodes = r#"[
            {"id": "o", "type": "alarm_out", "severity": "warning", "source_key": "k", "message": "m"},
            {"id": "f", "type": "filter", "condition": "true"},
            {"id": "e", "type": "expression", "output_field": "p", "expr": "1"},
            {"id": "h", "type": "http_out", "url": "u", "timeout_ms": 1, "retry_count": 0}
        ]"#;
        let (graph, legacy) = FlowGraph::parse(TIMER, nodes).unwrap();
        assert!(legacy);
        let wires: Vec<(&str, usize, &str)> = graph
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.port, edge.to.as_str()))
            .collect();
        assert_eq!(
            wires,
            [
                ("trigger", 0, "f"),
                ("f", 0, "e"),
                ("e", 0, "o"),
                ("e", 0, "h")
            ]
        );
        // The chain left to right as the graph server lays it out, the
        // outputs stacked after it.
        assert_eq!(graph.position(1), (120.0, 80.0));
        assert_eq!(graph.position(2), (400.0, 80.0));
        assert_eq!(graph.position(0), (680.0, 80.0));
        assert_eq!(graph.position(3), (680.0, 190.0));
        assert_eq!(graph.trigger_spot(), (120.0 - LAYOUT_COLUMN, 80.0));
        let builtin = Catalogue::builtin();
        assert_eq!(graph.validate(&builtin), Ok(()));

        // Written back for a legacy server: the chain, then the outputs,
        // positions riding along.
        let text = graph.legacy_json(&builtin).unwrap();
        let back: Vec<Value> = serde_json::from_str(&text).unwrap();
        let order: Vec<&str> = back
            .iter()
            .map(|node| node["id"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["f", "e", "o", "h"]);
        assert_eq!(back[0]["x"], 120);
        let again = FlowGraph::parse(TIMER, &text).unwrap().0;
        assert_eq!(again.position(2), graph.position(0), "positions kept");

        // A branch is not a straight line.
        let mut branched = graph.clone();
        branched
            .connect(End::Trigger, 0, 2, &builtin)
            .expect("trigger → expression");
        assert!(
            branched
                .legacy_json(&builtin)
                .unwrap_err()
                .contains("straight line")
        );
        // Nor an output fed from the middle of the chain.
        let mut middle = graph.clone();
        middle.edges.retain(|edge| edge.to != "h");
        middle.connect(End::Node(1), 0, 3, &builtin).unwrap();
        assert!(middle.legacy_json(&builtin).is_err());
        let empty = FlowGraph::parse(TIMER, "[]").unwrap().0;
        assert_eq!(empty.legacy_json(&builtin).unwrap(), "[]");
    }

    #[test]
    fn the_catalogue_decodes_kinds_options_and_defaults() {
        let catalogue = catalogue();
        let alarm = catalogue.node("alarm_out").unwrap();
        assert_eq!(alarm.category(), Category::Output);
        assert!(alarm.side_effect);
        assert_eq!(alarm.fields[0].kind, FieldKind::Severity);
        assert_eq!(alarm.fields[0].choices()[1].label(), "critical");
        assert_eq!(alarm.fields[2].kind, FieldKind::Other);
        assert!(alarm.fields[2].kind.is_json(), "unknown kinds edit as JSON");
        assert!(alarm.fields[1].default.is_none());
        let tag = &catalogue.triggers[1].fields[0];
        assert_eq!((tag.kind, tag.required), (FieldKind::Tag, true));
        let trigger = catalogue.trigger("timer").unwrap();
        assert_eq!(
            Value::Object(trigger.defaults()),
            json!({ "type": "timer", "interval_ms": 5000 })
        );
        let bare = FieldDef {
            key: "s".into(),
            kind: FieldKind::Severity,
            ..FieldDef::default()
        };
        assert_eq!(bare.choices().len(), 3, "the alarm severities");
        let groups: Vec<(Category, usize)> = catalogue
            .palette("")
            .into_iter()
            .map(|(category, nodes)| (category, nodes.len()))
            .collect();
        assert_eq!(
            groups,
            [
                (Category::Logic, 2),
                (Category::Output, 1),
                (Category::Timing, 1)
            ]
        );
        let found = catalogue.palette("WAIT");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1[0].kind, "delay");
        assert_eq!(catalogue.trigger_palette("tag").len(), 1);
    }

    #[test]
    fn the_builtin_catalogue_is_the_legacy_engines() {
        let builtin = Catalogue::builtin();
        assert_eq!(builtin.triggers.len(), 3);
        assert_eq!(builtin.nodes.len(), NODE_KINDS.len());
        let filter = builtin.node("filter").unwrap();
        assert_eq!(filter.outputs, ["out"]);
        assert_eq!(filter.category(), Category::Logic);
        assert_eq!(filter.fields[0].kind, FieldKind::String);
        assert!(filter.fields[0].required);
        let http = builtin.node("http_out").unwrap();
        assert!(http.outputs.is_empty() && http.side_effect);
        let headers = http
            .fields
            .iter()
            .find(|field| field.key == "headers")
            .unwrap();
        assert_eq!(headers.kind, FieldKind::Json);
        let interval = &builtin.trigger("timer").unwrap().fields[0];
        assert_eq!(interval.kind, FieldKind::DurationMs);
        assert_eq!(interval.default, Some(json!(5000)));
        let mut graph = FlowGraph::default();
        graph.set_trigger_kind(builtin.trigger("modbus").unwrap());
        for kind in &builtin.nodes {
            let id = graph.fresh_id(1);
            graph.add(kind, id, 0.0, 0.0);
        }
        assert_eq!(graph.validate(&builtin), Ok(()));
        assert_eq!(graph.fresh_id(1), "n12");
        // The data source's own "name" setting is its sensor's name.
        let sensor = graph.index_of("n8").unwrap();
        assert_eq!(graph.nodes[sensor]["name"], "Power");
    }

    #[test]
    fn a_switch_has_a_port_per_rule_then_otherwise() {
        let catalogue = catalogue();
        let mut graph = graph();
        assert_eq!(
            graph.outputs(End::Node(0), &catalogue),
            ["hot", "otherwise"]
        );
        assert_eq!(graph.outputs(End::Trigger, &catalogue).len(), 1);
        assert!(graph.outputs(End::Node(1), &catalogue).is_empty());
        assert_eq!(graph.label(End::Node(1), &catalogue), "Too hot");
        assert_eq!(graph.label(End::Node(2), &catalogue), "Delay");
        graph.nodes[0].insert(
            "rules".into(),
            json!([{"name": "hot", "expression": "a"}, {"name": "cold", "expression": "b"}]),
        );
        assert_eq!(graph.outputs(End::Node(0), &catalogue).len(), 3);
        assert_eq!(graph.prune_ports(&catalogue), 0);
        // No rules: "otherwise" only; the wire from port 1 goes.
        graph.nodes[0].insert("rules".into(), json!([]));
        assert_eq!(graph.prune_ports(&catalogue), 1);
        assert_eq!(graph.edges.len(), 2);
        graph.nodes[0].insert("rules".into(), json!([{"name": "x"}]));
        let problem = graph.validate(&catalogue).unwrap_err();
        assert_eq!(problem.at, Some(End::Node(0)));
        assert!(problem.message.contains("rule 1"), "{}", problem.message);
        // Rules added or removed keep the wires on their outputs.
        let ports = |graph: &FlowGraph| -> Vec<(usize, String)> {
            graph
                .edges
                .iter()
                .filter(|edge| edge.from == "s")
                .map(|edge| (edge.port, edge.to.clone()))
                .collect()
        };
        let mut rules = self::graph();
        rules.renumber_rules(0, 1, 3, None);
        assert_eq!(
            ports(&rules),
            [(0, "a".into()), (3, "d".into())],
            "otherwise moved"
        );
        rules.renumber_rules(0, 3, 2, Some(0));
        assert_eq!(ports(&rules), [(2, "d".into())], "the hot wire went");
        rules.renumber_rules(0, 2, 0, None);
        assert_eq!(ports(&rules), [(0, "d".into())]);

        // An unknown type keeps the ports its wires use.
        let mut odd = self::graph();
        odd.nodes[0].insert("type".into(), json!("mystery"));
        assert_eq!(odd.outputs(End::Node(0), &catalogue), ["1", "2"]);
    }

    #[test]
    fn wiring_refuses_loops_self_wires_and_missing_ports() {
        let catalogue = catalogue();
        let mut graph = graph();
        assert_eq!(
            graph.connect(End::Node(0), 0, 1, &catalogue),
            Ok(false),
            "already"
        );
        assert!(
            graph
                .connect(End::Node(0), 0, 0, &catalogue)
                .unwrap_err()
                .contains("itself")
        );
        assert!(
            graph
                .connect(End::Node(0), 5, 2, &catalogue)
                .unwrap_err()
                .contains("output 6")
        );
        assert!(
            graph.connect(End::Node(1), 0, 2, &catalogue).is_err(),
            "an alarm has no outputs"
        );
        assert!(graph.connect(End::Trigger, 0, 9, &catalogue).is_err());
        assert!(
            graph
                .connect(End::Node(2), 0, 0, &catalogue)
                .unwrap_err()
                .contains("loop")
        );
        assert_eq!(graph.edges.len(), 3, "refused wires change nothing");
        // Fan-in: the trigger also feeds the delay.
        assert_eq!(graph.connect(End::Trigger, 0, 2, &catalogue), Ok(true));
        assert_eq!(graph.incoming(2).len(), 2);
        assert_eq!(graph.outgoing(End::Trigger).len(), 2);
        assert!(graph.remove(0));
        assert_eq!(ids(&graph), ["a", "d"]);
        let left: Vec<(&str, &str)> = graph
            .edges
            .iter()
            .map(|edge| (edge.from.as_str(), edge.to.as_str()))
            .collect();
        assert_eq!(left, [("trigger", "d")], "its wires went with it");
        assert!(!graph.remove(7));
        let copy = graph.duplicate(0, "a2".into()).unwrap();
        assert_eq!(graph.nodes[copy]["message"], "Hot");
        assert_eq!(graph.position(copy), (560.0, 40.0));
        assert!(graph.incoming(copy).is_empty());
    }

    #[test]
    fn validation_names_the_node() {
        let catalogue = catalogue();
        let mut graph = graph();
        assert_eq!(graph.validate(&catalogue), Ok(()));
        graph.nodes[1].insert("message".into(), json!("  "));
        let problem = graph.validate(&catalogue).unwrap_err();
        assert_eq!(problem.at, Some(End::Node(1)));
        assert_eq!(problem.message, "Too hot (a): Message is required");
        assert_eq!(graph.missing(End::Node(1), &catalogue), ["Message"]);
        graph.nodes[1].insert("message".into(), json!("Hot"));
        graph.nodes[2].insert("id".into(), json!("a"));
        assert!(
            graph
                .validate(&catalogue)
                .unwrap_err()
                .message
                .contains("another node")
        );
        graph.nodes[2].insert("id".into(), json!("d"));
        graph.edges.push(Edge {
            from: "ghost".into(),
            port: 0,
            to: "d".into(),
        });
        assert!(
            graph
                .validate(&catalogue)
                .unwrap_err()
                .message
                .contains("ghost")
        );
        graph.edges.pop();
        graph.trigger.insert("type".into(), json!("modbus"));
        assert_eq!(
            graph.validate(&catalogue).unwrap_err().at,
            Some(End::Trigger),
            "this server has no modbus trigger"
        );
        graph.set_trigger_kind(&catalogue.triggers[1]);
        assert_eq!(
            graph.validate(&catalogue).unwrap_err().message,
            "Trigger: Tag is required"
        );
    }

    #[test]
    fn a_trace_lights_the_path_it_took() {
        let graph = graph();
        let run: TestRun = serde_json::from_str(
            r#"{"ok": true, "outputs_reached": 1, "trace": [
                {"node_id": "s", "type": "switch", "input": {"temp": 90}, "output": {"temp": 90},
                 "port": 0, "error": null, "duration_us": 12, "side_effect_skipped": false},
                {"node_id": "a", "type": "alarm_out", "input": {"temp": 90}, "output": null,
                 "port": null, "error": null, "duration_us": 3, "side_effect_skipped": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(run.outputs_text(), "1");
        assert!(run.trace[1].side_effect_skipped);
        let path = test_path(&graph, &run);
        assert_eq!(path.nodes, HashSet::from([0, 1]));
        assert_eq!(
            path.edges,
            HashSet::from([0, 1]),
            "trigger → switch → alarm"
        );
        assert!(path.errors.is_empty());
        let other = taken_path(&graph, [("s", Some(1), false), ("d", None, true)]);
        assert_eq!(other.edges, HashSet::from([0, 2]), "the otherwise branch");
        assert_eq!(other.errors, HashSet::from([2]));
        assert!(taken_path(&graph, [("zz", Some(0), false)]).is_empty());
        let legacy: TestResult = serde_json::from_str(
            r#"{"success": true, "output": {}, "node_results": [
                {"node_id": "s", "result": "continue", "output": {}},
                {"node_id": "d", "result": "filter_reject"}]}"#,
        )
        .unwrap();
        let converted = TestRun::from_legacy(&legacy);
        assert_eq!(converted.trace[0].port, Some(0));
        assert_eq!(converted.trace[1].port, None);
        assert_eq!(test_path(&graph, &converted).edges, HashSet::from([0]));
    }

    #[test]
    fn runs_decode_with_or_without_a_trace() {
        let runs: FlowRuns = serde_json::from_str(
            r#"{"runs": [
                {"id": 9, "flow_id": "f", "started_at": 1761390000, "duration_ms": 3,
                 "success": true, "error": null, "output": "{\"power_kw\":1.2}",
                 "trace": [{"node_id": "s", "port": 1}, {"node_id": "d", "port": null, "error": "late"}]},
                {"id": 8, "flow_id": "f", "started_at": 1761389990, "duration_ms": 1,
                 "success": false, "error": "field not found", "output": null,
                 "trace": "[{\"node_id\": \"s\", \"port\": 0}]"},
                {"id": 7, "flow_id": "f", "started_at": 1761389980, "duration_ms": 1,
                 "success": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(runs.runs[0].path_text(), "s (port 1) → d ✗");
        assert_eq!(runs.runs[1].trace.len(), 1, "a trace sent as text");
        assert!(runs.runs[2].trace.is_empty());
    }

    #[test]
    fn bodies_inputs_and_fields() {
        assert_eq!(
            draft_test_body("{}", "[]", json!({"payload": 1})),
            json!({"flow": {"trigger_config": "{}", "nodes": "[]"}, "input": {"payload": 1}})
        );
        assert_eq!(
            parse_input(TEST_INPUT).unwrap(),
            json!({"payload": {"metrics": {"temp": 85}}, "metadata": {}})
        );
        assert_eq!(
            parse_input(r#"{"payload": 2}"#).unwrap(),
            json!({"payload": 2, "metadata": {}})
        );
        assert!(parse_input("[]").is_err());
        assert!(
            parse_input(r#"{"temp": 1}"#)
                .unwrap_err()
                .contains("payload")
        );
        assert_eq!(
            test_body(serde_json::from_str(TEST_PAYLOAD).unwrap()),
            json!({ "payload": { "metrics": { "temp": 85 } } })
        );
        assert_eq!(parse_number("5000").unwrap().to_string(), "5000");
        assert_eq!(parse_number(" 0.5 ").unwrap().to_string(), "0.5");
        assert!(parse_number("fast").is_err());
        assert_eq!(parse_field(FieldKind::DurationMs, " ").unwrap(), None);
        assert_eq!(
            parse_field(FieldKind::Expression, "a > 1").unwrap(),
            Some(json!("a > 1"))
        );
        assert!(parse_field(FieldKind::Json, "{a").is_err());
        assert_eq!(field_text(&json!({ "a": 1 })), "{\n  \"a\": 1\n}");
        assert_eq!(field_text(&Value::Null), "");
        assert_eq!(flow_path("flow 1"), "/api/v1/flows/flow%201");
        assert_eq!(flow_action_path("f", "run"), "/api/v1/flows/f/run");
        let mut config = Map::new();
        config.insert("duration_ms".into(), json!(250));
        assert_eq!(preview(&config), "duration_ms 250");
    }

    #[test]
    fn list_fields_and_tag_fields_as_the_graph_server_declares_them() {
        // As `GET /flows/node-types` sends them: help may be empty, a tag
        // field may take several tags, a list field declares its items.
        let rules: FieldDef = serde_json::from_value(json!({
            "key": "rules", "label": "Rules", "kind": "json", "required": true, "help": "",
            "default": [{"name": "high", "expression": "value > 50"}],
            "item_fields": [
                {"key": "name", "label": "Output name", "kind": "string", "required": true, "help": ""},
                {"key": "expression", "label": "Condition", "kind": "expression", "required": true, "help": ""}
            ]
        }))
        .unwrap();
        assert_eq!(rules.item_fields.len(), 2);
        assert_eq!(rules.help(), None);
        let tags: FieldDef = serde_json::from_value(json!({
            "key": "data_source_ids", "label": "Tags", "kind": "tag", "required": true,
            "help": "", "multiple": true
        }))
        .unwrap();
        assert!(tags.multiple);

        // Tags by key or id, one or several.
        let resolve = |key: &str| match key {
            "inv1.power" => Some(12),
            "inv2.power" => Some(13),
            _ => None,
        };
        assert_eq!(
            parse_tags("inv1.power, 7", true, resolve).unwrap(),
            Some(json!([12, 7]))
        );
        assert_eq!(parse_tags(" 13 ", false, resolve).unwrap(), Some(json!(13)));
        assert_eq!(parse_tags("", true, resolve).unwrap(), None);
        assert!(
            parse_tags("nope", true, resolve)
                .unwrap_err()
                .contains("nope")
        );
        assert!(parse_tags("1, 2", false, resolve).is_err(), "one tag only");
        let name = |id: i64| (id == 12).then(|| "inv1.power".to_owned());
        assert_eq!(tag_text(&json!([12, 7]), name), "inv1.power, 7");
        assert_eq!(tag_text(&json!(12), name), "inv1.power");

        // An item from its fields' texts.
        assert_eq!(
            parse_item(&rules.item_fields, &["hot".into(), "temp > 80".into()]).unwrap(),
            json!({"name": "hot", "expression": "temp > 80"})
        );
        let bounds: Vec<FieldDef> = serde_json::from_value(json!([
            {"key": "gt", "label": "Greater than", "kind": "number"},
            {"key": "lt", "label": "Less than", "kind": "number"},
            {"key": "set", "label": "Set fields", "kind": "json"}
        ]))
        .unwrap();
        assert_eq!(
            parse_item(
                &bounds,
                &["50".into(), String::new(), r#"{"level": "high"}"#.into()]
            )
            .unwrap(),
            json!({"gt": 50, "set": {"level": "high"}}),
            "empty optional fields are left out"
        );
        assert!(
            parse_item(&bounds, &["fast".into(), String::new(), String::new()])
                .unwrap_err()
                .starts_with("Greater than")
        );
        let run: TestRun = serde_json::from_value(json!({
            "ok": false, "error": "boom", "trace": [], "outputs_reached": ["n2"]
        }))
        .unwrap();
        assert_eq!(
            (run.error.as_deref(), run.outputs_text().as_str()),
            (Some("boom"), "1")
        );
        assert!(
            validate_meta("node-types", "x", Some(&[])).is_err(),
            "reserved"
        );
    }

    #[test]
    fn arranging_lays_columns_by_distance_from_the_trigger() {
        let mut graph = graph();
        graph.add(catalogue().node("delay").unwrap(), "lone".into(), 9.0, 9.0);
        // The trigger also feeds the alarm directly: it stays in the
        // switch's next column, the longest path.
        graph.edges.push(Edge {
            from: TRIGGER.into(),
            port: 0,
            to: "a".into(),
        });
        assert_eq!(graph.arrange(), (0.0, 0.0));
        assert_eq!(graph.position(0), (LAYOUT_COLUMN, 0.0));
        assert_eq!(graph.position(1), (2.0 * LAYOUT_COLUMN, 0.0));
        assert_eq!(graph.position(2), (2.0 * LAYOUT_COLUMN, LAYOUT_ROW));
        assert_eq!(
            graph.position(3),
            (3.0 * LAYOUT_COLUMN, 0.0),
            "a node nothing feeds goes last"
        );
    }

    #[test]
    fn the_meta_form_needs_an_id_and_a_name() {
        let flows = vec![Flow::blank(
            "flow_1",
            "x",
            true,
            &Catalogue::builtin().triggers[0],
            1,
        )];
        assert!(validate_meta("", "x", Some(&flows)).is_err());
        assert!(validate_meta("a", " ", None).is_err());
        assert!(validate_meta("flow_1", "x", Some(&flows)).is_err());
        assert!(validate_meta("a/b", "x", Some(&flows)).is_err());
        assert!(validate_meta("..", "x", Some(&flows)).is_err());
        assert!(validate_meta("flow_2", "x", Some(&flows)).is_ok());
        assert!(validate_meta("flow_1", "x", None).is_ok(), "editing");
        assert_eq!(flows[0].nodes, "[]");
        assert_eq!(flows[0].trigger_label(), "Modbus");
    }
}
