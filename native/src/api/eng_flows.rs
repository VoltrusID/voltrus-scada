//! Data Manager › Flows: the server's flow routes (`src/api/flows.rs`), the
//! stored flow (`models::Flow`, `models::FlowRun`) and the graph inside it
//! (`flow_engine::trigger::TriggerConfig`, `flow_engine::node::NodeConfig`),
//! mirrored, with the edits the native canvas editor makes to it.
//!
//! A stored flow carries its trigger and its steps as two JSON strings,
//! written by the web client as `JSON.stringify(value, null, 2)`. There are
//! no wires and no positions in it: the engine runs the process steps in
//! array order, then the output steps, each fed by the last process step
//! (or the trigger). The wires the editor draws are that order, and
//! connecting two steps reorders the array. [`FlowGraph`] keeps the JSON
//! maps as the server stored them (serde_json's map keeps key order here,
//! as GPUI turns on `preserve_order`), so an unedited flow saves back
//! byte for byte.

use anyhow::Result;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::ops::encode;
use super::{Method, VoltrusApi};

/// The list route.
pub const FLOWS: &str = "/api/v1/flows";

/// `GET`, `PUT`, `DELETE /api/v1/flows/{id}`.
pub fn flow_path(id: &str) -> String {
    format!("{FLOWS}/{}", encode(id))
}

/// `POST /api/v1/flows/{id}/start`, `/stop`, `/test`; `GET …/runs`.
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
    /// The steps as JSON text: `[{"id": "n1", "type": "filter", …}, …]`.
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

/// `models::FlowRun`: one run the engine logged.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
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
}

/// `GET /api/v1/flows/{id}/runs`: the latest 20, newest first.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct FlowRuns {
    #[serde(default)]
    pub runs: Vec<FlowRun>,
}

/// `POST /api/v1/flows/{id}/test`: the stored steps run once, in array
/// order, on the payload.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct TestResult {
    #[serde(default)]
    pub success: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default)]
    pub node_results: Vec<NodeResult>,
}

/// One step of a test run: `continue` with its output, `filter_reject`,
/// or an error (the run stops there).
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

/// The body of `POST /api/v1/flows/{id}/test`.
pub fn test_body(payload: Value) -> Value {
    json!({ "payload": payload })
}

/// The web client's default test payload.
pub const TEST_PAYLOAD: &str = r#"{"metrics": {"temp": 85}}"#;

/// `JSON.stringify(value, null, 2)`.
pub fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

impl Flow {
    /// A new flow as the web client makes it: the trigger's defaults, no
    /// steps, never created yet.
    pub fn blank(id: &str, name: &str, enabled: bool, trigger: &TriggerKind, now: i64) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            enabled,
            trigger_config: pretty(&Value::Object((trigger.defaults)())),
            nodes: pretty(&Value::Array(Vec::new())),
            created_at: 0,
            updated_at: now,
        }
    }

    /// The same flow renamed or enabled differently; its trigger and
    /// steps stay the text the server has.
    pub fn with_meta(&self, name: &str, enabled: bool, now: i64) -> Self {
        Self {
            name: name.to_owned(),
            enabled,
            updated_at: now,
            ..self.clone()
        }
    }

    /// The flow with `graph` as its trigger and steps (the editor's save).
    pub fn with_graph(&self, name: &str, enabled: bool, graph: &FlowGraph, now: i64) -> Self {
        Self {
            name: name.to_owned(),
            enabled,
            trigger_config: graph.trigger_json(),
            nodes: graph.nodes_json(),
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

    /// How many steps, if the text is an array.
    pub fn node_count(&self) -> Option<usize> {
        serde_json::from_str::<Value>(&self.nodes)
            .ok()?
            .as_array()
            .map(Vec::len)
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

// ── The steps the engine knows ───────────────────────────────────────────

/// Where a step sits in the web client's palette and in the engine's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// The trigger.
    Input,
    /// Runs in array order and feeds the next.
    Process,
    /// Runs after every process step, fed by the last.
    Output,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Process => "Process",
            Self::Output => "Output",
        }
    }
}

/// The theme colour a step is drawn in (the web client's per-type colour,
/// as a theme token).
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

