#![allow(clippy::needless_range_loop)]
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

use super::message::Message;
use super::state::{FlowState, ParkedMessage};
use crate::cache::DeviceCache;
use crate::storage::Storage;

fn default_flow_policy() -> String {
    "immediate".to_string()
}
fn default_flow_reason() -> String {
    "flow:unknown".to_string()
}
fn default_switch_mode() -> String {
    "first".to_string()
}
fn default_rate_mode() -> String {
    "drop".to_string()
}
fn default_notify_channel() -> String {
    "settings".to_string()
}
fn default_severity() -> String {
    "info".to_string()
}
fn default_http_timeout_ms() -> u64 {
    5000
}
fn default_decimals() -> i64 {
    2
}

/// Longest wait a `delay` node may hold a message (one hour).
pub const MAX_DELAY_MS: u64 = 3_600_000;

/// Where a message goes after a node ran.
#[derive(Debug, Clone)]
pub enum NodeResult {
    /// Out of port 0.
    Continue(Message),
    /// Dropped: nothing leaves the node (a `dsl` program that returned nothing,
    /// a message a `rate_limit` node holds back).
    FilterReject,
    /// Out of one numbered port (filter's reject, threshold's "below", ...).
    Port(usize, Message),
    /// Out of several ports at once (a `switch` in `all` mode).
    Ports(Vec<(usize, Message)>),
}

/// A node's result plus a note for the run trace: what a side-effect node would
/// have done in a test run, or what a `debug` node recorded.
#[derive(Debug, Clone)]
pub struct NodeOutput {
    pub result: NodeResult,
    pub report: Option<Value>,
}

impl From<NodeResult> for NodeOutput {
    fn from(result: NodeResult) -> Self {
        Self {
            result,
            report: None,
        }
    }
}

#[derive(Clone)]
pub struct NodeContext {
    pub flow_id: String,
    pub node_id: String,
    pub state: FlowState,
    pub storage: Storage,
    pub cache: DeviceCache,
    /// Test run: side-effect nodes describe what they would do instead of doing it,
    /// and `delay` does not wait.
    pub dry_run: bool,
    /// SMTP settings for `notify` nodes that send email.
    pub smtp: Option<crate::models::SmtpConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ThresholdRule {
    pub gt: Option<f64>,
    pub gte: Option<f64>,
    pub lt: Option<f64>,
    pub lte: Option<f64>,
    pub eq: Option<f64>,
    #[serde(default)]
    pub set: HashMap<String, Value>,
}

/// One output of a `switch` node: the message leaves on this rule's port when
/// `expression` is true.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SwitchRule {
    #[serde(default)]
    pub name: String,
    pub expression: String,
}

