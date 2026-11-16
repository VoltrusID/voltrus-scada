use axum::extract::{Extension, Path, State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Access, Ctx, Tool, ToolError, ToolFuture, from_handler, parse_args};
use crate::api::flows;
use crate::flow_engine::graph::FlowGraph;
use crate::flow_engine::trigger::TriggerConfig;

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list_flows",
            title: "List flows",
            description: "List automation flows: id, name, enabled, the trigger and the node \
                graph (both as JSON text). The graph is {version: 2, nodes, edges}: a message \
                from the trigger follows the edges from \"trigger\", and each node sends its \
                result out of one of its output ports.",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: list_flows,
        },
        Tool {
            name: "create_flow",
            title: "Create flow",
            description: "Create an automation flow: a `trigger` plus `nodes`, either an \
                ordered pipeline (array, run in order) or a graph {version: 2, nodes: [...], \
                edges: [{from, port, to}]} where \"trigger\" is the source and ports are the \
                output indexes flow_node_types lists (filter: 0 pass, 1 reject). The payload \
                starts as the trigger's data (for a modbus trigger: {device_id, metrics: {name: \
                value}}) and each node reads and extends it. Typical: trigger modbus device \
                \"plts_1\" -> threshold node on field \"temp\" -> alarm_out. The flow is created \
                disabled unless `enabled` is true, so review it (test_flow) and call start_flow. \
                write_out, http_out, notify and script nodes act on the plant or the network: \
                create them only on explicit request. Input is validated before anything is \
                stored.",
            access: Access::Write,
            destructive: false,
            input_schema: create_schema,
            call: create_flow,
        },
        Tool {
            name: "flow_node_types",
            title: "Flow node types",
            description: "The flow node and trigger catalogue: every type with its config \
                fields, output ports (in port order) and whether it acts outside the flow \
                (side_effect).",
            access: Access::Read,
            destructive: false,
            input_schema: || json!({"type": "object", "properties": {}, "additionalProperties": false}),
            call: node_types,
        },
        Tool {
            name: "test_flow",
            title: "Test flow",
            description: "Dry-run a flow definition (the same trigger and nodes create_flow \
                takes) on a sample payload, without storing it. Returns the per-node trace \
                (input, output, port, error) and the nodes the message ended at. Nothing acts \
                outside the flow: write_out, http_out, notify, alarm_out and data_source_out \
                only report what they would have done.",
            access: Access::Write,
            destructive: false,
            input_schema: test_schema,
            call: test_flow,
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
            "nodes": {"oneOf": [
                {"type": "object", "description": "Graph (v2). Node objects take the fields of their type (see flow_node_types) plus optional x, y, name.",
                 "properties": {
                    "version": {"const": 2},
                    "nodes": {"type": "array", "items": {"type": "object", "properties": {
                        "id": {"type": "string"}, "type": {"type": "string"}
                    }, "required": ["id", "type"]}},
                    "edges": {"type": "array", "items": {"type": "object", "properties": {
                        "from": {"type": "string", "description": "Node id, or \"trigger\"."},
                        "port": {"type": "integer", "minimum": 0, "default": 0},
                        "to": {"type": "string"}
                    }, "required": ["from", "to"], "additionalProperties": false}}
                 }, "required": ["nodes", "edges"]},
                {
                "type": "array",
                "description": "Pipeline (v1), run in order. Expressions use payload field names.",
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
            ]}
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
            nodes: Value,
        }
        let args: Args = parse_args(args)?;
        let name = args.name.trim().to_string();
        if name.is_empty() {
            return Err(ToolError::new("name must not be empty"));
        }
        // Validate before picking an id, so a broken flow reports why.
        TriggerConfig::from_value(args.trigger.clone())
            .map_err(|e| ToolError::new(format!("invalid trigger: {e}")))?;
        let graph = FlowGraph::from_value(args.nodes.clone())
            .map_err(|e| ToolError::new(format!("invalid nodes: {e}")))?;

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

        let flow = flows::FlowInput {
            id: id.clone(),
            name,
            enabled: args.enabled,
            trigger_config: args.trigger,
            nodes: args.nodes,
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
        Ok(json!({
            "id": id,
            "enabled": args.enabled,
            "nodes": graph.nodes.len(),
            "edges": graph.edges.len(),
        }))
    })
}

fn node_types(_ctx: &Ctx, _args: Value) -> ToolFuture<'_> {
    Box::pin(async move { Ok(crate::flow_engine::catalogue::catalogue()) })
}

fn test_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "trigger": {"type": "object", "description": "As in create_flow."},
            "nodes": {"description": "As in create_flow: a pipeline array or a {version: 2, nodes, edges} graph."},
            "payload": {"type": "object", "description": "Message payload the trigger would produce.", "default": {}},
            "metadata": {"type": "object", "additionalProperties": {"type": "string"}}
        },
        "required": ["trigger", "nodes"],
        "additionalProperties": false
    })
}

fn test_flow(ctx: &Ctx, args: Value) -> ToolFuture<'_> {
    Box::pin(async move {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            trigger: Value,
            nodes: Value,
            #[serde(default)]
            payload: Value,
            #[serde(default)]
            metadata: std::collections::HashMap<String, String>,
        }
        let args: Args = parse_args(args)?;
        let req = flows::TestFlowRequest {
            flow: flows::TestFlowDefinition {
                id: None,
                trigger_config: args.trigger,
                nodes: args.nodes,
            },
            input: flows::RunInput {
                payload: args.payload,
                metadata: args.metadata,
            },
        };
        from_handler(flows::test_flow_definition(State(ctx.state.clone()), axum::Json(req)).await)
            .await
    })
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
