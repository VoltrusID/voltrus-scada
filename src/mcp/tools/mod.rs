//! The tool registry. Each submodule returns its tools from `tools()`; to add
//! a tool, add a module (or a `Tool` in one) and list it in [`registry`].
//!
//! A tool is a name, an LLM-facing description, a JSON Schema for its input and
//! an async function. Tools call the same axum handlers the REST routes use
//! (see [`from_handler`]), so validation, write policy and side effects (poller
//! start, asset sync, flow engine reload, ...) are the REST ones, not copies.

use std::future::Future;
use std::pin::Pin;
use std::sync::LazyLock;

use axum::response::IntoResponse;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::api::{AppError, AppState, AuthUser};

mod alarms;
mod commands;
mod data;
mod devices;
mod flows;
mod project;
mod screens;
mod system;
mod tags;
mod templates;

/// Which principals may call a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Any authenticated principal, including a read-only `api` key.
    Read,
    /// Principals the REST operational write routes admit: admin, demo,
    /// integration. Every call is audit-logged.
    Write,
    /// Admin sessions only, for tools that expose the whole configuration but
    /// change nothing. API and integration keys are refused. Audit-logged.
    AdminRead,
    /// Admin sessions only, for tools that import or replace configuration.
    /// API and integration keys are refused. Audit-logged.
    AdminWrite,
}

impl Access {
    /// The tool changes no state (MCP `readOnlyHint`).
    pub fn read_only(self) -> bool {
        matches!(self, Access::Read | Access::AdminRead)
    }

    /// Only the admin role may call the tool.
    pub fn admin_only(self) -> bool {
        matches!(self, Access::AdminRead | Access::AdminWrite)
    }

    /// Every call is written to the audit log as `mcp:<tool>`.
    pub fn audited(self) -> bool {
        self != Access::Read
    }
}

/// Who is calling, and the server they are calling.
pub struct Ctx {
    pub state: AppState,
    pub user: AuthUser,
}

/// A tool failure the model can read and act on (returned as `isError: true`).
#[derive(Debug)]
pub struct ToolError(pub String);

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<anyhow::Error> for ToolError {
    fn from(e: anyhow::Error) -> Self {
        Self(format!("{e:#}"))
    }
}

impl From<AppError> for ToolError {
    fn from(e: AppError) -> Self {
        e.0.into()
    }
}

/// Tool output: a JSON object (lists are wrapped, e.g. `{"devices": [...]}`).
pub type ToolResult = Result<Value, ToolError>;
pub type ToolFuture<'a> = Pin<Box<dyn Future<Output = ToolResult> + Send + 'a>>;

pub struct Tool {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub access: Access,
    /// Deletes or overwrites existing configuration (MCP `destructiveHint`).
    pub destructive: bool,
    pub input_schema: fn() -> Value,
    pub call: for<'a> fn(&'a Ctx, Value) -> ToolFuture<'a>,
}

static REGISTRY: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    let mut tools = Vec::new();
    tools.extend(system::tools());
    tools.extend(devices::tools());
    tools.extend(tags::tools());
    tools.extend(data::tools());
    tools.extend(alarms::tools());
    tools.extend(screens::tools());
    tools.extend(flows::tools());
    tools.extend(commands::tools());
    tools.extend(templates::tools());
    tools.extend(project::tools());
    tools
});

pub fn registry() -> &'static [Tool] {
    &REGISTRY
}

pub fn find(name: &str) -> Option<&'static Tool> {
    REGISTRY.iter().find(|t| t.name == name)
}

/// Deserialize tool arguments, reporting a mistake in terms the model can fix.
pub fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|e| ToolError::new(format!("invalid arguments: {e}")))
}

/// Largest handler response a tool will read back.
const MAX_HANDLER_BODY: usize = 8 * 1024 * 1024;

/// Turn the result of a REST handler call into tool output: a success status
/// gives the handler's JSON body (`{"ok": true}` when it has none), anything
/// else a [`ToolError`] carrying the handler's `error` message and status.
pub async fn from_handler<R: IntoResponse>(result: Result<R, AppError>) -> ToolResult {
    let response = match result {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_HANDLER_BODY)
        .await
        .map_err(|e| ToolError::new(format!("could not read the result: {e}")))?;
    if bytes.is_empty() && !status.is_success() {
        return Err(ToolError::new(format!(
            "{} (HTTP {})",
            status.canonical_reason().unwrap_or("request refused"),
            status.as_u16()
        )));
    }
    let body: Value = if bytes.is_empty() {
        json!({"ok": true})
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    if status.is_success() {
        return Ok(body);
    }
    let message = body
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| body.as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string());
    Err(ToolError::new(format!(
        "{message} (HTTP {})",
        status.as_u16()
    )))
}

/// Ensure an object result (some handlers return bare values).
pub fn into_object(key: &str, value: Value) -> Value {
    if value.is_object() {
        value
    } else {
        json!({ key: value })
    }
}