/// A whole number >= 0 that may also arrive as `5.0` or `"5"`, which form
/// editors produce.
fn de_u64<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let v = Value::deserialize(d)?;
    let err = || serde::de::Error::custom("expected a whole number >= 0");
    match &v {
        Value::Number(n) => n
            .as_u64()
            .or_else(|| {
                n.as_f64()
                    .filter(|f| *f >= 0.0 && f.fract() == 0.0)
                    .map(|f| f as u64)
            })
            .ok_or_else(err),
        Value::String(s) => s.trim().parse().map_err(|_| err()),
        _ => Err(err()),
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum NodeConfig {
    #[serde(rename = "filter")]
    Filter { id: String, condition: String },
    #[serde(rename = "transform")]
    Transform {
        id: String,
        #[serde(default)]
        map: HashMap<String, String>,
    },
    #[serde(rename = "threshold")]
    Threshold {
        id: String,
        field: String,
        rules: Vec<ThresholdRule>,
    },
    #[serde(rename = "lookup")]
    Lookup {
        id: String,
        field: String,
        output_field: String,
        #[serde(default)]
        table: HashMap<String, Value>,
        #[serde(default)]
        default: Value,
    },
    #[serde(rename = "expression")]
    Expression {
        id: String,
        output_field: String,
        expr: String,
    },
    #[serde(rename = "data_source_out")]
    DataSourceOut {
        id: String,
        value_field: String,
        source_key: String,
        #[serde(default)]
        name: String,
        #[serde(default)]
        unit: String,
        #[serde(default = "default_decimals")]
        decimals: i64,
    },
    #[serde(rename = "http_out")]
    HttpOut {
        id: String,
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default = "default_http_timeout_ms")]
        timeout_ms: u64,
        #[serde(default)]
        retry_count: u64,
    },
    #[serde(rename = "alarm_out")]
    AlarmOut {
        id: String,
        severity: String,
        source_key: String,
        message: String,
    },
    #[serde(rename = "write_out")]
    WriteOut {
        id: String,
        data_source_id: i64,
        #[serde(default)]
        value_expr: Option<String>,
        #[serde(default = "default_flow_policy")]
        policy: String,
        #[serde(default = "default_flow_reason")]
        reason: String,
    },
    #[serde(rename = "script")]
    Script {
        id: String,
        script_id: i64,
        #[serde(default)]
        inputs: Vec<String>,
    },
    #[serde(rename = "dsl")]
    Dsl {
        id: String,
        #[serde(default)]
        source: String,
    },
    #[serde(rename = "switch")]
    Switch {
        id: String,
        rules: Vec<SwitchRule>,
        /// `first`: only the first matching rule's port; `all`: every matching rule's port.
        #[serde(default = "default_switch_mode")]
        mode: String,
    },
    #[serde(rename = "delay")]
    Delay {
        id: String,
        #[serde(deserialize_with = "de_u64")]
        duration_ms: u64,
    },
    #[serde(rename = "rate_limit")]
    RateLimit {
        id: String,
        #[serde(deserialize_with = "de_u64")]
        max: u64,
        #[serde(deserialize_with = "de_u64")]
        duration_ms: u64,
        /// `drop`: extra messages leave on the `dropped` port; `keep_latest`:
        /// the newest extra message is held and sent on when the window opens.
        #[serde(default = "default_rate_mode")]
        mode: String,
    },
    #[serde(rename = "on_change")]
    OnChange {
        id: String,
        field: String,
        #[serde(default)]
        deadband: f64,
    },
    #[serde(rename = "notify")]
    Notify {
        id: String,
        /// `settings` (the notification settings of `source_key`), `email` or `webhook`.
        #[serde(default = "default_notify_channel")]
        channel: String,
        /// Comma-separated email addresses or webhook URLs (email / webhook channels).
        #[serde(default)]
        to: String,
        #[serde(default)]
        source_key: String,
        #[serde(default = "default_severity")]
        severity: String,
        #[serde(default)]
        subject: String,
        message: String,
    },
    #[serde(rename = "debug")]
    Debug {
        id: String,
        /// Record only this payload path; empty records the whole message.
        #[serde(default)]
        field: String,
    },
}

/// Every node type, in catalogue order.
pub const NODE_TYPES: &[&str] = &[
    "filter",
    "switch",
    "threshold",
    "on_change",
    "transform",
    "expression",
    "lookup",
    "dsl",
    "script",
    "delay",
    "rate_limit",
    "write_out",
    "alarm_out",
    "notify",
    "http_out",
    "data_source_out",
    "debug",
];

/// Output port names of a node type with fixed outputs (`switch` adds one port
/// per rule in front of `otherwise`).
pub fn static_outputs(node_type: &str) -> &'static [&'static str] {
    match node_type {
        "filter" => &["pass", "reject"],
        "threshold" => &["above", "below"],
        "http_out" | "write_out" | "script" | "notify" => &["out", "error"],
        "switch" => &["otherwise"],
        "rate_limit" => &["out", "dropped"],
        "on_change" => &["changed", "unchanged"],
        _ => &["out"],
    }
}

/// Whether a node type acts outside the flow (device writes, HTTP, alarms,
/// notifications, tag values). Test runs never run these for real.
pub fn is_side_effect_type(node_type: &str) -> bool {
    matches!(
        node_type,
        "write_out" | "http_out" | "notify" | "alarm_out" | "data_source_out"
    )
}

impl NodeConfig {
    pub fn id(&self) -> &str {
        match self {
            NodeConfig::Filter { id, .. }
            | NodeConfig::Transform { id, .. }
            | NodeConfig::Threshold { id, .. }
            | NodeConfig::Lookup { id, .. }
            | NodeConfig::Expression { id, .. }
            | NodeConfig::DataSourceOut { id, .. }
            | NodeConfig::HttpOut { id, .. }
            | NodeConfig::AlarmOut { id, .. }
            | NodeConfig::WriteOut { id, .. }
            | NodeConfig::Script { id, .. }
            | NodeConfig::Dsl { id, .. }
            | NodeConfig::Switch { id, .. }
            | NodeConfig::Delay { id, .. }
            | NodeConfig::RateLimit { id, .. }
            | NodeConfig::OnChange { id, .. }
            | NodeConfig::Notify { id, .. }
            | NodeConfig::Debug { id, .. } => id,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            NodeConfig::Filter { .. } => "filter",
            NodeConfig::Transform { .. } => "transform",
            NodeConfig::Threshold { .. } => "threshold",
            NodeConfig::Lookup { .. } => "lookup",
            NodeConfig::Expression { .. } => "expression",
            NodeConfig::DataSourceOut { .. } => "data_source_out",
            NodeConfig::HttpOut { .. } => "http_out",
            NodeConfig::AlarmOut { .. } => "alarm_out",
            NodeConfig::WriteOut { .. } => "write_out",
            NodeConfig::Script { .. } => "script",
            NodeConfig::Dsl { .. } => "dsl",
            NodeConfig::Switch { .. } => "switch",
            NodeConfig::Delay { .. } => "delay",
            NodeConfig::RateLimit { .. } => "rate_limit",
            NodeConfig::OnChange { .. } => "on_change",
            NodeConfig::Notify { .. } => "notify",
            NodeConfig::Debug { .. } => "debug",
        }
    }

