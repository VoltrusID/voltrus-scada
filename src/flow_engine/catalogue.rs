//! The node and trigger catalogue served at `GET /api/v1/flows/node-types`.
//! Both flow editors build their palette and config forms from it.

use serde_json::{Value, json};

use super::node::{NODE_TYPES, is_side_effect_type, static_outputs};
use super::trigger::TRIGGER_TYPES;

/// One config field. `kind` is one of string, number, bool, select, expression,
/// code, tag, device, json, duration_ms, cron, severity.
struct F {
    key: &'static str,
    label: &'static str,
    kind: &'static str,
    required: bool,
    default: Option<Value>,
    options: &'static [(&'static str, &'static str)],
    help: &'static str,
    extra: Option<(&'static str, Value)>,
}

fn f(key: &'static str, label: &'static str, kind: &'static str) -> F {
    F {
        key,
        label,
        kind,
        required: false,
        default: None,
        options: &[],
        help: "",
        extra: None,
    }
}

impl F {
    fn req(mut self) -> Self {
        self.required = true;
        self
    }
    fn def(mut self, v: Value) -> Self {
        self.default = Some(v);
        self
    }
    fn opts(mut self, o: &'static [(&'static str, &'static str)]) -> Self {
        self.options = o;
        self
    }
    fn help(mut self, h: &'static str) -> Self {
        self.help = h;
        self
    }
    /// For `json` fields that hold a list of objects: the fields of one item,
    /// so an editor can offer a list form instead of raw JSON.
    fn items(mut self, fields: Vec<F>) -> Self {
        self.extra = Some((
            "item_fields",
            Value::Array(fields.into_iter().map(F::into_json).collect()),
        ));
        self
    }
    /// The field takes several values (a list of ids for `tag`).
    fn multiple(mut self) -> Self {
        self.extra = Some(("multiple", json!(true)));
        self
    }
    fn into_json(self) -> Value {
        let mut v = json!({
            "key": self.key,
            "label": self.label,
            "kind": self.kind,
            "required": self.required,
            "help": self.help,
        });
        let obj = v.as_object_mut().expect("object");
        if let Some(d) = self.default {
            obj.insert("default".into(), d);
        }
        if self.kind == "select" {
            obj.insert(
                "options".into(),
                self.options
                    .iter()
                    .map(|(value, label)| json!({"value": value, "label": label}))
                    .collect(),
            );
        }
        if let Some((k, x)) = self.extra {
            obj.insert(k.into(), x);
        }
        v
    }
}

const SEVERITIES: &[(&str, &str)] = &[
    ("critical", "Critical"),
    ("warning", "Warning"),
    ("info", "Info"),
];

fn node_meta(t: &str) -> (&'static str, &'static str, &'static str, Vec<F>) {
    match t {
        "filter" => (
            "Filter",
            "logic",
            "Lets the message out on pass when the condition is true, on reject otherwise.",
            vec![
                f("condition", "Condition", "expression")
                    .req()
                    .help("Uses payload fields, e.g. metrics.temp > 50 && status == \"online\"."),
            ],
        ),
        "switch" => (
            "Switch",
            "logic",
            "Routes the message by rules checked in order. Each rule has its own output, in rule order, followed by otherwise.",
            vec![
                f("rules", "Rules", "json")
                    .req()
                    .def(json!([{"name": "high", "expression": "value > 50"}]))
                    .help("List of {name, expression}. Rule outputs come first, then otherwise.")
                    .items(vec![
                        f("name", "Output name", "string").help("Empty: rule 1, rule 2, ..."),
                        f("expression", "Condition", "expression").req(),
                    ]),
                f("mode", "Match", "select")
                    .def(json!("first"))
                    .opts(&[
                        ("first", "First matching rule"),
                        ("all", "Every matching rule"),
                    ]),
            ],
        ),
        "threshold" => (
            "Threshold",
            "logic",
            "Checks a numeric field against rules in order; the first rule that matches sets its fields and the message leaves on above. No match leaves on below.",
            vec![
                f("field", "Field", "string").req().help("Payload path, e.g. metrics.temp."),
                f("rules", "Rules", "json")
                    .req()
                    .def(json!([{"gt": 50, "set": {"level": "high"}}]))
                    .help("List of {gt, gte, lt, lte, eq, set}; all given bounds must hold.")
                    .items(vec![
                        f("gt", "Greater than", "number"),
                        f("gte", "At least", "number"),
                        f("lt", "Less than", "number"),
                        f("lte", "At most", "number"),
                        f("eq", "Equals", "number"),
                        f("set", "Set fields", "json").help("Payload fields set when the rule matches."),
                    ]),
            ],
        ),
        "on_change" => (
            "On change",
            "logic",
            "Lets the message out on changed only when the field moved by more than the deadband since the last message that passed. The first message always passes.",
            vec![
                f("field", "Field", "string").req().help("Payload path, e.g. value."),
                f("deadband", "Deadband", "number")
                    .def(json!(0))
                    .help("Smallest change that counts. Non-numeric values pass on any change."),
            ],
        ),
        "transform" => (
            "Transform",
            "transform",
            "Builds a new payload: each output field is set from an expression on the old one.",
            vec![
                f("map", "Fields", "json")
                    .def(json!({}))
                    .help("Object of output field path -> expression."),
            ],
        ),
        "expression" => (
            "Expression",
            "transform",
            "Sets one payload field to the result of an expression.",
            vec![
                f("output_field", "Output field", "string").req(),
                f("expr", "Expression", "expression").req(),
            ],
        ),
        "lookup" => (
            "Lookup",
            "transform",
            "Looks a field's value up in a table and stores the match (or the default) in another field.",
            vec![
                f("field", "Field", "string").req(),
                f("output_field", "Output field", "string").req(),
                f("table", "Table", "json").def(json!({})).help("Object of value -> result."),
                f("default", "Default", "json").help("Result when the value is not in the table."),
            ],
        ),
        "dsl" => (
            "DSL",
            "advanced",
            "Runs a small Voltrus DSL program on the payload. A program that returns nothing drops the message.",
            vec![f("source", "Program", "code").req()],
        ),
        "script" => (
            "Script",
            "advanced",
            "Runs a saved JavaScript script with the chosen payload fields as input and merges its result into the payload. A failing script sends the message to error.",
            vec![
                f("script_id", "Script id", "number").req().help("Id of a script from Scripts."),
                f("inputs", "Input fields", "json")
                    .def(json!([]))
                    .help("List of payload fields passed to the script."),
            ],
        ),
        "delay" => (
            "Delay",
            "timing",
            "Waits before sending the message on (at most one hour). The flow does not start another run while it waits. Test runs do not wait.",
            vec![f("duration_ms", "Delay", "duration_ms").req().def(json!(1000))],
        ),
        "rate_limit" => (
            "Rate limit",
            "timing",
            "Lets at most max messages through per period. Extra messages leave on dropped, or with keep latest the newest one is held and sent on when the period allows.",
            vec![
                f("max", "Max messages", "number").req().def(json!(1)),
                f("duration_ms", "Per", "duration_ms").req().def(json!(60000)),
                f("mode", "Extra messages", "select")
                    .def(json!("drop"))
                    .opts(&[
                        ("drop", "Drop (send on dropped)"),
                        ("keep_latest", "Keep the latest"),
                    ]),
            ],
        ),
        "write_out" => (
            "Write tag",
            "output",
            "Writes a value to a tag through the command queue, under the tag's write policy (writable, range, deadband, rate, reason). A refused write goes to error.",
            vec![
                f("data_source_id", "Tag", "tag").req(),
                f("value_expr", "Value", "expression")
                    .help("Expression for the value. Empty uses the first number in the payload."),
                f("reason", "Reason", "string").help("Shown in the command log; defaults to the flow."),
            ],
        ),
        "alarm_out" => (
            "Raise alarm",
            "output",
            "Raises an alarm.",
            vec![
                f("severity", "Severity", "severity").req().def(json!("warning")),
                f("source_key", "Source key", "string").req(),
                f("message", "Message", "string").req(),
            ],
        ),
        "notify" => (
            "Notify",
            "output",
            "Sends a notification. Placeholders {{field.path}}, {{payload}}, {{flow_id}} and {{error}} are filled from the message. A failed delivery goes to error.",
            vec![
                f("channel", "Send through", "select")
                    .req()
                    .def(json!("settings"))
                    .opts(&[
                        ("settings", "Notification settings of a source"),
                        ("email", "Email"),
                        ("webhook", "Webhook"),
                    ]),
                f("to", "To", "string")
                    .help("Email addresses or webhook URLs, comma-separated (email and webhook)."),
                f("source_key", "Source key", "string").help(
                    "Whose notification settings to use (settings channel); also shown as the source.",
                ),
                f("severity", "Severity", "severity").def(json!("info")),
                f("subject", "Subject", "string"),
                f("message", "Message", "string").req(),
            ],
        ),
        "http_out" => (
            "HTTP request",
            "output",
            "POSTs the payload as JSON to a URL. A failed request goes to error.",
            vec![
                f("url", "URL", "string").req(),
                f("headers", "Headers", "json").def(json!({})),
                f("timeout_ms", "Timeout", "duration_ms").def(json!(5000)),
                f("retry_count", "Retries", "number").def(json!(0)),
            ],
        ),
        "data_source_out" => (
            "Set flow tag",
            "output",
            "Publishes a numeric payload field as a live value under the device flow_<flow id>.",
            vec![
                f("value_field", "Value field", "string").req(),
                f("source_key", "Metric name", "string").req(),
                f("name", "Display name", "string"),
                f("unit", "Unit", "string"),
                f("decimals", "Decimals", "number").def(json!(2)),
            ],
        ),
        "debug" => (
            "Debug",
            "advanced",
            "Records the message (or one field) in the run trace and sends it on unchanged.",
            vec![f("field", "Field", "string").help("Payload path to record; empty records the whole message.")],
        ),
        _ => ("", "advanced", "", vec![]),
    }
}

fn trigger_meta(t: &str) -> (&'static str, &'static str, Vec<F>) {
    match t {
        "manual" => (
            "Manual",
            "Runs only from the run button or POST /api/v1/flows/{id}/run.",
            vec![],
        ),
        "tag_change" => (
            "Tag change",
            "Fires when one of the tags (any protocol) changes by more than the deadband. Payload: data_source_id, key, device_id, metric, value, previous, quality, ts (ms). The first value seen after start only sets the baseline.",
            vec![
                f("data_source_ids", "Tags", "tag").req().multiple(),
                f("deadband", "Deadband", "number")
                    .def(json!(0))
                    .help("Smallest change that fires."),
            ],
        ),
        "modbus" => (
            "Device poll",
            "Fires on every poll of a device (any protocol). Payload: device_id, status, timestamp, metrics, simulated.",
            vec![
                f("device_id", "Device", "device")
                    .req()
                    .help("\"*\" for every device."),
                f("metric", "Metric", "string").help("Only fire when the poll has this metric."),
                f("debounce_ms", "Debounce", "duration_ms").help("Minimum time between runs."),
                f("on_change_only", "Only on change", "bool")
                    .def(json!(false))
                    .help("Skip polls where the metric (or all metrics) did not change."),
            ],
        ),
        "mqtt" => (
            "MQTT message",
            "Fires on messages to the built-in broker on a topic filter (+ and # wildcards). A JSON body becomes the payload; other bodies become {value}.",
            vec![
                f("topic", "Topic filter", "string").req(),
                f("debounce_ms", "Debounce", "duration_ms"),
            ],
        ),
        "alarm" => (
            "Alarm",
            "Fires when an alarm is raised, cleared or acknowledged. Payload: event, alarm_id, source_key, severity, message, value, state, actor, ts.",
            vec![
                f("on", "On", "select").req().def(json!("raised")).opts(&[
                    ("raised", "Raised"),
                    ("cleared", "Cleared"),
                    ("acknowledged", "Acknowledged"),
                    ("any", "Any of these"),
                ]),
                f("severity", "Severity", "severity").help("Only alarms of this severity."),
                f("source_key", "Source key", "string").help("Only alarms of this source."),
            ],
        ),
        "timer" => (
            "Interval",
            "Fires every interval (checked once a second). Payload: timestamp.",
            vec![
                f("interval_ms", "Every", "duration_ms")
                    .req()
                    .def(json!(60000)),
            ],
        ),
        "schedule" => (
            "Schedule",
            "Fires on a cron schedule in the server's local time (minute hour day month weekday, or @hourly, @daily, @weekly, @monthly). Payload: timestamp.",
            vec![f("cron", "Cron", "cron").req().def(json!("0 * * * *"))],
        ),
        "webhook" => (
            "Webhook",
            "Fires on POST /api/v1/flows/{id}/webhook with the flow's token in the X-Flow-Token header (an admin creates it with POST /api/v1/flows/{id}/webhook-token). The JSON body becomes the payload. Rate-limited.",
            vec![],
        ),
        _ => ("", "", vec![]),
    }
}

/// The catalogue document.
pub fn catalogue() -> Value {
    let triggers: Vec<Value> = TRIGGER_TYPES
        .iter()
        .map(|t| {
            let (label, description, fields) = trigger_meta(t);
            json!({
                "type": t,
                "label": label,
                "description": description,
                "fields": fields.into_iter().map(F::into_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let nodes: Vec<Value> = NODE_TYPES
        .iter()
        .map(|t| {
            let (label, category, description, fields) = node_meta(t);
            let mut v = json!({
                "type": t,
                "label": label,
                "category": category,
                "description": description,
                "outputs": static_outputs(t),
                "side_effect": is_side_effect_type(t),
                "fields": fields.into_iter().map(F::into_json).collect::<Vec<_>>(),
            });
            if *t == "switch" {
                v["dynamic_outputs"] = json!("rules");
            }
            v
        })
        .collect();
    json!({ "triggers": triggers, "nodes": nodes })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KINDS: &[&str] = &[
        "string",
        "number",
        "bool",
        "select",
        "expression",
        "code",
        "tag",
        "device",
        "json",
        "duration_ms",
        "cron",
        "severity",
    ];

    fn check_fields(owner: &str, fields: &Value) {
        for field in fields.as_array().unwrap() {
            let kind = field["kind"].as_str().unwrap();
            assert!(KINDS.contains(&kind), "{owner}: kind {kind}");
            assert!(!field["key"].as_str().unwrap().is_empty(), "{owner}");
            assert!(!field["label"].as_str().unwrap().is_empty(), "{owner}");
            assert!(field["required"].is_boolean(), "{owner}");
            if kind == "select" {
                assert!(!field["options"].as_array().unwrap().is_empty(), "{owner}");
            }
            if let Some(items) = field.get("item_fields") {
                check_fields(owner, items);
            }
        }
    }

    #[test]
    fn every_type_is_described() {
        let c = catalogue();
        for n in c["nodes"].as_array().unwrap() {
            let t = n["type"].as_str().unwrap();
            assert!(!n["label"].as_str().unwrap().is_empty(), "{t}");
            assert!(!n["description"].as_str().unwrap().is_empty(), "{t}");
            assert!(
                ["logic", "transform", "output", "timing", "advanced"]
                    .contains(&n["category"].as_str().unwrap()),
                "{t}"
            );
            assert!(!n["outputs"].as_array().unwrap().is_empty(), "{t}");
            check_fields(t, &n["fields"]);
        }
        for tr in c["triggers"].as_array().unwrap() {
            let t = tr["type"].as_str().unwrap();
            assert!(!tr["label"].as_str().unwrap().is_empty(), "{t}");
            check_fields(t, &tr["fields"]);
        }
        assert_eq!(c["nodes"].as_array().unwrap().len(), NODE_TYPES.len());
        assert_eq!(c["triggers"].as_array().unwrap().len(), TRIGGER_TYPES.len());
    }

    /// A node built from each type's defaults (and a placeholder for required
    /// fields without one) parses, so the editors' "new node" is valid.
    #[test]
    fn defaults_build_a_node_that_parses() {
        let c = catalogue();
        for n in c["nodes"].as_array().unwrap() {
            let t = n["type"].as_str().unwrap();
            let mut node = json!({"id": "x", "type": t});
            for field in n["fields"].as_array().unwrap() {
                let key = field["key"].as_str().unwrap();
                if let Some(d) = field.get("default") {
                    node[key] = d.clone();
                } else if field["required"] == true {
                    node[key] = match field["kind"].as_str().unwrap() {
                        "number" | "tag" => json!(1),
                        "expression" => json!("value > 1"),
                        "string" if key == "url" => json!("https://example.com"),
                        _ => json!("x"),
                    };
                }
            }
            let cfg: super::super::node::NodeConfig = serde_json::from_value(node.clone())
                .unwrap_or_else(|e| panic!("{t}: {e} ({node})"));
            assert_eq!(cfg.type_name(), t);
            assert_eq!(
                cfg.outputs().last().map(String::as_str),
                n["outputs"].as_array().unwrap().last().unwrap().as_str(),
                "{t}"
            );
        }
    }

    #[test]
    fn side_effect_nodes() {
        let c = catalogue();
        let side: Vec<&str> = c["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|n| n["side_effect"] == true)
            .map(|n| n["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            side,
            [
                "write_out",
                "alarm_out",
                "notify",
                "http_out",
                "data_source_out"
            ]
        );
    }
}