/// A step type of `flow_engine::node::NodeConfig`, with the web client's
/// label, description and default settings.
pub struct NodeKind {
    pub id: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub category: Category,
    pub tone: Tone,
    /// The settings a new step starts with (`NODE_META[type].defaultConfig`).
    pub defaults: fn() -> Map<String, Value>,
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// Every step type, in the web client's `NODE_META` order.
pub static NODE_KINDS: &[NodeKind] = &[
    NodeKind {
        id: "filter",
        label: "Filter",
        desc: "Pass only if condition is true",
        category: Category::Process,
        tone: Tone::Info,
        defaults: || object(json!({ "condition": "payload.metrics.temp > 0" })),
    },
    NodeKind {
        id: "transform",
        label: "Transform",
        desc: "Map fields to new schema",
        category: Category::Process,
        tone: Tone::Accent,
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
        category: Category::Process,
        tone: Tone::Warning,
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
        category: Category::Process,
        tone: Tone::Series(4),
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
        category: Category::Process,
        tone: Tone::Success,
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
        category: Category::Process,
        tone: Tone::Series(2),
        defaults: || object(json!({ "script_id": 0, "inputs": [] })),
    },
    NodeKind {
        id: "data_source_out",
        label: "Data Source",
        desc: "Write to virtual sensor",
        category: Category::Output,
        tone: Tone::Series(1),
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
        category: Category::Output,
        tone: Tone::Series(5),
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
        category: Category::Output,
        tone: Tone::Danger,
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
        category: Category::Output,
        tone: Tone::Series(3),
        defaults: || {
            object(json!({
                "data_source_id": 0,
                "value_expr": "payload.value",
                "policy": "immediate",
                "reason": "flow:write"
            }))
        },
    },
    NodeKind {
        id: "dsl",
        label: "Transform DSL",
        desc: "Inline transform DSL expression",
        category: Category::Process,
        tone: Tone::Series(6),
        defaults: || object(json!({ "source": "set payload.status = \"OK\"" })),
    },
];

/// A trigger type of `flow_engine::trigger::TriggerConfig`.
pub struct TriggerKind {
    pub id: &'static str,
    pub label: &'static str,
    pub tone: Tone,
    /// `TRIGGER_DEFAULTS[type]`, its `type` first.
    pub defaults: fn() -> Map<String, Value>,
}

/// The triggers, in the web client's order.
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

/// The palette's step types of `category`, in the web client's order.
pub fn palette(category: Category) -> impl Iterator<Item = &'static NodeKind> {
    NODE_KINDS
        .iter()
        .filter(move |kind| kind.category == category)
}

/// A step's `type`.
pub fn config_type(config: &Map<String, Value>) -> &str {
    config.get("type").and_then(Value::as_str).unwrap_or("")
}

/// A step's `id`.
pub fn config_id(config: &Map<String, Value>) -> &str {
    config.get("id").and_then(Value::as_str).unwrap_or("")
}

/// Where a step runs; a type the console does not know runs as a process
/// step (the engine treats every non-output type so).
pub fn category_of(config: &Map<String, Value>) -> Category {
    node_kind(config_type(config)).map_or(Category::Process, |kind| kind.category)
}

/// A step's label: its type's, or the raw type.
pub fn label_of(config: &Map<String, Value>) -> String {
    match node_kind(config_type(config)) {
        Some(kind) => kind.label.to_owned(),
        None if config_type(config).is_empty() => "Unknown".to_owned(),
        None => config_type(config).to_owned(),
    }
}

/// The line under a step's label on the canvas, as the web client's
/// `FlowNode` picks it.
pub fn preview(config: &Map<String, Value>) -> String {
    for key in ["condition", "field", "output_field", "url", "message"] {
        if let Some(text) = config.get(key).and_then(Value::as_str)
            && !text.is_empty()
        {
            return text.to_owned();
        }
    }
    for key in ["map", "table"] {
        if let Some(value) = config.get(key).filter(|value| !value.is_null()) {
            return value.to_string().chars().take(30).collect();
        }
    }
    String::new()
}

// ── The graph ────────────────────────────────────────────────────────────

/// A wire's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Trigger,
    Node(usize),
}

/// One wire: the engine feeds step `to` from `from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wire {
    pub from: End,
    pub to: usize,
}

/// What a structural edit did to the steps: for each new position, the
/// position it had before (`None` for a step just added). The editor moves
/// its positions and its selection by it.
pub type Order = Vec<Option<usize>>;

/// A flow's trigger and steps as JSON maps, in the server's key order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlowGraph {
    pub trigger: Map<String, Value>,
    pub nodes: Vec<Map<String, Value>>,
}

