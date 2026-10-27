//! JSON-RPC 2.0 message handling for the MCP methods the server implements:
//! `initialize`, `ping`, `tools/list`, `tools/call`, and the notifications a
//! client sends after the handshake.

use serde_json::{Value, json};

use super::tools::{self, Access, Ctx};
use crate::api::{AppState, AuthUser, role_allows_operational_writes};

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// The authenticated principal may not call this tool (server-defined code).
pub const FORBIDDEN: i64 = -32003;

/// Newest revision first. A client asking for a revision on this list gets it
/// back; anything else is answered with the newest one, and the client decides
/// whether it can continue.
const SUPPORTED_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Voltrus SCADA: industrial monitoring server. \
Tags (data sources) are the points that carry values; devices (Modbus) poll registers; \
a tag with source_type \"modbus\", device_id = the device id and key = the register name shows that register's live value. \
Read tools are always available; write tools (create/delete/update/acknowledge/start/stop/write_tag) need an integration API key or an operator login. \
read_live_values gives current values, read_history gives trends. write_tag queues a command and honours the tag's write policy. \
Device templates describe known hardware (registers or objects): list_device_templates finds one, instantiate_device_template creates the device. \
import_catalog_template and the project_* tools (YAML export, import plan, apply) are limited to a signed-in admin; API and integration keys are refused.";

pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

fn result_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// Handle one HTTP body: a single message, or a batch (revision 2025-03-26
/// allows them). `None` means nothing to send back (notification or response).
pub async fn handle_message(state: &AppState, user: &AuthUser, message: Value) -> Option<Value> {
    match message {
        Value::Array(items) => {
            if items.is_empty() {
                return Some(error_response(Value::Null, INVALID_REQUEST, "empty batch"));
            }
            let mut replies = Vec::new();
            for item in items {
                if let Some(reply) = handle_single(state, user, item).await {
                    replies.push(reply);
                }
            }
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        single => handle_single(state, user, single).await,
    }
}

async fn handle_single(state: &AppState, user: &AuthUser, message: Value) -> Option<Value> {
    let Some(obj) = message.as_object() else {
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            "a JSON-RPC message must be an object",
        ));
    };
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(
            obj.get("id").cloned().unwrap_or(Value::Null),
            INVALID_REQUEST,
            "jsonrpc must be \"2.0\"",
        ));
    }
    let id = obj.get("id").cloned();
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A client response (result/error) to a server request: the server
        // sends none, so there is nothing to do with it.
        if obj.contains_key("result") || obj.contains_key("error") {
            return None;
        }
        return Some(error_response(
            id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "missing method",
        ));
    };
    let params = obj.get("params").cloned().unwrap_or(Value::Null);

    // No id: a notification. `notifications/initialized`, `notifications/cancelled`
    // and the like need no action in a stateless server.
    let id = id?;
    if !matches!(id, Value::String(_) | Value::Number(_)) {
        return Some(error_response(
            Value::Null,
            INVALID_REQUEST,
            "id must be a string or a number",
        ));
    }

    Some(match method {
        "initialize" => result_response(id, initialize(&params)),
        "ping" => result_response(id, json!({})),
        "tools/list" => result_response(id, tools_list()),
        "tools/call" => tools_call(state, user, id, params).await,
        other => error_response(id, METHOD_NOT_FOUND, format!("method not found: {other}")),
    })
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|v| SUPPORTED_VERSIONS.contains(v))
        .unwrap_or(SUPPORTED_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {
            "name": "voltrus-scada",
            "title": "Voltrus SCADA",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

fn tools_list() -> Value {
    let list: Vec<Value> = tools::registry()
        .iter()
        .map(|tool| {
            let read_only = tool.access.read_only();
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": tool.description,
                "inputSchema": (tool.input_schema)(),
                "annotations": {
                    "title": tool.title,
                    "readOnlyHint": read_only,
                    "destructiveHint": tool.destructive,
                    "openWorldHint": false,
                },
            })
        })
        .collect();
    json!({"tools": list})
}

async fn tools_call(state: &AppState, user: &AuthUser, id: Value, params: Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, INVALID_PARAMS, "params.name (tool name) is required");
    };
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => {
            return error_response(id, INVALID_PARAMS, "params.arguments must be an object");
        }
    };
    let Some(tool) = tools::find(name) else {
        return error_response(id, INVALID_PARAMS, format!("unknown tool: {name}"));
    };

    if tool.access.admin_only() && user.role != "admin" {
        state.audit_logger.log(crate::security::AuditEvent {
            action: format!("mcp:{name}:denied"),
            username: user.username.clone(),
            resource: format!("mcp_tool:{name}"),
            details: Some(format!("role={}", user.role)),
        });
        return error_response(
            id,
            FORBIDDEN,
            format!(
                "tool '{name}' reads or replaces the whole configuration and is limited to a \
                 signed-in admin; this credential has role '{}'. API and integration keys cannot \
                 call it.",
                user.role
            ),
        );
    }
    if tool.access == Access::Write && !role_allows_operational_writes(&user.role) {
        state.audit_logger.log(crate::security::AuditEvent {
            action: format!("mcp:{name}:denied"),
            username: user.username.clone(),
            resource: format!("mcp_tool:{name}"),
            details: Some(format!("role={}", user.role)),
        });
        return error_response(
            id,
            FORBIDDEN,
            format!(
                "tool '{name}' changes the system and needs the admin, demo or integration role; \
                 this credential has role '{}'. Use an integration API key.",
                user.role
            ),
        );
    }

    let ctx = Ctx {
        state: state.clone(),
        user: user.clone(),
    };
    let audit_keys: Vec<String> = arguments
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let outcome = (tool.call)(&ctx, arguments).await;

    if tool.access.audited() {
        let (status, detail) = match &outcome {
            Ok(_) => ("ok", String::new()),
            Err(e) => (
                "error",
                format!("; error={}", e.0.chars().take(200).collect::<String>()),
            ),
        };
        state.audit_logger.log(crate::security::AuditEvent {
            action: format!("mcp:{name}"),
            username: user.username.clone(),
            resource: format!("mcp_tool:{name}"),
            details: Some(format!(
                "outcome={status}; args={}{detail}",
                audit_keys.join(",")
            )),
        });
    }

    match outcome {
        Ok(value) => {
            let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
            result_response(
                id,
                json!({
                    "content": [{"type": "text", "text": text}],
                    "structuredContent": value,
                    "isError": false,
                }),
            )
        }
        Err(e) => result_response(
            id,
            json!({
                "content": [{"type": "text", "text": e.0}],
                "isError": true,
            }),
        ),
    }
}
