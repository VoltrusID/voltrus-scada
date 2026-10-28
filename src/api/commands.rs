use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use rand::Rng;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::models::*;

// ── Write Tag ──

/// POST /api/v1/tags/{id}/write
#[utoipa::path(
    post,
    path = "/api/v1/tags/{id}/write",
    params(
        ("id" = i64, Path, description = "Tag ID")
    ),
    request_body = WriteRequest,
    responses(
        (status = 202, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 429),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn write_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(tag_id): Path<i64>,
    Json(body): Json<WriteRequest>,
) -> Result<impl IntoResponse, AppError> {
    let Some(value) = body.value.as_f64() else {
        return Ok(WriteError::Invalid("Invalid numeric value".into()).into_response());
    };
    let submission = TagWriteSubmission {
        tag_id,
        value,
        policy: body.policy,
        scheduled_at: body.scheduled_at,
        reason: body.reason,
        command_id: None,
    };
    match submit_tag_write(&state, &user.username, submission).await {
        Ok(cmd) => Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "command_id": cmd.id,
                "status": "pending",
                "data_source_id": tag_id,
                "value": value,
                "created_at": cmd.created_at,
            })),
        )
            .into_response()),
        Err(WriteError::Internal(e)) => Err(e.into()),
        Err(refusal) => Ok(refusal.into_response()),
    }
}

/// One tag write on its way into the command queue, from any front end (REST,
/// WebSocket).
pub struct TagWriteSubmission {
    pub tag_id: i64,
    pub value: f64,
    pub policy: String,
    pub scheduled_at: Option<i64>,
    pub reason: Option<String>,
    /// Caller-chosen command id (WebSocket clients correlate acknowledgements
    /// by it). `None` generates one.
    pub command_id: Option<String>,
}

/// Why a write was refused before it reached the queue.
#[derive(Debug)]
pub enum WriteError {
    /// The data source does not exist.
    NotFound(String),
    /// The tag is not writable (403).
    NotWritable(String),
    /// The request or value violates the tag's write policy (400).
    Invalid(String),
    /// The tag's write rate limit is used up (429).
    RateLimited(String),
    Internal(anyhow::Error),
}

impl WriteError {
    /// The text shown to the caller. Internal errors are logged, not exposed.
    pub fn message(&self) -> String {
        match self {
            WriteError::NotFound(m)
            | WriteError::NotWritable(m)
            | WriteError::Invalid(m)
            | WriteError::RateLimited(m) => m.clone(),
            WriteError::Internal(e) => {
                tracing::error!("Tag write failed: {e:#}");
                "Internal error".to_string()
            }
        }
    }
}

impl From<anyhow::Error> for WriteError {
    fn from(e: anyhow::Error) -> Self {
        WriteError::Internal(e)
    }
}

impl IntoResponse for WriteError {
    fn into_response(self) -> axum::response::Response {
        let status = match &self {
            WriteError::NotFound(_) => StatusCode::NOT_FOUND,
            WriteError::NotWritable(_) => StatusCode::FORBIDDEN,
            WriteError::Invalid(_) => StatusCode::BAD_REQUEST,
            WriteError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            WriteError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error": self.message()}))).into_response()
    }
}

/// Longest command id a client may choose.
const MAX_CLIENT_COMMAND_ID_LEN: usize = 64;

/// Check a write against the tag's policy (writable, mandatory reason, range,
/// deadband, rate limit) and put it into the command queue, with an audit entry
/// naming `username`. Every write entry point goes through here so none can skip
/// the policy.
pub async fn submit_tag_write(
    state: &AppState,
    username: &str,
    req: TagWriteSubmission,
) -> Result<Command, WriteError> {
    let tag_id = req.tag_id;
    let value = req.value;

    // 1. Get write policy — also validates that the data source exists
    let policy = state
        .storage
        .get_write_policy(tag_id)
        .await?
        .ok_or_else(|| WriteError::NotFound("Data source not found".into()))?;

    // 2. Check writable
    if !policy.writable {
        return Err(WriteError::NotWritable("Tag is not writable".into()));
    }

    // 3. Check write_requires_reason
    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    if policy.write_requires_reason && reason.is_none() {
        return Err(WriteError::Invalid(
            "Reason is required for writes on this tag".into(),
        ));
    }

    // 4. A NaN or infinite value passes every comparison below
    if !value.is_finite() {
        return Err(WriteError::Invalid("Invalid numeric value".into()));
    }

    // 5. Check value within write_min / write_max range
    if let Some(min) = policy.write_min
        && value < min
    {
        return Err(WriteError::Invalid(format!(
            "Value {value} is below minimum {min}"
        )));
    }
    if let Some(max) = policy.write_max
        && value > max
    {
        return Err(WriteError::Invalid(format!(
            "Value {value} is above maximum {max}"
        )));
    }

    // 6. Check deadband against current cached value
    if let Some(deadband) = policy.write_deadband
        && deadband > 0.0
    {
        // The cache holds a metric under the bare register name, scoped by device
        // (the data source key may be device-qualified, `plts_1.voltage`).
        let source = state.storage.get_data_source_by_id(tag_id).await?;
        if let Some(source) = source
            && let Some(current) = state
                .cache
                .get_device_metric(
                    &source.device_id,
                    super::data::metric_name(&source.key, &source.device_id),
                )
                .await
            && (value - current).abs() <= deadband
        {
            return Err(WriteError::Invalid(format!(
                "Value {value} is within deadband {deadband} of current value {current}"
            )));
        }
    }

    // 7. Check rate limit
    if let Some(max_rate) = policy.write_max_rate
        && max_rate > 0
    {
        let recent = state.storage.count_recent_writes(tag_id, 60).await?;
        if recent >= max_rate {
            return Err(WriteError::RateLimited(format!(
                "Rate limit exceeded: {recent} writes in the last 60s (max {max_rate})"
            )));
        }
    }

    // 8. Get device info
    let target = state
        .storage
        .get_data_source_device_id(tag_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Device not found for data source"))?;

    // 9. Create and insert Command
    let command_id = match req.command_id {
        Some(id) => {
            let valid = !id.is_empty()
                && id.len() <= MAX_CLIENT_COMMAND_ID_LEN
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
            if !valid {
                return Err(WriteError::Invalid(
                    "Command id must be 1-64 characters of letters, digits, '-' or '_'".into(),
                ));
            }
            if state.storage.get_command(&id).await?.is_some() {
                return Err(WriteError::Invalid("Command id is already in use".into()));
            }
            id
        }
        None => rand::rng()
            .sample_iter(&rand::distr::Alphanumeric)
            .take(21)
            .map(char::from)
            .collect::<String>()
            .to_lowercase(),
    };
    let now = crate::timeutil::now_secs();
    let cmd = Command {
        id: command_id,
        data_source_id: tag_id,
        device_id: target.device_id,
        protocol: target.protocol,
        value: value.to_string(),
        status: CommandStatus::Pending.as_str().to_string(),
        policy: req.policy,
        scheduled_at: req.scheduled_at,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: now,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: username.to_string(),
        device_ref: target.device_ref,
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };

    state.storage.insert_command_audited(&cmd, reason).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "tag_write".to_string(),
        username: username.to_string(),
        resource: format!("data_source:{tag_id}"),
        details: Some(match reason {
            Some(r) => format!("command={} value={value} reason={r}", cmd.id),
            None => format!("command={} value={value}", cmd.id),
        }),
    });