impl FlowGraph {
    /// The stored texts; both must be JSON (an object, an array of
    /// objects), as the server needs them.
    pub fn parse(trigger_config: &str, nodes: &str) -> Result<Self, String> {
        let trigger = match serde_json::from_str::<Value>(trigger_config) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err("The trigger is not a JSON object".into()),
            Err(error) => return Err(format!("The trigger is not JSON: {error}")),
        };
        let nodes = match serde_json::from_str::<Value>(nodes) {
            Ok(Value::Array(items)) => items
                .into_iter()
                .enumerate()
                .map(|(index, item)| match item {
                    Value::Object(map) => Ok(map),
                    _ => Err(format!("Step {} is not a JSON object", index + 1)),
                })
                .collect::<Result<Vec<_>, _>>()?,
            Ok(_) => return Err("The steps are not a JSON array".into()),
            Err(error) => return Err(format!("The steps are not JSON: {error}")),
        };
        Ok(Self { trigger, nodes })
    }

    pub fn of(flow: &Flow) -> Result<Self, String> {
        Self::parse(&flow.trigger_config, &flow.nodes)
    }

    /// The trigger as the web client writes it.
    pub fn trigger_json(&self) -> String {
        pretty(&Value::Object(self.trigger.clone()))
    }

    /// The steps as the web client writes them.
    pub fn nodes_json(&self) -> String {
        pretty(&Value::Array(
            self.nodes.iter().cloned().map(Value::Object).collect(),
        ))
    }

    pub fn trigger_type(&self) -> &str {
        config_type(&self.trigger)
    }

    pub fn category(&self, index: usize) -> Category {
        self.nodes.get(index).map_or(Category::Process, category_of)
    }

    /// The step that feeds the outputs: the last process step, or the
    /// trigger.
    pub fn chain_end(&self) -> End {
        (0..self.nodes.len())
            .rev()
            .find(|index| self.category(*index) != Category::Output)
            .map_or(End::Trigger, End::Node)
    }

    /// The wires the engine follows: the process steps in array order from
    /// the trigger, then every output from the chain's end.
    pub fn wires(&self) -> Vec<Wire> {
        let mut wires = Vec::with_capacity(self.nodes.len());
        let mut previous = End::Trigger;
        for index in 0..self.nodes.len() {
            if self.category(index) != Category::Output {
                wires.push(Wire {
                    from: previous,
                    to: index,
                });
                previous = End::Node(index);
            }
        }
        for index in 0..self.nodes.len() {
            if self.category(index) == Category::Output {
                wires.push(Wire {
                    from: previous,
                    to: index,
                });
            }
        }
        wires
    }

    /// What feeds step `index`.
    pub fn source_of(&self, index: usize) -> Option<End> {
        self.wires()
            .into_iter()
            .find(|wire| wire.to == index)
            .map(|wire| wire.from)
    }

    fn slots(&self) -> Vec<(Option<usize>, Map<String, Value>)> {
        self.nodes
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, config)| (Some(index), config))
            .collect()
    }

    /// Store `slots` with the process steps first and the outputs after,
    /// each in their order: the order the engine runs (and `test` too).
    fn settle(&mut self, slots: Vec<(Option<usize>, Map<String, Value>)>) -> Order {
        let (process, outputs): (Vec<_>, Vec<_>) = slots
            .into_iter()
            .partition(|(_, config)| category_of(config) != Category::Output);
        let mut order = Vec::with_capacity(process.len() + outputs.len());
        self.nodes.clear();
        for (before, config) in process.into_iter().chain(outputs) {
            order.push(before);
            self.nodes.push(config);
        }
        order
    }

    /// Wire `from`'s output into step `to`'s input: `to` runs right after
    /// `from`. `Ok(None)` when it already does; `Err` with why the engine
    /// cannot run it so.
    pub fn connect(&mut self, from: End, to: usize) -> Result<Option<Order>, String> {
        if to >= self.nodes.len() {
            return Err("There is no such step".into());
        }
        if let End::Node(source) = from {
            if source >= self.nodes.len() {
                return Err("There is no such step".into());
            }
            if source == to {
                return Err("A step cannot feed itself".into());
            }
            if self.category(source) == Category::Output {
                return Err("Outputs end the flow: nothing runs after them".into());
            }
        }
        if self.category(to) == Category::Output {
            return if from == self.chain_end() {
                Ok(None)
            } else {
                Err(format!(
                    "Outputs take the result of the chain's last step ({}): wire the chain instead",
                    self.end_label(self.chain_end())
                ))
            };
        }
        if self.source_of(to) == Some(from) {
            return Ok(None);
        }
        let mut slots = self.slots();
        let moved = slots.remove(to);
        let at = match from {
            End::Trigger => 0,
            End::Node(source) => (if source > to { source - 1 } else { source }) + 1,
        };
        slots.insert(at, moved);
        Ok(Some(self.settle(slots)))
    }

    /// Add a step of `kind` with `id`: a process step after `after` (or at
    /// the chain's end), an output after the others. Its new position.
    pub fn add(&mut self, kind: &NodeKind, after: Option<End>, id: String) -> (usize, Order) {
        let mut config = Map::new();
        config.insert("id".into(), Value::String(id));
        config.insert("type".into(), Value::String(kind.id.into()));
        config.extend((kind.defaults)());
        let mut slots = self.slots();
        let at = match (kind.category, after) {
            (Category::Output, _) => slots.len(),
            (_, Some(End::Trigger)) => 0,
            (_, Some(End::Node(source)))
                if source < self.nodes.len() && self.category(source) != Category::Output =>
            {
                source + 1
            }
            _ => match self.chain_end() {
                End::Trigger => 0,
                End::Node(last) => last + 1,
            },
        };
        slots.insert(at, (None, config));
        let order = self.settle(slots);
        let index = order.iter().position(Option::is_none).unwrap_or(0);
        (index, order)
    }

    /// Remove step `index`.
    pub fn remove(&mut self, index: usize) -> Option<Order> {
        if index >= self.nodes.len() {
            return None;
        }
        let mut slots = self.slots();
        slots.remove(index);
        Some(self.settle(slots))
    }

    /// Run step `index` one place earlier (or later) among the steps of
    /// its category; `None` at the end of the line.
    pub fn shift(&mut self, index: usize, earlier: bool) -> Option<Order> {
        let category = (index < self.nodes.len()).then(|| self.category(index))?;
        let other = if earlier {
            (0..index).rev().find(|at| self.category(*at) == category)?
        } else {
            (index + 1..self.nodes.len()).find(|at| self.category(*at) == category)?
        };
        let mut slots = self.slots();
        slots.swap(index, other);
        Some(self.settle(slots))
    }

    /// Replace the trigger with `kind`'s defaults, as the web client does
    /// on a type change.
    pub fn set_trigger_kind(&mut self, kind: &TriggerKind) {
        self.trigger = (kind.defaults)();
    }

    /// An id no step has, from `seed` (the web client's `n<ms>`).
    pub fn fresh_id(&self, seed: u64) -> String {
        let mut next = seed;
        loop {
            let id = format!("n{next}");
            if !self.nodes.iter().any(|node| config_id(node) == id) {
                return id;
            }
            next += 1;
        }
    }

    /// A wire end's label: `Trigger · Modbus`, `Filter`.
    pub fn end_label(&self, end: End) -> String {
        match end {
            End::Trigger => format!(
                "Trigger · {}",
                trigger_kind(self.trigger_type()).map_or(self.trigger_type(), |kind| kind.label)
            ),
            End::Node(index) => self.nodes.get(index).map_or_else(String::new, label_of),
        }
    }

    /// What the server's `TriggerConfig` and `NodeConfig` would refuse:
    /// the server stores a flow before it reads it, so a flow it cannot
    /// read is saved and answered with an error.
    pub fn validate(&self) -> Result<(), String> {
        let kind = self.trigger_type();
        let Some(shape) = trigger_shape(kind) else {
            return Err(format!("The trigger has the unknown type “{kind}”"));
        };
        check(&self.trigger, shape, "Trigger")?;
        for (index, node) in self.nodes.iter().enumerate() {
            let what = format!("Step {} ({})", index + 1, label_of(node));
            if !node.get("id").is_some_and(Value::is_string) {
                return Err(format!("{what}: id must be text"));
            }
            let kind = config_type(node);
            let Some(shape) = node_shape(kind) else {
                return Err(format!("{what}: the server has no step type “{kind}”"));
            };
            check(node, shape, &what)?;
        }
        Ok(())
    }
}

