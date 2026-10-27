#![allow(clippy::needless_range_loop)]
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::message::Message;
use super::state::FlowState;
use crate::cache::DeviceCache;
use crate::models::Command;
use crate::storage::Storage;
use rand::Rng;

fn default_flow_policy() -> String {
    "immediate".to_string()
}
fn default_flow_reason() -> String {
    "flow:unknown".to_string()
}

#[derive(Debug, Clone)]
pub enum NodeResult {
    Continue(Message),
    FilterReject,
}

#[derive(Clone)]
pub struct NodeContext {
    pub flow_id: String,
    pub node_id: String,
    pub state: FlowState,
    pub storage: Storage,
    pub cache: DeviceCache,
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
        name: String,
        unit: String,
        decimals: i64,
    },
    #[serde(rename = "http_out")]
    HttpOut {
        id: String,
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        timeout_ms: u64,
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
}

impl NodeConfig {
    pub async fn process(&self, mut msg: Message, ctx: &NodeContext) -> Result<NodeResult, String> {
        // Clone the shared handles up front so the `&NodeContext` borrow ends
        // before any `.await` (keeps this future `Send` for tokio::spawn).
        let storage = ctx.storage.clone();
        let cache = ctx.cache.clone();
        let flow_id = ctx.flow_id.clone();
        match self {
            NodeConfig::Filter { condition, .. } => {
                let result = crate::expression::evaluate_json(condition, &msg.payload)?;
                let truthy = is_truthy(&result);
                if truthy {
                    Ok(NodeResult::Continue(msg))
                } else {
                    Ok(NodeResult::FilterReject)
                }
            }
            NodeConfig::Transform { map, .. } => {
                let mut new_payload = serde_json::json!({});
                for (path, expr) in map {
                    let value = crate::expression::evaluate_json(expr, &msg.payload)?;
                    set_json_path(&mut new_payload, path, value);
                }
                msg.payload = new_payload;
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::Threshold { field, rules, .. } => {
                let field_value = get_json_path(&msg.payload, field)?;
                let field_num = to_f64(&field_value)?;
                for rule in rules {
                    if rule_matches(field_num, rule) {
                        for (path, value) in &rule.set {
                            set_json_path(&mut msg.payload, path, value.clone());
                        }
                        return Ok(NodeResult::Continue(msg));
                    }
                }
                Ok(NodeResult::Continue(msg))
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
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::Expression {
                output_field, expr, ..
            } => {
                let result = crate::expression::evaluate_json(expr, &msg.payload)?;
                set_json_path(&mut msg.payload, output_field, result);
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::DataSourceOut {
                value_field,
                source_key,
                ..
            } => {
                let value = get_json_path(&msg.payload, value_field)?;
                let numeric_value = to_f64(&value)?;
                let device_id = format!("flow_{}", flow_id);
                let mut metrics = HashMap::new();
                metrics.insert(source_key.clone(), numeric_value);
                cache.update(&device_id, metrics, false).await;
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::HttpOut {
                url,
                headers,
                timeout_ms,
                retry_count,
                ..
            } => {
                let client = reqwest::Client::new();
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
                        Err(e) => last_error = Some(e.to_string()),
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
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::AlarmOut {
                severity,
                source_key,
                message,
                ..
            } => {
                if let Err(e) = storage
                    .create_alarm(source_key, severity, 0.0, 0.0, message, 0.0)
                    .await
                {
                    tracing::warn!("AlarmOut node: failed to create alarm: {}", e);
                    return Err(format!("AlarmOut failed: {}", e));
                }
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::WriteOut {
                data_source_id,
                value_expr,
                policy,
                reason,
                ..
            } => {
                // 1. Resolve the write value.
                let value_str = if let Some(expr) = value_expr {
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

                // 2. Look up the target device and enqueue the command.
                let ds_id = *data_source_id;
                match storage.get_data_source_device_id(ds_id).await {
                    Ok(Some(target)) => {
                        let command_id: String = rand::rng()
                            .sample_iter(&rand::distr::Alphanumeric)
                            .take(21)
                            .map(char::from)
                            .collect::<String>()
                            .to_lowercase();
                        let now = crate::timeutil::now_secs();
                        let cmd = Command {
                            id: format!("cmd_{}", command_id),
                            data_source_id: ds_id,
                            device_id: target.device_id,
                            protocol: target.protocol,
                            value: value_str,
                            status: "pending".to_string(),
                            policy: policy.clone(),
                            scheduled_at: None,
                            retry_count: 0,
                            max_retries: 3,
                            error_message: None,
                            created_at: now,
                            sent_at: None,
                            acked_at: None,
                            resolved_at: None,
                            created_by: reason.clone(),
                            device_ref: target.device_ref,
                            readback_status: None,
                            readback_value: None,
                            readback_attempts: 0,
                            readback_ts: None,
                        };
                        if let Err(e) = storage.insert_command(&cmd).await {
                            return Err(format!("WriteOut failed to insert command: {}", e));
                        }
                    }
                    Ok(None) => {
                        return Err(format!(
                            "WriteOut: device not found for data_source_id {}",
                            ds_id
                        ));
                    }
                    Err(e) => {
                        return Err(format!("WriteOut: error looking up device: {}", e));
                    }
                }
                Ok(NodeResult::Continue(msg))
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
                        let result = crate::script_engine::execute_script(
                            &script.source,
                            &input,
                            script.cpu_limit_ms as u64,
                            script.memory_limit_bytes as u64,
                        );
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

                if result.success
                    && let Some(ref out) = result.output
                    && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(out)
                    && let serde_json::Value::Object(map) = parsed
                {
                    for (k, v) in map {
                        set_json_path(&mut msg.payload, &k, v);
                    }
                }
                Ok(NodeResult::Continue(msg))
            }
            NodeConfig::Dsl { source, .. } => {
                match crate::expression::run_dsl(source, &msg.payload) {
                    Ok(Some(v)) => {
                        msg.payload = v;
                        Ok(NodeResult::Continue(msg))
                    }
                    Ok(None) => Ok(NodeResult::FilterReject),
                    Err(_) => Ok(NodeResult::Continue(msg)),
                }
            }
        }
    }
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
