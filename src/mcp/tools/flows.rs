use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::flows;
use crate::flow_engine::node::NodeConfig;
use crate::flow_engine::trigger::TriggerConfig;
use crate::models::Flow;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_flows",
            title: "List flows",
            description: "List automation flows: id, name, enabled, the trigger and the node \
                pipeline (both as JSON text). A flow runs when its trigger fires and passes a \
                message through its nodes in order.",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: list_flows,
        },
        Tool {
            name: "create_flow",
            title: "Create flow",
            description: "Create an automation flow: a `trigger` plus an ordered pipeline of \
                `nodes`. The payload starts as the trigger's data (for a modbus trigger: \
                {device_id, metrics: {name: value}}) and each node reads and extends it. \
                Typical: trigger modbus device \"plts_1\" -> threshold node on field \"temp\" -> \
                alarm_out. The flow is created disabled unless `enabled` is true, so review it \
                and call start_flow. write_out, http_out and script nodes act on the plant or \
                the network: create them only on explicit request. Input is validated before \
                anything is stored.",
            access: Access::Write,
            destructive: false,
            input_schema: create_schema,
            call: create_flow,
        },
        Tool {
            name: "start_flow",
            title: "Start flow",
            description: "Enable a flow (by id from list_flows) so it runs on its trigger. The \
                state is stored and survives restarts.",
            access: Access::Write,
            destructive: false,
            input_schema: id_schema,
            call: start_flow,
        },
        Tool {
            name: "stop_flow",
            title: "Stop flow",
            description: "Disable a flow (by id from list_flows) so it no longer runs. The flow \
                is kept; start_flow re-enables it.",
            access: Access::Write,
            destructive: false,
            input_schema: id_schema,
            call: stop_flow,
        },
    ]
}

fn id_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"id": {"type": "string", "description": "Flow id from list_flows."}},
        "required": ["id"],
        "additionalProperties": false
    })
}

fn create_schema() -> Value {
    let node = |ty: &str, props: Value, required: &[&str]| {
        let mut properties = json!({
            "type": {"const": ty},
            "id": {"type": "string", "minLength": 1, "description": "Node id, unique inside the flow."}
        });
        if let (Some(base), Some(extra)) = (properties.as_object_mut(), props.as_object()) {
            base.extend(extra.clone());
        }
        let mut req = vec!["type", "id"];
        req.extend_from_slice(required);
        json!({"type": "object", "properties": properties, "required": req, "additionalProperties": false})
    };
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string", "minLength": 1, "description": "Flow id. Defaults to a slug of the name, made unique."},
            "name": {"type": "string", "minLength": 1},
            "enabled": {"type": "boolean", "default": false, "description": "Run the flow as soon as it is created."},
            "trigger": {
                "description": "What starts the flow.",
                "oneOf": [
                    {"type": "object", "properties": {
                        "type": {"const": "modbus"},
                        "device_id": {"type": "string", "description": "Device id, or \"*\" for any device."},
                        "metric": {"type": ["string", "null"], "description": "Only fire when this metric is present."},
                        "debounce_ms": {"type": "integer", "minimum": 0},
                        "on_change_only": {"type": "boolean"}
                    }, "required": ["type", "device_id"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "type": {"const": "timer"},
                        "interval_ms": {"type": "integer", "minimum": 1}
                    }, "required": ["type", "interval_ms"], "additionalProperties": false},
                    {"type": "object", "properties": {
                        "type": {"const": "mqtt"},
                        "topic": {"type": "string"},
                        "debounce_ms": {"type": "integer", "minimum": 0}
                    }, "required": ["type", "topic"], "additionalProperties": false}
                ]
            },
            "nodes": {
                "type": "array",
                "description": "Pipeline, run in order. Expressions use payload field names.",
                "items": {"oneOf": [
                    node("filter", json!({"condition": {"type": "string", "description": "Expression; the message continues only when it is true."}}), &["condition"]),
                    node("transform", json!({"map": {"type": "object", "additionalProperties": {"type": "string"}, "description": "output field -> expression"}}), &[]),
                    node("threshold", json!({
                        "field": {"type": "string"},
                        "rules": {"type": "array", "items": {"type": "object", "properties": {
                            "gt": {"type": "number"}, "gte": {"type": "number"}, "lt": {"type": "number"}, "lte": {"type": "number"}, "eq": {"type": "number"},
                            "set": {"type": "object", "description": "Fields set on the payload when the rule matches."}
                        }, "additionalProperties": false}}
                    }), &["field", "rules"]),
                    node("lookup", json!({
                        "field": {"type": "string"}, "output_field": {"type": "string"},
                        "table": {"type": "object"}, "default": {}
                    }), &["field", "output_field", "default"]),
                    node("expression", json!({"output_field": {"type": "string"}, "expr": {"type": "string"}}), &["output_field", "expr"]),
                    node("data_source_out", json!({
                        "value_field": {"type": "string"}, "source_key": {"type": "string"},
                        "name": {"type": "string"}, "unit": {"type": "string"}, "decimals": {"type": "integer"}
                    }), &["value_field", "source_key", "name", "unit", "decimals"]),
                    node("http_out", json!({
                        "url": {"type": "string"}, "headers": {"type": "object", "additionalProperties": {"type": "string"}},
                        "timeout_ms": {"type": "integer", "minimum": 1}, "retry_count": {"type": "integer", "minimum": 0}
                    }), &["url", "timeout_ms", "retry_count"]),
                    node("alarm_out", json!({
                        "severity": {"type": "string", "enum": ["critical", "warning", "info"]},
                        "source_key": {"type": "string"}, "message": {"type": "string"}
                    }), &["severity", "source_key", "message"]),
                    node("write_out", json!({
                        "data_source_id": {"type": "integer", "description": "Tag id to write (its write policy applies)."},
                        "value_expr": {"type": ["string", "null"]},
                        "policy": {"type": "string", "default": "immediate"},
                        "reason": {"type": "string"}
                    }), &["data_source_id"]),
                    node("script", json!({"script_id": {"type": "integer"}, "inputs": {"type": "array", "items": {"type": "string"}}}), &["script_id"]),
                    node("dsl", json!({"source": {"type": "string"}}), &[])
                ]}
            }
        },
        "required": ["name", "trigger", "nodes"],
        "additionalProperties": false
    })
}