// ── What the server reads ────────────────────────────────────────────────

/// A JSON type a serde field accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ty {
    Text,
    /// `Option<String>`: text or null.
    OptText,
    Int,
    UInt,
    /// `Option<u64>`.
    OptUInt,
    /// `Option<bool>`.
    OptBool,
    /// `HashMap<String, String>`.
    TextMap,
    /// `HashMap<String, Value>`.
    Object,
    /// `Vec<String>`.
    TextList,
    /// `Vec<ThresholdRule>`.
    Rules,
    Any,
}

/// (key, type, required).
type Shape = &'static [(&'static str, Ty, bool)];

fn node_shape(kind: &str) -> Option<Shape> {
    Some(match kind {
        "filter" => &[("condition", Ty::Text, true)],
        "transform" => &[("map", Ty::TextMap, false)],
        "threshold" => &[("field", Ty::Text, true), ("rules", Ty::Rules, true)],
        "lookup" => &[
            ("field", Ty::Text, true),
            ("output_field", Ty::Text, true),
            ("table", Ty::Object, false),
            ("default", Ty::Any, true),
        ],
        "expression" => &[("output_field", Ty::Text, true), ("expr", Ty::Text, true)],
        "data_source_out" => &[
            ("value_field", Ty::Text, true),
            ("source_key", Ty::Text, true),
            ("name", Ty::Text, true),
            ("unit", Ty::Text, true),
            ("decimals", Ty::Int, true),
        ],
        "http_out" => &[
            ("url", Ty::Text, true),
            ("headers", Ty::TextMap, false),
            ("timeout_ms", Ty::UInt, true),
            ("retry_count", Ty::UInt, true),
        ],
        "alarm_out" => &[
            ("severity", Ty::Text, true),
            ("source_key", Ty::Text, true),
            ("message", Ty::Text, true),
        ],
        "write_out" => &[
            ("data_source_id", Ty::Int, true),
            ("value_expr", Ty::OptText, false),
            ("policy", Ty::Text, false),
            ("reason", Ty::Text, false),
        ],
        "script" => &[
            ("script_id", Ty::Int, true),
            ("inputs", Ty::TextList, false),
        ],
        "dsl" => &[("source", Ty::Text, false)],
        _ => return None,
    })
}

fn trigger_shape(kind: &str) -> Option<Shape> {
    Some(match kind {
        "modbus" => &[
            ("device_id", Ty::Text, true),
            ("metric", Ty::OptText, false),
            ("debounce_ms", Ty::OptUInt, false),
            ("on_change_only", Ty::OptBool, false),
        ],
        "timer" => &[("interval_ms", Ty::UInt, true)],
        "mqtt" => &[
            ("topic", Ty::Text, true),
            ("debounce_ms", Ty::OptUInt, false),
        ],
        _ => return None,
    })
}

fn fits(value: &Value, ty: Ty) -> bool {
    let text_map = |value: &Value| {
        value
            .as_object()
            .is_some_and(|map| map.values().all(Value::is_string))
    };
    match ty {
        Ty::Text => value.is_string(),
        Ty::OptText => value.is_string() || value.is_null(),
        Ty::Int => value.is_i64(),
        Ty::UInt => value.is_u64(),
        Ty::OptUInt => value.is_u64() || value.is_null(),
        Ty::OptBool => value.is_boolean() || value.is_null(),
        Ty::TextMap => text_map(value),
        Ty::Object => value.is_object(),
        Ty::TextList => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        Ty::Rules => value.as_array().is_some_and(|rules| {
            rules.iter().all(|rule| {
                rule.as_object().is_some_and(|rule| {
                    ["gt", "gte", "lt", "lte", "eq"].iter().all(|key| {
                        rule.get(*key)
                            .is_none_or(|bound| bound.is_number() || bound.is_null())
                    }) && rule.get("set").is_none_or(Value::is_object)
                })
            })
        }),
        Ty::Any => true,
    }
}