    /// Output port names, in port order.
    pub fn outputs(&self) -> Vec<String> {
        match self {
            NodeConfig::Switch { rules, .. } => rules
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    if r.name.trim().is_empty() {
                        format!("rule {}", i + 1)
                    } else {
                        r.name.clone()
                    }
                })
                .chain(std::iter::once("otherwise".to_string()))
                .collect(),
            other => static_outputs(other.type_name())
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// The port a failed run of this node sends the message to, if it has one.
    pub fn error_port(&self) -> Option<usize> {
        match self {
            NodeConfig::HttpOut { .. }
            | NodeConfig::WriteOut { .. }
            | NodeConfig::Script { .. }
            | NodeConfig::Notify { .. } => Some(1),
            _ => None,
        }
    }

    pub fn is_side_effect(&self) -> bool {
        is_side_effect_type(self.type_name())
    }

    /// Check the configuration beyond its shape (expressions parse, values in
    /// range), so a broken flow is refused when saved instead of failing later.
    pub fn validate(&self) -> Result<(), String> {
        let expr = |what: &str, e: &str| -> Result<(), String> {
            if e.trim().is_empty() {
                return Err(format!("{what} is empty"));
            }
            crate::expression::check_syntax(e).map_err(|err| format!("{what}: {err}"))
        };
        match self {
            NodeConfig::Filter { condition, .. } => expr("condition", condition),
            NodeConfig::Transform { map, .. } => {
                for (k, v) in map {
                    expr(&format!("map[{k}]"), v)?;
                }
                Ok(())
            }
            NodeConfig::Threshold { field, .. } if field.trim().is_empty() => {
                Err("field is empty".into())
            }
            NodeConfig::OnChange {
                field, deadband, ..
            } => {
                if field.trim().is_empty() {
                    Err("field is empty".into())
                } else if deadband.is_nan() || *deadband < 0.0 {
                    Err("deadband must be 0 or more".into())
                } else {
                    Ok(())
                }
            }
            NodeConfig::Expression {
                expr: e,
                output_field,
                ..
            } => {
                if output_field.trim().is_empty() {
                    return Err("output_field is empty".into());
                }
                expr("expr", e)
            }
            NodeConfig::WriteOut { value_expr, .. } => match value_expr {
                Some(e) if !e.trim().is_empty() => expr("value_expr", e),
                _ => Ok(()),
            },
            NodeConfig::HttpOut { url, .. } => {
                if url.starts_with("http://") || url.starts_with("https://") {
                    Ok(())
                } else {
                    Err("url must start with http:// or https://".into())
                }
            }
            NodeConfig::Switch { rules, mode, .. } => {
                if rules.is_empty() {
                    return Err("a switch needs at least one rule".into());
                }
                if mode != "first" && mode != "all" {
                    return Err(format!("mode must be \"first\" or \"all\", not {mode:?}"));
                }
                for (i, r) in rules.iter().enumerate() {
                    expr(&format!("rule {} expression", i + 1), &r.expression)?;
                }
                Ok(())
            }
            NodeConfig::Delay { duration_ms, .. } => {
                if *duration_ms > MAX_DELAY_MS {
                    Err(format!(
                        "duration_ms must be at most {MAX_DELAY_MS} (one hour)"
                    ))
                } else {
                    Ok(())
                }
            }
            NodeConfig::RateLimit {
                max,
                duration_ms,
                mode,
                ..
            } => {
                if *max == 0 {
                    return Err("max must be at least 1".into());
                }
                if *duration_ms == 0 {
                    return Err("duration_ms must be at least 1".into());
                }
                if mode != "drop" && mode != "keep_latest" {
                    return Err(format!(
                        "mode must be \"drop\" or \"keep_latest\", not {mode:?}"
                    ));
                }
                Ok(())
            }
            NodeConfig::Notify {
                channel,
                to,
                source_key,
                message,
                ..
            } => {
                if message.trim().is_empty() {
                    return Err("message is empty".into());
                }
                match channel.as_str() {
                    "settings" if source_key.trim().is_empty() => Err(
                        "the settings channel needs the source_key whose notification settings to use"
                            .into(),
                    ),
                    "settings" => Ok(()),
                    "email" | "webhook" if split_targets(to).is_empty() => Err(format!(
                        "the {channel} channel needs at least one recipient in to"
                    )),
                    "email" | "webhook" => Ok(()),
                    other => Err(format!(
                        "channel must be settings, email or webhook, not {other:?}"
                    )),
                }
            }
            _ => Ok(()),
        }
    }