fn list_flows(ctx: &Ctx, _args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        let out = from_handler(flows::list_flows(State(ctx.state.clone())).await).await?;
        let count = out["flows"].as_array().map_or(0, Vec::len);
        let mut out = out;
        out["count"] = json!(count);
        Ok(out)
    })
}

fn create_flow(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            id: Option<String>,
            name: String,
            #[serde(default)]
            enabled: bool,
            trigger: Value,
            nodes: Vec<Value>,
        }
        let args: Args = parse_args(args)?;
        let name = args.name.trim().to_string();
        if name.is_empty() {
            return Err(ToolError::new("name must not be empty"));
        }
        // Validate before storing: the create handler persists first and only
        // then parses what it was given.
        let trigger: TriggerConfig = serde_json::from_value(args.trigger)
            .map_err(|e| ToolError::new(format!("invalid trigger: {e}")))?;
        let nodes: Vec<NodeConfig> = serde_json::from_value(Value::Array(args.nodes))
            .map_err(|e| ToolError::new(format!("invalid nodes: {e}")))?;
        let mut seen = std::collections::HashSet::new();
        for node in &nodes {
            let id = node_id(node);
            if id.trim().is_empty() || !seen.insert(id.clone()) {
                return Err(ToolError::new(format!(
                    "node ids must be non-empty and unique (offending id: '{id}')"
                )));
            }
        }

        let id = match args.id.map(|i| i.trim().to_string()) {
            Some(id) if id.is_empty() => return Err(ToolError::new("id must not be empty")),
            Some(id) => {
                if ctx.state.storage.get_flow(&id).await?.is_some() {
                    return Err(ToolError::new(format!("flow '{id}' already exists")));
                }
                id
            }
            None => unique_id(ctx, &name).await?,
        };

        let now = crate::timeutil::now_secs();
        let flow = Flow {
            id: id.clone(),
            name,
            enabled: args.enabled,
            trigger_config: serde_json::to_string(&trigger)
                .map_err(|e| ToolError::new(e.to_string()))?,
            nodes: serde_json::to_string(&nodes).map_err(|e| ToolError::new(e.to_string()))?,
            created_at: now,
            updated_at: now,
        };
        from_handler(
            flows::create_flow(
                State(ctx.state.clone()),
                Extension(ctx.user.clone()),
                axum::Json(flow),
            )
            .await,
        )
        .await?;
        Ok(json!({"id": id, "enabled": args.enabled, "nodes": nodes.len()}))
    })
}

fn node_id(node: &NodeConfig) -> String {
    match node {
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
        | NodeConfig::Dsl { id, .. } => id.clone(),
    }
}

async fn unique_id(ctx: &Ctx, name: &str) -> Result<String, ToolError> {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let slug = slug.trim_matches('_').to_string();
    let base = if slug.is_empty() {
        format!("flow_{}", crate::timeutil::now_millis())
    } else {
        slug
    };
    let mut id = base.clone();
    let mut n = 1;
    while ctx.state.storage.get_flow(&id).await?.is_some() {
        id = format!("{base}_{n}");
        n += 1;
    }
    Ok(id)
}

fn start_flow(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move { set_enabled(ctx, args, true).await })
}

fn stop_flow(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move { set_enabled(ctx, args, false).await })
}

async fn set_enabled(ctx: &Ctx, args: Value, enabled: bool) -> super::ToolResult {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Args {
        id: String,
    }
    let Args { id } = parse_args(args)?;
    if ctx.state.storage.get_flow(&id).await?.is_none() {
        return Err(ToolError::new(format!("flow '{id}' not found")));
    }
    let auth = ctx.user.clone();
    let result = if enabled {
        flows::start_flow(State(ctx.state.clone()), Extension(auth), Path(id.clone())).await
    } else {
        flows::stop_flow(State(ctx.state.clone()), Extension(auth), Path(id.clone())).await
    };
    from_handler(result).await?;
    Ok(json!({"ok": true, "id": id, "enabled": enabled}))
}