fn expected(ty: Ty) -> &'static str {
    match ty {
        Ty::Text => "text",
        Ty::OptText => "text or null",
        Ty::Int => "a whole number",
        Ty::UInt | Ty::OptUInt => "a whole number of 0 or more",
        Ty::OptBool => "true or false",
        Ty::TextMap => "an object of text values",
        Ty::Object => "an object",
        Ty::TextList => "a list of text",
        Ty::Rules => "a list of rules ({\"gt\": 80, \"set\": {…}})",
        Ty::Any => "a value",
    }
}

fn check(config: &Map<String, Value>, shape: Shape, what: &str) -> Result<(), String> {
    for (key, ty, required) in shape {
        match config.get(*key) {
            None if *required => return Err(format!("{what}: {key} is missing")),
            None => {}
            Some(value) if !fits(value, *ty) => {
                return Err(format!("{what}: {key} must be {}", expected(*ty)));
            }
            Some(_) => {}
        }
    }
    Ok(())
}

// ── Settings fields ──────────────────────────────────────────────────────

/// How a setting is edited (the web client's `ConfigPanel` picks it by the
/// value's JSON type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Number,
    Bool,
    /// Objects, arrays and null, as JSON text.
    Json,
}

pub fn field_kind(value: &Value) -> FieldKind {
    match value {
        Value::Bool(_) => FieldKind::Bool,
        Value::Number(_) => FieldKind::Number,
        Value::String(_) => FieldKind::Text,
        Value::Null | Value::Array(_) | Value::Object(_) => FieldKind::Json,
    }
}

/// A step's settings in their stored order: every key but `id` and `type`.
pub fn node_fields(config: &Map<String, Value>) -> Vec<(String, FieldKind)> {
    config
        .iter()
        .filter(|(key, _)| key.as_str() != "id" && key.as_str() != "type")
        .map(|(key, value)| (key.clone(), field_kind(value)))
        .collect()
}

/// The text a field shows for `value`.
pub fn field_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => pretty(value),
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

/// One field of the trigger's settings, as the web client's `ConfigPanel`
/// shows them per trigger type.
pub struct TriggerField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
}

pub fn trigger_fields(kind: &str) -> &'static [TriggerField] {
    match kind {
        "modbus" => &[
            TriggerField {
                key: "device_id",
                label: "Device ID",
                kind: FieldKind::Text,
            },
            TriggerField {
                key: "metric",
                label: "Metric (optional)",
                kind: FieldKind::Text,
            },
            TriggerField {
                key: "debounce_ms",
                label: "Debounce (ms)",
                kind: FieldKind::Number,
            },
            TriggerField {
                key: "on_change_only",
                label: "On change only",
                kind: FieldKind::Bool,
            },
        ],
        "timer" => &[TriggerField {
            key: "interval_ms",
            label: "Interval (ms)",
            kind: FieldKind::Number,
        }],
        "mqtt" => &[
            TriggerField {
                key: "topic",
                label: "Topic",
                kind: FieldKind::Text,
            },
            TriggerField {
                key: "debounce_ms",
                label: "Debounce (ms)",
                kind: FieldKind::Number,
            },
        ],
        _ => &[],
    }
}

/// The value a trigger field shows: the stored one, else the web client's
/// fallback (`debounce_ms || 1000`, `interval_ms || 5000`).
pub fn trigger_value(trigger: &Map<String, Value>, key: &str) -> Value {
    match (trigger.get(key), key) {
        (Some(value), _) if !value.is_null() => value.clone(),
        (_, "debounce_ms") => Value::from(1000),
        (_, "interval_ms") => Value::from(5000),
        (_, "on_change_only") => Value::Bool(false),
        _ => Value::String(String::new()),
    }
}

// ── Calls ────────────────────────────────────────────────────────────────

impl VoltrusApi {
    /// `GET /api/v1/flows`.
    pub async fn flows(&self) -> Result<Vec<Flow>> {
        Ok(self.get_json::<FlowList>(FLOWS).await?.flows)
    }