    pub async fn process(&self, msg: Message, ctx: &NodeContext) -> Result<NodeResult, String> {
        self.run(msg, ctx).await.map(|o| o.result)
    }

    /// Run the node on `msg`. A test run (`ctx.dry_run`) of a side-effect node
    /// returns what it would have done in `report` and changes nothing.
    pub async fn run(&self, mut msg: Message, ctx: &NodeContext) -> Result<NodeOutput, String> {
        // Clone the shared handles up front so the `&NodeContext` borrow ends
        // before any `.await` (keeps this future `Send` for tokio::spawn).
        let storage = ctx.storage.clone();
        let cache = ctx.cache.clone();
        let flow_id = ctx.flow_id.clone();
        let dry_run = ctx.dry_run;
        let state_key = format!("{}/{}", ctx.flow_id, ctx.node_id);
        match self {
            NodeConfig::Filter { condition, .. } => {
                let result = crate::expression::evaluate_json(condition, &msg.payload)?;
                if is_truthy(&result) {
                    Ok(NodeResult::Continue(msg).into())
                } else {
                    Ok(NodeResult::Port(1, msg).into())
                }
            }
            NodeConfig::Transform { map, .. } => {
                let mut new_payload = serde_json::json!({});
                for (path, expr) in map {
                    let value = crate::expression::evaluate_json(expr, &msg.payload)?;
                    set_json_path(&mut new_payload, path, value);
                }
                msg.payload = new_payload;
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::Threshold { field, rules, .. } => {
                let field_value = get_json_path(&msg.payload, field)?;
                let field_num = to_f64(&field_value)?;
                for rule in rules {
                    if rule_matches(field_num, rule) {
                        for (path, value) in &rule.set {
                            set_json_path(&mut msg.payload, path, value.clone());
                        }
                        return Ok(NodeResult::Continue(msg).into());
                    }
                }
                Ok(NodeResult::Port(1, msg).into())
            }
            NodeConfig::Lookup {
                field,
                output_field,
                table,
                default,
                ..
            } => {
                let field_value = get_json_path(&msg.payload, field)?;
                let lookup_key = match &field_value {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    _ => return Err("Unsupported lookup key type".to_string()),
                };
                let result = table
                    .get(&lookup_key)
                    .cloned()
                    .unwrap_or_else(|| default.clone());
                set_json_path(&mut msg.payload, output_field, result);
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::Expression {
                output_field, expr, ..
            } => {
                let result = crate::expression::evaluate_json(expr, &msg.payload)?;
                set_json_path(&mut msg.payload, output_field, result);
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::DataSourceOut {
                value_field,
                source_key,
                ..
            } => {
                let value = get_json_path(&msg.payload, value_field)?;
                let numeric_value = to_f64(&value)?;
                let device_id = format!("flow_{}", flow_id);
                if dry_run {
                    return Ok(NodeOutput {
                        result: NodeResult::Continue(msg),
                        report: Some(json!({"would_set": {
                            "device_id": device_id, "metric": source_key, "value": numeric_value,
                        }})),
                    });
                }
                let mut metrics = HashMap::new();
                metrics.insert(source_key.clone(), numeric_value);
                cache.update(&device_id, metrics, false).await;
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::HttpOut {
                url,
                headers,
                timeout_ms,
                retry_count,
                ..
            } => {
                if dry_run {
                    // Header values often carry credentials: name them only.
                    let mut names: Vec<&String> = headers.keys().collect();
                    names.sort();
                    return Ok(NodeOutput {
                        report: Some(json!({"would_post": {
                            "url": url, "headers": names, "body": msg.payload,
                        }})),
                        result: NodeResult::Continue(msg),
                    });
                }
                crate::security::outbound::check_url(url)
                    .map_err(|e| format!("HttpOut refused: {e}"))?;
                let client = crate::security::outbound::client(None);
                let mut last_error: Option<String> = None;
                for attempt in 0..=*retry_count {
                    let mut request = client.post(url).json(&msg.payload);
                    for (k, v) in headers {
                        request = request.header(k, v);
                    }
                    request = request.timeout(std::time::Duration::from_millis(*timeout_ms));
                    match request.send().await {
                        Ok(resp) if resp.status().is_success() => {
                            last_error = None;
                            break;
                        }
                        Ok(resp) => last_error = Some(format!("HTTP {}", resp.status())),
                        // The error text names the URL, password included: drop it
                        // (the run's error is readable by every signed-in user).
                        Err(e) => last_error = Some(e.without_url().to_string()),
                    }
                    if attempt < *retry_count {
                        tokio::time::sleep(std::time::Duration::from_millis(100 * (attempt + 1)))
                            .await;
                    }
                }
                if let Some(e) = last_error {
                    return Err(format!(
                        "HttpOut failed after {} retries: {}",
                        retry_count, e
                    ));
                }
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::AlarmOut {
                severity,
                source_key,
                message,
                ..
            } => {
                if dry_run {
                    return Ok(NodeOutput {
                        result: NodeResult::Continue(msg),
                        report: Some(json!({"would_raise_alarm": {
                            "source_key": source_key, "severity": severity, "message": message,
                        }})),
                    });
                }
                // "event": raised by the flow, not by a tag crossing a limit, so
                // limit evaluation never clears it.
                if let Err(e) = storage
                    .create_alarm_with_type(source_key, severity, 0.0, 0.0, message, 0.0, "event")
                    .await
                {
                    tracing::warn!("AlarmOut node: failed to create alarm: {}", e);
                    return Err(format!("AlarmOut failed: {}", e));
                }
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::WriteOut {
                data_source_id,
                value_expr,
                policy,
                reason,
                ..
            } => {
                // 1. Resolve the write value.
                let value_str = if let Some(expr) =
                    value_expr.as_deref().filter(|e| !e.trim().is_empty())
                {
                    let result = crate::expression::evaluate_json(expr, &msg.payload)?;
                    match &result {
                        Value::Number(n) => {
                            let f = n
                                .as_f64()
                                .ok_or_else(|| "Invalid number in expression result".to_string())?;
                            format!("{}", f)
                        }
                        Value::String(s) => s.clone(),
                        other => {
                            return Err(format!(
                                "WriteOut value_expr must evaluate to a number or string, got {:?}",
                                other
                            ));
                        }
                    }
                } else {
                    // Take the first numeric value from message payload.
                    let mut found: Option<String> = None;
                    if let Value::Object(map) = &msg.payload {
                        for (_key, v) in map {
                            if let Value::Number(n) = v
                                && let Some(f) = n.as_f64()
                            {
                                found = Some(format!("{}", f));
                                break;
                            }
                        }
                    }
                    found.ok_or_else(|| {
                        "WriteOut: no numeric value found in message payload".to_string()
                    })?
                };

                // 2. Enqueue through the tag's write policy (writable, range,
                //    deadband, rate limit, reason), like a write from the API.
                let value: f64 = value_str
                    .trim()
                    .parse()
                    .map_err(|_| format!("WriteOut: value {value_str:?} is not a number"))?;
                let reason = reason.trim();
                let reason = if reason.is_empty() || reason == "flow:unknown" {
                    format!("flow {flow_id}")
                } else {
                    reason.to_string()
                };
                if dry_run {
                    // The static part of the write policy, without touching the queue.
                    let check = preview_write_policy(&storage, *data_source_id, value).await;
                    return match check {
                        Ok(()) => Ok(NodeOutput {
                            report: Some(json!({"would_write": {
                                "data_source_id": data_source_id, "value": value,
                                "policy": policy, "reason": reason,
                            }})),
                            result: NodeResult::Continue(msg),
                        }),
                        Err(e) => Err(format!("WriteOut would be refused: {e}")),
                    };
                }
                let submission = crate::api::commands::TagWriteSubmission {
                    tag_id: *data_source_id,
                    value,
                    policy: policy.clone(),
                    scheduled_at: None,
                    reason: Some(reason),
                    command_id: None,
                };
                let username = format!("flow:{flow_id}");
                crate::api::commands::enqueue_checked_write(
                    &storage,
                    &cache,
                    &username,
                    crate::edition::ActionOrigin::Flow,
                    submission,
                )
                .await
                .map_err(|e| format!("WriteOut refused: {}", e.message()))?;
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::Script {
                script_id, inputs, ..
            } => {
                let result = match storage.get_script(*script_id).await {
                    Ok(Some(script)) if script.enabled && script.syntax_error.is_none() => {
                        let mut tag_input = serde_json::Map::new();
                        for key in inputs {
                            if let Some(val) = msg.payload.get(key) {
                                tag_input.insert(key.clone(), val.clone());
                            }
                        }
                        let input = serde_json::Value::Object(tag_input);
                        let result = crate::script_engine::run_script(
                            script.source.clone(),
                            input,
                            script.cpu_limit_ms,
                            script.memory_limit_bytes,
                        )
                        .await;
                        if !dry_run {
                            let _ = storage
                                .create_script_execution(
                                    *script_id,
                                    Some(&flow_id),
                                    None,
                                    result.duration_ms as i64,
                                    if result.success { "success" } else { "error" },
                                    result.error.as_deref(),
                                    result.output.as_deref(),
                                )
                                .await;
                        }
                        result
                    }
                    Ok(Some(_)) => crate::script_engine::ScriptResult {
                        success: false,
                        output: None,
                        error: Some("Script has syntax error or is disabled".into()),
                        duration_ms: 0,
                    },
                    _ => crate::script_engine::ScriptResult {
                        success: false,
                        output: None,
                        error: Some("Script not found".into()),
                        duration_ms: 0,
                    },
                };

                if !result.success {
                    return Err(result.error.unwrap_or_else(|| "Script failed".to_string()));
                }
                if let Some(ref out) = result.output
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(out)
                    && let serde_json::Value::Object(map) = parsed
                {
                    for (k, v) in map {
                        set_json_path(&mut msg.payload, &k, v);
                    }
                }
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::Dsl { source, .. } => {
                match crate::expression::run_dsl(source, &msg.payload) {
                    Ok(Some(v)) => {
                        msg.payload = v;
                        Ok(NodeResult::Continue(msg).into())
                    }
                    Ok(None) => Ok(NodeResult::FilterReject.into()),
                    Err(_) => Ok(NodeResult::Continue(msg).into()),
                }
            }
            NodeConfig::Switch { rules, mode, .. } => {
                let mut ports = Vec::new();
                for (i, rule) in rules.iter().enumerate() {
                    let v = crate::expression::evaluate_json(&rule.expression, &msg.payload)
                        .map_err(|e| format!("rule {}: {e}", i + 1))?;
                    if is_truthy(&v) {
                        ports.push(i);
                        if mode != "all" {
                            break;
                        }
                    }
                }
                match ports.len() {
                    0 => Ok(NodeResult::Port(rules.len(), msg).into()),
                    1 => Ok(NodeResult::Port(ports[0], msg).into()),
                    _ => Ok(NodeResult::Ports(
                        ports.into_iter().map(|p| (p, msg.clone())).collect(),
                    )
                    .into()),
                }
            }
            NodeConfig::Delay { duration_ms, .. } => {
                let ms = (*duration_ms).min(MAX_DELAY_MS);
                if dry_run {
                    return Ok(NodeOutput {
                        result: NodeResult::Continue(msg),
                        report: Some(json!({"delay_ms": ms, "waited": false})),
                    });
                }
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                Ok(NodeResult::Continue(msg).into())
            }
            NodeConfig::RateLimit {
                max,
                duration_ms,
                mode,
                ..
            } => {
                let window = std::time::Duration::from_millis(*duration_ms);
                let now = std::time::Instant::now();
                match ctx
                    .state
                    .rate_try_pass(&state_key, *max as usize, window, now)
                {
                    Ok(()) => Ok(NodeResult::Continue(msg).into()),
                    Err(_) if mode != "keep_latest" => Ok(NodeResult::Port(1, msg).into()),
                    Err(release_at) => {
                        let wait_ms = release_at.saturating_duration_since(now).as_millis() as u64;
                        ctx.state.park(ParkedMessage {
                            flow_id: ctx.flow_id.clone(),
                            node_id: ctx.node_id.clone(),
                            message: msg,
                            release_at,
                        });
                        Ok(NodeOutput {
                            result: NodeResult::FilterReject,
                            report: Some(json!({"held": true, "release_in_ms": wait_ms})),
                        })
                    }
                }
            }
            NodeConfig::OnChange {
                field, deadband, ..
            } => {
                let current = get_json_path(&msg.payload, field)?;
                let changed = match ctx.state.last_value(&state_key) {
                    None => true,
                    Some(prev) => match (prev.as_f64(), current.as_f64()) {
                        (Some(a), Some(b)) => (b - a).abs() > *deadband,
                        _ => prev != current,
                    },
                };
                if changed {
                    ctx.state.set_last_value(&state_key, current);
                    Ok(NodeResult::Continue(msg).into())
                } else {
                    Ok(NodeResult::Port(1, msg).into())
                }
            }
            NodeConfig::Notify {
                channel,
                to,
                source_key,
                severity,
                subject,
                message,
                ..
            } => {
                let text = render_template(message, &msg, &flow_id);
                let title = if subject.trim().is_empty() {
                    format!("Flow {flow_id}")
                } else {
                    render_template(subject, &msg, &flow_id)
                };
                let targets = split_targets(to);
                if dry_run {
                    return Ok(NodeOutput {
                        report: Some(json!({"would_notify": {
                            "channel": channel, "to": targets, "source_key": source_key,
                            "severity": severity, "subject": title, "message": text,
                        }})),
                        result: NodeResult::Continue(msg),
                    });
                }
                let delivery = crate::notification::Delivery {
                    severity: severity.clone(),
                    source_key: if source_key.is_empty() {
                        format!("flow:{flow_id}")
                    } else {
                        source_key.clone()
                    },
                    source_name: title,
                    value: 0.0,
                    threshold: 0.0,
                    unit: String::new(),
                    value_text: "0".to_string(),
                    threshold_text: "0".to_string(),
                    message: text,
                    triggered_at: crate::notification::webhook::format_triggered_at(
                        crate::timeutil::now_secs(),
                    ),
                    device_id: format!("flow_{flow_id}"),
                };
                let notifier = ctx
                    .smtp
                    .as_ref()
                    .and_then(|cfg| crate::notification::email::EmailNotifier::new(cfg).ok());
                let errors = match channel.as_str() {
                    "email" => {
                        let Some(notifier) = notifier else {
                            return Err("notify: SMTP is not configured".into());
                        };
                        let mut errors = Vec::new();
                        for t in &targets {
                            if let Err(e) =
                                crate::notification::send_email(&notifier, t, &delivery).await
                            {
                                errors.push(format!("email to {t}: {e}"));
                            }
                        }
                        errors
                    }
                    "webhook" => {
                        let mut errors = Vec::new();
                        for t in &targets {
                            if let Err(e) = crate::notification::send_webhook(t, &delivery).await {
                                errors.push(format!("webhook {t}: {e}"));
                            }
                        }
                        errors
                    }
                    _ => {
                        let settings = storage
                            .get_notification_settings_by_source(source_key)
                            .await
                            .map_err(|e| format!("notify: {e}"))?;
                        if !settings.iter().any(|s| s.enabled) {
                            return Err(format!(
                                "notify: no enabled notification settings for {source_key}"
                            ));
                        }
                        crate::notification::deliver_to_settings(
                            &settings,
                            &delivery,
                            notifier.as_ref(),
                        )
                        .await
                    }
                };
                if errors.is_empty() {
                    Ok(NodeResult::Continue(msg).into())
                } else {
                    Err(format!("notify: {}", errors.join("; ")))
                }
            }
            NodeConfig::Debug { field, .. } => {
                let recorded = if field.trim().is_empty() {
                    json!({"payload": msg.payload, "metadata": msg.metadata})
                } else {
                    get_json_path(&msg.payload, field).unwrap_or(Value::Null)
                };
                Ok(NodeOutput {
                    result: NodeResult::Continue(msg),
                    report: Some(json!({"debug": recorded})),
                })
            }
        }
    }
}

/// Split a comma-, semicolon- or newline-separated recipient list.
pub fn split_targets(to: &str) -> Vec<String> {
    to.split([',', '\n', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Fill `{{path}}` placeholders from the message: a payload path (`metrics.temp`,
/// also written `payload.metrics.temp`), `payload` (the whole payload as JSON),
/// `flow_id`, `error` (the error a node sent on its error port) or `meta.<key>`.
/// Unknown paths render empty.
pub fn render_template(template: &str, msg: &Message, flow_id: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let key = after[..end].trim();
        let value = match key {
            "payload" => msg.payload.to_string(),
            "flow_id" => flow_id.to_string(),
            "error" => msg.metadata.get("error").cloned().unwrap_or_default(),
            k if k.starts_with("meta.") => msg.metadata.get(&k[5..]).cloned().unwrap_or_default(),
            k => match get_json_path(&msg.payload, k.strip_prefix("payload.").unwrap_or(k)) {
                Ok(Value::String(s)) => s,
                Ok(Value::Null) | Err(_) => String::new(),
                Ok(v) => v.to_string(),
            },
        };
        out.push_str(&value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// What `enqueue_checked_write` would refuse without looking at the queue or
/// the live value: unknown tag, not writable, outside the write range.
async fn preview_write_policy(storage: &Storage, tag_id: i64, value: f64) -> Result<(), String> {
    let policy = storage
        .get_write_policy(tag_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Data source not found".to_string())?;
    if !policy.writable {
        return Err("Tag is not writable".into());
    }
    if !value.is_finite() {
        return Err("Invalid numeric value".into());
    }
    if let Some(min) = policy.write_min
        && value < min
    {
        return Err(format!("Value {value} is below minimum {min}"));
    }
    if let Some(max) = policy.write_max
        && value > max
    {
        return Err(format!("Value {value} is above maximum {max}"));
    }
    Ok(())
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn get_json_path(value: &Value, path: &str) -> Result<Value, String> {
    let mut current = value;
    for segment in path.split('.') {
        match current {
            Value::Object(map) => {
                current = map
                    .get(segment)
                    .ok_or_else(|| format!("Path segment '{}' not found", segment))?;
            }
            _ => {
                return Err(format!(
                    "Cannot resolve path segment '{}' in non-object",
                    segment
                ));
            }
        }
    }
    Ok(current.clone())
}

fn set_json_path(value: &mut Value, path: &str, new_value: Value) {
    let segments: Vec<&str> = path.split('.').collect();
    if segments.is_empty() {
        *value = new_value;
        return;
    }
    let mut current = value;
    for i in 0..segments.len() - 1 {
        let segment = segments[i];
        if !current.is_object() {
            *current = serde_json::json!({});
        }
        let map = current.as_object_mut().unwrap();
        if !map.contains_key(segment) {
            map.insert(segment.to_string(), serde_json::json!({}));
        }
        current = map.get_mut(segment).unwrap();
    }
    let last = segments.last().unwrap();
    if !current.is_object() {
        *current = serde_json::json!({});
    }
    current
        .as_object_mut()
        .unwrap()
        .insert(last.to_string(), new_value);
}

fn to_f64(value: &Value) -> Result<f64, String> {
    match value {
        Value::Number(n) => n.as_f64().ok_or_else(|| "Invalid number".to_string()),
        Value::String(s) => s
            .parse()
            .map_err(|_| format!("Cannot convert string '{}' to number", s)),
        _ => Err(format!("Cannot convert {:?} to number", value)),
    }
}

fn rule_matches(value: f64, rule: &ThresholdRule) -> bool {
    if let Some(gt) = rule.gt
        && value <= gt
    {
        return false;
    }
    if let Some(gte) = rule.gte
        && value < gte
    {
        return false;
    }
    if let Some(lt) = rule.lt
        && value >= lt
    {
        return false;
    }
    if let Some(lte) = rule.lte
        && value > lte
    {
        return false;
    }
    if let Some(eq) = rule.eq
        && (value - eq).abs() > f64::EPSILON
    {
        return false;
    }
    true
}

#[cfg(test)]
mod write_out_tests {
    use super::*;
    use crate::models::WritePolicyUpdate;

    async fn context_with_tag(writable: bool, write_max: Option<f64>) -> (NodeContext, i64) {
        let storage = Storage::new_in_memory().unwrap();
        storage.create_device("plc-1", "PLC 1", 0).await.unwrap();
        let id = storage
            .create_data_source(
                "setpoint", "setpoint", "", "", 0.0, 100.0, 0.0, 0.0, 0.0, 0, 2, "modbus", "{}",
                "plc-1", None, "line",
            )
            .await
            .unwrap();
        storage
            .update_write_policy(
                id,
                &WritePolicyUpdate {
                    writable: Some(writable),
                    write_min: None,
                    write_max: Some(write_max),
                    write_max_rate: None,
                    write_requires_reason: None,
                    write_deadband: None,
                },
            )
            .await
            .unwrap();
        let ctx = NodeContext {
            flow_id: "f1".into(),
            node_id: "n1".into(),
            state: crate::flow_engine::state::FlowState::new(),
            storage,
            cache: DeviceCache::new(),
            dry_run: false,
            smtp: None,
        };
        (ctx, id)
    }

    fn write_node(data_source_id: i64) -> NodeConfig {
        serde_json::from_value(serde_json::json!({
            "type": "write_out", "id": "w", "data_source_id": data_source_id,
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn write_out_obeys_the_tag_write_policy() {
        let msg = || Message::new(serde_json::json!({"v": 50.0}));

        let (ctx, id) = context_with_tag(false, None).await;
        let err = write_node(id).process(msg(), &ctx).await.unwrap_err();
        assert!(err.contains("not writable"), "{err}");
        assert!(ctx.storage.get_queued_commands().await.unwrap().is_empty());

        let (ctx, id) = context_with_tag(true, Some(10.0)).await;
        let err = write_node(id).process(msg(), &ctx).await.unwrap_err();
        assert!(err.contains("above maximum"), "{err}");

        let (ctx, id) = context_with_tag(true, Some(100.0)).await;
        write_node(id).process(msg(), &ctx).await.unwrap();
        let queued = ctx.storage.get_queued_commands().await.unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].created_by, "flow:f1");
    }
}
