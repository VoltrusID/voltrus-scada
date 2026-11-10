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
Read tools are always available; write tools (create/delete/update/acknowledge/start/stop/write_tag) need an integration API key or an operator login, and the server may be set read-only. \
write_tag and deletions answer first with a proposal and a confirmation_token: show the summary to the user and call confirm_action only once they agree. Tool calls are rate limited per credential. \
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
        "tools/list" => result_response(id, tools_list(state).await),
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

/// Name of the tool that executes a proposed write.
pub const CONFIRM_TOOL: &str = "confirm_action";

async fn tools_list(state: &AppState) -> Value {
    let read_only_mode = crate::api::settings::mcp_read_only(state).await;
    let guard = &state.mcp_guard;
    let mut list: Vec<Value> = tools::registry()
        .iter()
        // In read-only mode the write tools are not offered at all, so a model
        // does not plan around them.
        .filter(|tool| !read_only_mode || tool.access.read_only())
        .map(|tool| {
            let read_only = tool.access.read_only();
            let mut schema = (tool.input_schema)();
            let mut description = tool.description.to_string();
            if guard.needs_confirmation(tool.name, tool.destructive, read_only) {
                if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
                    props.insert(
                        "confirmation_token".into(),
                        json!({
                            "type": "string",
                            "description": "Token from this tool's proposal; with it the call is carried out. Same as calling confirm_action."
                        }),
                    );
                }
                description.push_str(
                    " Two steps: the first call changes nothing and returns a proposal with a \
                     summary and a confirmation_token; show the summary to the user and, once they \
                     agree, call confirm_action with the token.",
                );
            }
            json!({
                "name": tool.name,
                "title": tool.title,
                "description": description,
                "inputSchema": schema,
                "annotations": {
                    "title": tool.title,
                    "readOnlyHint": read_only,
                    "destructiveHint": tool.destructive,
                    "openWorldHint": false,
                },
            })
        })
        .collect();
    let any_confirmable = tools::registry()
        .iter()
        .any(|t| guard.needs_confirmation(t.name, t.destructive, t.access.read_only()));
    if !read_only_mode && any_confirmable {
        list.push(json!({
            "name": CONFIRM_TOOL,
            "title": "Confirm a proposed action",
            "description": "Carry out an action another tool proposed (write_tag, deletions...). \
                Pass the confirmation_token from the proposal. Call it only after the user has \
                seen the proposal's summary and agreed. Tokens are single use and expire.",
            "inputSchema": {
                "type": "object",
                "properties": {"token": {"type": "string", "description": "confirmation_token from the proposal."}},
                "required": ["token"],
                "additionalProperties": false
            },
            "annotations": {
                "title": "Confirm a proposed action",
                "readOnlyHint": false,
                "destructiveHint": true,
                "openWorldHint": false,
            },
        }));
    }
    json!({"tools": list})
}

fn denied(state: &AppState, user: &AuthUser, name: &str, why: &str) {
    state.audit_logger.log(crate::security::AuditEvent {
        action: format!("mcp:{name}:denied"),
        username: user.username.clone(),
        resource: format!("mcp_tool:{name}"),
        details: Some(format!("role={}; {why}", user.role)),
    });
}

/// May this principal call `tool` at all (mode and role)? `Err` is the
/// JSON-RPC error to answer with.
fn authorize(
    state: &AppState,
    user: &AuthUser,
    id: &Value,
    tool: &tools::Tool,
    read_only_mode: bool,
) -> Result<(), Value> {
    let name = tool.name;
    if read_only_mode && !tool.access.read_only() {
        denied(state, user, name, "read-only mode");
        return Err(error_response(
            id.clone(),
            FORBIDDEN,
            format!(
                "tool '{name}' changes the system and this MCP server is in read-only mode \
                 (mcp.read_only or the admin setting); only read tools are available."
            ),
        ));
    }
    if tool.access.admin_only() && user.role != "admin" {
        denied(state, user, name, "admin only");
        return Err(error_response(
            id.clone(),
            FORBIDDEN,
            format!(
                "tool '{name}' reads or replaces the whole configuration and is limited to a \
                 signed-in admin; this credential has role '{}'. API and integration keys cannot \
                 call it.",
                user.role
            ),
        ));
    }
    if tool.access == Access::Write && !role_allows_operational_writes(&user.role) {
        denied(state, user, name, "role");
        return Err(error_response(
            id.clone(),
            FORBIDDEN,
            format!(
                "tool '{name}' changes the system and needs the admin, demo or integration role; \
                 this credential has role '{}'. Use an integration API key.",
                user.role
            ),
        ));
    }
    Ok(())
}

/// A refusal the model can read and act on, as a tool result.
fn tool_error(id: Value, text: impl Into<String>) -> Value {
    result_response(
        id,
        json!({"content": [{"type": "text", "text": text.into()}], "isError": true}),
    )
}

fn rate_limited(state: &AppState, user: &AuthUser, id: Value, write: bool) -> Option<Value> {
    let wait = state.mcp_guard.check_rate(&user.username, write).err()?;
    let config = state.mcp_guard.config();
    Some(tool_error(
        id,
        format!(
            "rate limit reached for this credential ({} tool calls per minute, of which {} \
             writes); retry in {wait} s.",
            config.calls_per_minute, config.writes_per_minute
        ),
    ))
}