    Ok(cmd)
}

// ── Get Command Status ──

/// GET /api/v1/commands/{command_id}
#[utoipa::path(
    get,
    path = "/api/v1/commands/{command_id}",
    params(
        ("command_id" = String, Path, description = "Command ID")
    ),
    responses(
        (status = 200, body = Command),
        (status = 404),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_command_status(
    State(state): State<AppState>,
    Path(command_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let cmd = state
        .storage
        .get_command(&command_id)
        .await?
        .ok_or_else(|| AppError::not_found("Command not found"))?;
    Ok(Json(cmd))
}

// ── List Commands ──

/// GET /api/v1/commands
#[utoipa::path(
    get,
    path = "/api/v1/commands",
    responses(
        (status = 200, body = Vec<Command>),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_commands(
    State(state): State<AppState>,
    Query(query): Query<CommandQuery>,
) -> Result<impl IntoResponse, AppError> {
    let commands = state.storage.list_commands(&query).await?;
    Ok(Json(commands))
}

// ── Cancel Command ──

/// POST /api/v1/commands/{command_id}/cancel
#[utoipa::path(
    post,
    path = "/api/v1/commands/{command_id}/cancel",
    params(
        ("command_id" = String, Path, description = "Command ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 404),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn cancel_command(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(command_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let cmd = state
        .storage
        .get_command(&command_id)
        .await?
        .ok_or_else(|| AppError::not_found("Command not found"))?;

    // Only cancel if status is pending or queued
    if cmd.status != CommandStatus::Pending.as_str() && cmd.status != CommandStatus::Queued.as_str()
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Cannot cancel command with status '{}'", cmd.status)})),
        )
            .into_response());
    }

    state
        .storage
        .update_command_status(
            &command_id,
            CommandStatus::Cancelled.as_str(),
            None,
            &user.username,
            Some("Cancelled by user"),
        )
        .await?;

    Ok(Json(json!({"ok": true, "command_id": command_id, "status": "cancelled"})).into_response())
}

// ── Get Command Audit ──

/// GET /api/v1/commands/{command_id}/audit
#[utoipa::path(
    get,
    path = "/api/v1/commands/{command_id}/audit",
    params(
        ("command_id" = String, Path, description = "Command ID")
    ),
    responses(
        (status = 200, body = Vec<CommandAuditEntry>),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_command_audit(
    State(state): State<AppState>,
    Path(command_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let entries = state.storage.get_command_audit(&command_id).await?;
    Ok(Json(entries))
}

// ── Write Policy ──

/// GET /api/v1/data-sources/{id}/write-policy
#[utoipa::path(
    get,
    path = "/api/v1/data-sources/{id}/write-policy",
    params(
        ("id" = i64, Path, description = "Data Source ID")
    ),
    responses(
        (status = 200, body = WritePolicy),
        (status = 404),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_write_policy(
    State(state): State<AppState>,
    Path(data_source_id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    match state.storage.get_write_policy(data_source_id).await? {
        Some(policy) => Ok(Json(policy).into_response()),
        None => Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Data source not found"})),
        )
            .into_response()),
    }
}

/// PATCH /api/v1/data-sources/{id}/write-policy
#[utoipa::path(
    patch,
    path = "/api/v1/data-sources/{id}/write-policy",
    params(
        ("id" = i64, Path, description = "Data Source ID")
    ),
    request_body = WritePolicyUpdate,
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_write_policy(
    State(state): State<AppState>,
    Path(data_source_id): Path<i64>,
    Json(body): Json<WritePolicyUpdate>,
) -> Result<impl IntoResponse, AppError> {
    state
        .storage
        .update_write_policy(data_source_id, &body)
        .await?;
    Ok(Json(json!({"ok": true})))
}