    /// `GET /api/v1/flows/{id}`.
    pub async fn flow(&self, id: &str) -> Result<Flow> {
        self.get_json(&flow_path(id)).await
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

    /// `POST /api/v1/flows/{id}/test`.
    pub async fn test_flow(&self, id: &str, payload: Value) -> Result<TestResult> {
        self.send_json(
            Method::POST,
            &flow_action_path(id, "test"),
            &test_body(payload),
        )
        .await
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
mod tests {
    use super::*;

    /// A flow as the web client stores it and `GET /api/v1/flows/{id}`
    /// sends it back: `JSON.stringify(…, null, 2)` texts, keys in the
    /// order the web client wrote them.
    const STORED: &str = r#"{
        "id": "flow_1761390000000",
        "name": "Boiler power",
        "enabled": true,
        "trigger_config": "{\n  \"type\": \"modbus\",\n  \"device_id\": \"*\",\n  \"metric\": \"\",\n  \"debounce_ms\": 1000,\n  \"on_change_only\": false\n}",
        "nodes": "[\n  {\n    \"id\": \"n1761390001000\",\n    \"type\": \"filter\",\n    \"condition\": \"payload.metrics.temp > 0\"\n  },\n  {\n    \"id\": \"n1761390002000\",\n    \"type\": \"expression\",\n    \"output_field\": \"payload.power_kw\",\n    \"expr\": \"payload.metrics.voltage * payload.metrics.current * 0.001\"\n  },\n  {\n    \"id\": \"n1761390003000\",\n    \"type\": \"data_source_out\",\n    \"value_field\": \"payload.power_kw\",\n    \"source_key\": \"flow_power\",\n    \"name\": \"Power\",\n    \"unit\": \"kW\",\n    \"decimals\": 2\n  }\n]",
        "created_at": 1761390000,
        "updated_at": 1761390500
    }"#;

    fn stored() -> Flow {
        serde_json::from_str(STORED).unwrap()
    }

    fn ids(graph: &FlowGraph) -> Vec<&str> {
        graph.nodes.iter().map(config_id).collect()
    }

    #[test]
    fn the_list_and_one_flow_decode_in_the_servers_shape() {
        let list: FlowList = serde_json::from_str(&format!(r#"{{"flows": [{STORED}]}}"#)).unwrap();
        let flow = &list.flows[0];
        assert_eq!(flow, &stored());
        assert_eq!(flow.trigger_type().as_deref(), Some("modbus"));
        assert_eq!(flow.trigger_label(), "Modbus");
        assert_eq!(flow.node_count(), Some(3));
        let created: FlowCreated = serde_json::from_str(r#"{"id": "flow_1"}"#).unwrap();
        assert_eq!(created.id, "flow_1");
    }

    #[test]
    fn an_unedited_flow_saves_back_byte_for_byte() {
        let flow = stored();
        let graph = FlowGraph::of(&flow).unwrap();
        assert_eq!(graph.trigger_json(), flow.trigger_config);
        assert_eq!(graph.nodes_json(), flow.nodes);
        let saved = flow.with_graph("Boiler power", true, &graph, 1761390600);
        assert_eq!(saved.trigger_config, flow.trigger_config);
        assert_eq!(saved.nodes, flow.nodes);
        assert_eq!(saved.created_at, 1761390000);
        assert_eq!(saved.updated_at, 1761390600);
        // The body has every field the server's `Flow` needs.
        let body = serde_json::to_value(&saved).unwrap();
        let keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "id",
                "name",
                "enabled",
                "trigger_config",
                "nodes",
                "created_at",
                "updated_at"
            ]
        );
        assert!(graph.validate().is_ok());
    }

    #[test]
    fn adding_a_step_and_a_wire_writes_the_web_clients_json() {
        let flow = stored();
        let mut graph = FlowGraph::of(&flow).unwrap();
        // A threshold after the filter: process steps go before outputs.
        let kind = node_kind("threshold").unwrap();
        let (index, order) = graph.add(kind, Some(End::Node(0)), "n1761390004000".into());
        assert_eq!(index, 1);
        assert_eq!(order, [Some(0), None, Some(1), Some(2)]);
        assert_eq!(
            ids(&graph),
            [
                "n1761390001000",
                "n1761390004000",
                "n1761390002000",
                "n1761390003000"
            ]
        );
        // Wire the expression straight from the trigger: it runs first.
        let order = graph.connect(End::Trigger, 2).unwrap().unwrap();
        assert_eq!(order, [Some(2), Some(0), Some(1), Some(3)]);
        assert_eq!(
            graph.nodes_json(),
            "[\n  {\n    \"id\": \"n1761390002000\",\n    \"type\": \"expression\",\n    \"output_field\": \"payload.power_kw\",\n    \"expr\": \"payload.metrics.voltage * payload.metrics.current * 0.001\"\n  },\n  {\n    \"id\": \"n1761390001000\",\n    \"type\": \"filter\",\n    \"condition\": \"payload.metrics.temp > 0\"\n  },\n  {\n    \"id\": \"n1761390004000\",\n    \"type\": \"threshold\",\n    \"field\": \"payload.metrics.temp\",\n    \"rules\": [\n      {\n        \"gt\": 80,\n        \"set\": {\n          \"alarm\": \"HIGH\"\n        }\n      }\n    ]\n  },\n  {\n    \"id\": \"n1761390003000\",\n    \"type\": \"data_source_out\",\n    \"value_field\": \"payload.power_kw\",\n    \"source_key\": \"flow_power\",\n    \"name\": \"Power\",\n    \"unit\": \"kW\",\n    \"decimals\": 2\n  }\n]"
        );
        assert!(graph.validate().is_ok());
    }

    #[test]
    fn wires_follow_the_engines_order() {
        let graph = FlowGraph::of(&stored()).unwrap();
        assert_eq!(
            graph.wires(),
            [
                Wire {
                    from: End::Trigger,
                    to: 0
                },
                Wire {
                    from: End::Node(0),
                    to: 1
                },
                Wire {
                    from: End::Node(1),
                    to: 2
                },
            ]
        );
        assert_eq!(graph.chain_end(), End::Node(1));
        assert_eq!(graph.source_of(2), Some(End::Node(1)));
        // No process steps: every output hangs off the trigger.
        let mut outputs = FlowGraph::parse(r#"{"type":"timer","interval_ms":5000}"#, "[]").unwrap();
        outputs.add(node_kind("alarm_out").unwrap(), None, "a".into());
        outputs.add(node_kind("http_out").unwrap(), None, "b".into());
        assert_eq!(
            outputs.wires(),
            [
                Wire {
                    from: End::Trigger,
                    to: 0
                },
                Wire {
                    from: End::Trigger,
                    to: 1
                },
            ]
        );
    }

    #[test]
    fn connecting_reorders_or_says_why_not() {
        let mut graph = FlowGraph::of(&stored()).unwrap();
        let before = graph.clone();
        assert_eq!(graph.connect(End::Node(0), 1), Ok(None), "already so");
        assert_eq!(
            graph.connect(End::Node(1), 2),
            Ok(None),
            "the chain's end feeds outputs"
        );
        assert!(
            graph
                .connect(End::Node(0), 2)
                .unwrap_err()
                .contains("Expression")
        );
        assert!(
            graph
                .connect(End::Node(2), 0)
                .unwrap_err()
                .contains("Outputs end")
        );
        assert!(graph.connect(End::Node(1), 1).is_err());
        assert!(graph.connect(End::Trigger, 9).is_err());
        assert_eq!(graph, before, "refused wires change nothing");
        // The filter after the expression: the expression runs first.
        let order = graph.connect(End::Node(1), 0).unwrap().unwrap();
        assert_eq!(order, [Some(1), Some(0), Some(2)]);
        assert_eq!(graph.nodes[0]["type"], "expression");
    }

    #[test]
    fn structural_edits_keep_outputs_last_and_report_the_order() {
        let mut graph = FlowGraph::parse(
            r#"{"type":"timer","interval_ms":1000}"#,
            r#"[{"id":"o","type":"alarm_out","severity":"warning","source_key":"k","message":"m"},{"id":"p","type":"filter","condition":"true"}]"#,
        )
        .unwrap();
        // Stored with an output first: untouched, it stays so.
        assert_eq!(ids(&graph), ["o", "p"]);
        let order = graph.shift(1, true);
        assert_eq!(order, None, "the filter is the only process step");
        let (index, order) = graph.add(node_kind("expression").unwrap(), None, "e".into());
        assert_eq!(ids(&graph), ["p", "e", "o"]);
        assert_eq!(index, 1);
        assert_eq!(order, [Some(1), None, Some(0)]);
        let order = graph.shift(1, true).unwrap();
        assert_eq!(ids(&graph), ["e", "p", "o"]);
        assert_eq!(order, [Some(1), Some(0), Some(2)]);
        let order = graph.remove(0).unwrap();
        assert_eq!(ids(&graph), ["p", "o"]);
        assert_eq!(order, [Some(1), Some(2)]);
        assert!(graph.remove(5).is_none());
    }

    #[test]
    fn new_steps_and_flows_start_from_the_web_clients_defaults() {
        let mut graph = FlowGraph::default();
        graph.set_trigger_kind(trigger_kind("mqtt").unwrap());
        assert_eq!(
            graph.trigger_json(),
            "{\n  \"type\": \"mqtt\",\n  \"topic\": \"sensors/+/temp\",\n  \"debounce_ms\": 1000\n}"
        );
        for kind in NODE_KINDS {
            let id = graph.fresh_id(1);
            graph.add(kind, None, id);
        }
        assert!(graph.validate().is_ok(), "{:?}", graph.validate());
        assert_eq!(graph.fresh_id(1), "n12", "ids n1…n11 are taken");
        let first = graph
            .nodes
            .iter()
            .find(|node| node["type"] == "http_out")
            .unwrap();
        let keys: Vec<&str> = first.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            ["id", "type", "url", "headers", "timeout_ms", "retry_count"]
        );
        let flow = Flow::blank(
            "flow_1",
            "New Flow",
            true,
            trigger_kind("modbus").unwrap(),
            9,
        );
        assert_eq!(
            flow.trigger_config,
            "{\n  \"type\": \"modbus\",\n  \"device_id\": \"*\",\n  \"metric\": \"\",\n  \"debounce_ms\": 1000,\n  \"on_change_only\": false\n}"
        );
        assert_eq!(flow.nodes, "[]");
        assert_eq!((flow.created_at, flow.updated_at), (0, 9));
        let palette_ids: Vec<&str> = palette(Category::Process).map(|kind| kind.id).collect();
        assert_eq!(
            palette_ids,
            [
                "filter",
                "transform",
                "threshold",
                "lookup",
                "expression",
                "script",
                "dsl"
            ]
        );
    }

    #[test]
    fn validation_mirrors_what_the_server_reads() {
        let mut graph = FlowGraph::of(&stored()).unwrap();
        graph.nodes[2].insert("decimals".into(), json!(2.5));
        assert_eq!(
            graph.validate().unwrap_err(),
            "Step 3 (Data Source): decimals must be a whole number"
        );
        graph.nodes[2].insert("decimals".into(), json!(2));
        graph.nodes[1].shift_remove("expr");
        assert_eq!(
            graph.validate().unwrap_err(),
            "Step 2 (Expression): expr is missing"
        );
        graph.nodes[1].insert("expr".into(), json!("1"));
        graph.trigger.insert("debounce_ms".into(), json!(-1));
        assert!(
            graph
                .validate()
                .unwrap_err()
                .starts_with("Trigger: debounce_ms")
        );
        graph.trigger.insert("debounce_ms".into(), Value::Null);
        assert!(graph.validate().is_ok(), "null is an empty Option");
        graph.nodes[0].insert("type".into(), json!("sql"));
        assert!(graph.validate().unwrap_err().contains("no step type “sql”"));
        let rules = FlowGraph::parse(
            r#"{"type":"timer","interval_ms":5}"#,
            r#"[{"id":"t","type":"threshold","field":"f","rules":[{"gt":"80"}]}]"#,
        )
        .unwrap();
        assert!(rules.validate().unwrap_err().contains("rules"));
        assert!(FlowGraph::parse("[]", "[]").is_err());
        assert!(
            FlowGraph::parse("{}", "[1]")
                .unwrap_err()
                .contains("Step 1")
        );
    }

    #[test]
    fn fields_follow_the_value_types_and_numbers_stay_whole() {
        let graph = FlowGraph::of(&stored()).unwrap();
        let fields = node_fields(&graph.nodes[2]);
        assert_eq!(
            fields,
            [
                ("value_field".to_owned(), FieldKind::Text),
                ("source_key".to_owned(), FieldKind::Text),
                ("name".to_owned(), FieldKind::Text),
                ("unit".to_owned(), FieldKind::Text),
                ("decimals".to_owned(), FieldKind::Number),
            ]
        );
        assert_eq!(field_kind(&Value::Null), FieldKind::Json);
        assert_eq!(parse_number("5000").unwrap().to_string(), "5000");
        assert_eq!(parse_number(" 0.5 ").unwrap().to_string(), "0.5");
        assert!(parse_number("").is_err());
        assert!(parse_number("fast").is_err());
        assert_eq!(field_text(&json!({ "a": 1 })), "{\n  \"a\": 1\n}");
        assert_eq!(parse_json("{\"a\": 1}").unwrap(), json!({ "a": 1 }));
        assert!(parse_json("{a").is_err());
        let trigger = &graph.trigger;
        assert_eq!(trigger_value(trigger, "debounce_ms"), json!(1000));
        assert_eq!(trigger_value(&Map::new(), "interval_ms"), json!(5000));
        assert_eq!(trigger_fields("modbus").len(), 4);
        assert_eq!(preview(&graph.nodes[0]), "payload.metrics.temp > 0");
        assert_eq!(preview(&graph.nodes[2]), "");
        let transform = object(json!({ "map": { "status": "x" } }));
        assert_eq!(preview(&transform), "{\"status\":\"x\"}");
    }

    #[test]
    fn the_meta_form_needs_an_id_and_a_name() {
        let flows = vec![stored()];
        assert!(validate_meta("", "x", Some(&flows)).is_err());
        assert!(validate_meta("a", " ", None).is_err());
        assert!(validate_meta("flow_1761390000000", "x", Some(&flows)).is_err());
        assert!(validate_meta("a/b", "x", Some(&flows)).is_err());
        assert!(validate_meta(".", "x", Some(&flows)).is_err());
        assert!(validate_meta("..", "x", Some(&flows)).is_err());
        assert!(validate_meta("flow_2", "x", Some(&flows)).is_ok());
        assert!(
            validate_meta("flow_1761390000000", "x", None).is_ok(),
            "editing"
        );
    }

    #[test]
    fn runs_and_test_results_decode_in_the_servers_shape() {
        let runs: FlowRuns = serde_json::from_str(
            r#"{"runs": [
                {"id": 9, "flow_id": "f", "started_at": 1761390000, "duration_ms": 3,
                 "success": true, "error": null, "output": "{\"power_kw\":1.2}"},
                {"id": 8, "flow_id": "f", "started_at": 1761389990, "duration_ms": 1,
                 "success": false, "error": "field not found", "output": null}
            ]}"#,
        )
        .unwrap();
        assert_eq!(runs.runs.len(), 2);
        assert_eq!(runs.runs[1].error.as_deref(), Some("field not found"));
        let result: TestResult = serde_json::from_str(
            r#"{"success": true, "output": {"metrics": {"temp": 85}},
                "node_results": [
                  {"node_id": "n1", "result": "continue", "output": {"metrics": {"temp": 85}}},
                  {"node_id": "n2", "result": "filter_reject"},
                  {"node_id": "n3", "error": "boom"}
                ]}"#,
        )
        .unwrap();
        let outcomes: Vec<&str> = result
            .node_results
            .iter()
            .map(NodeResult::outcome)
            .collect();
        assert_eq!(outcomes, ["continue", "filter_reject", "error"]);
        assert_eq!(
            test_body(serde_json::from_str(TEST_PAYLOAD).unwrap()),
            json!({ "payload": { "metrics": { "temp": 85 } } })
        );
        assert_eq!(flow_path("flow 1"), "/api/v1/flows/flow%201");
        assert_eq!(flow_action_path("f", "runs"), "/api/v1/flows/f/runs");
    }
}