async fn tools_call(state: &AppState, user: &AuthUser, id: Value, params: Value) -> Value {
    let Some(name) = params.get("name").and_then(Value::as_str) else {
        return error_response(id, INVALID_PARAMS, "params.name (tool name) is required");
    };
    let mut arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => {
            return error_response(id, INVALID_PARAMS, "params.arguments must be an object");
        }
    };
    let read_only_mode = crate::api::settings::mcp_read_only(state).await;

    if name == CONFIRM_TOOL {
        return confirm(state, user, id, arguments, read_only_mode).await;
    }

    let Some(tool) = tools::find(name) else {
        return error_response(id, INVALID_PARAMS, format!("unknown tool: {name}"));
    };
    if let Err(e) = authorize(state, user, &id, tool, read_only_mode) {
        return e;
    }

    let read_only = tool.access.read_only();
    let token = arguments
        .as_object_mut()
        .and_then(|o| o.remove("confirmation_token"));
    if state
        .mcp_guard
        .needs_confirmation(tool.name, tool.destructive, read_only)
    {
        let Some(token) = token else {
            // The proposal changes nothing: it counts as a call, not a write.
            if let Some(limited) = rate_limited(state, user, id.clone(), false) {
                return limited;
            }
            return propose(state, user, id, tool, arguments).await;
        };
        let Some(token) = token.as_str() else {
            return tool_error(id, "confirmation_token must be a string");
        };
        let pending = match state.mcp_guard.take(token, &user.username) {
            Ok(p) => p,
            Err(e) => return tool_error(id, e.to_string()),
        };
        if pending.tool != tool.name || pending.arguments != arguments {
            return tool_error(
                id,
                "this confirmation token was issued for a different call; repeat the proposed \
                 call exactly, or use confirm_action",
            );
        }
    } else if token.is_some() {
        return tool_error(id, format!("tool '{name}' takes no confirmation_token"));
    }

    if let Some(limited) = rate_limited(state, user, id.clone(), !read_only) {
        return limited;
    }
    execute(state, user, id, tool, arguments).await
}

/// `confirm_action`: carry out a proposed call.
async fn confirm(
    state: &AppState,
    user: &AuthUser,
    id: Value,
    arguments: Value,
    read_only_mode: bool,
) -> Value {
    let Some(token) = arguments.get("token").and_then(Value::as_str) else {
        return error_response(id, INVALID_PARAMS, "arguments.token is required");
    };
    if read_only_mode {
        denied(state, user, CONFIRM_TOOL, "read-only mode");
        return error_response(
            id,
            FORBIDDEN,
            "this MCP server is in read-only mode (mcp.read_only or the admin setting); nothing \
             can be confirmed.",
        );
    }
    let pending = match state.mcp_guard.take(token, &user.username) {
        Ok(p) => p,
        Err(e) => return tool_error(id, e.to_string()),
    };
    let Some(tool) = tools::find(&pending.tool) else {
        return tool_error(id, format!("unknown tool: {}", pending.tool));
    };
    // The role is checked again: it may have been taken away since the proposal.
    if let Err(e) = authorize(state, user, &id, tool, read_only_mode) {
        return e;
    }
    if let Some(limited) = rate_limited(state, user, id.clone(), true) {
        return limited;
    }
    execute(state, user, id, tool, pending.arguments).await
}

/// One line per argument, values as JSON, long values cut.
fn summarize_arguments(arguments: &Value) -> String {
    let Some(obj) = arguments.as_object() else {
        return String::new();
    };
    obj.iter()
        .map(|(k, v)| {
            let mut text = v.to_string();
            if text.chars().count() > 120 {
                text = text.chars().take(120).collect::<String>() + "...";
            }
            format!("{k} = {text}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn propose(
    state: &AppState,
    user: &AuthUser,
    id: Value,
    tool: &tools::Tool,
    arguments: Value,
) -> Value {
    let mut summary = format!("{}: {}", tool.title, summarize_arguments(&arguments));
    // Name the tag for a write, so the person confirming sees what moves.
    if tool.name == "write_tag"
        && let Some(tag_id) = arguments.get("tag_id").and_then(Value::as_i64)
        && let Ok(Some(tag)) = state.storage.get_data_source_by_id(tag_id).await
    {
        summary.push_str(&format!(" (tag \"{}\", key {})", tag.name, tag.key));
    }
    if tool.destructive {
        summary.push_str(". This deletes or overwrites existing configuration");
    }
    let ttl = state.mcp_guard.config().confirmation_ttl_secs;
    let token = state
        .mcp_guard
        .propose(&user.username, tool.name, arguments.clone());
    state.audit_logger.log(crate::security::AuditEvent {
        action: format!("mcp:{}:proposed", tool.name),
        username: user.username.clone(),
        resource: format!("mcp_tool:{}", tool.name),
        details: Some(summary.chars().take(300).collect()),
    });
    let text = format!(
        "Confirmation required, nothing has been changed yet. Proposed: {summary}. Show this to \
         the user; if they agree, call {CONFIRM_TOOL} with {{\"token\": \"{token}\"}} within \
         {ttl} seconds."
    );
    result_response(
        id,
        json!({
            "content": [{"type": "text", "text": text}],
            "structuredContent": {
                "status": "confirmation_required",
                "tool": tool.name,
                "arguments": arguments,
                "summary": summary,
                "confirmation_token": token,
                "expires_in_secs": ttl,
            },
            "isError": false,
        }),
    )
}

async fn execute(
    state: &AppState,
    user: &AuthUser,
    id: Value,
    tool: &tools::Tool,
    arguments: Value,
) -> Value {
    let name = tool.name;
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
