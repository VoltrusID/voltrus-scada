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
        (status = 428, description = "The edition wants this write signed: retry with `signature`"),
        (status = 429),
    ),
    tag = "Commands",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn write_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(tag_id): Path<i64>,
    Json(body): Json<WriteRequest>,
) -> Result<impl IntoResponse, AppError> {
    let origin = super::signature::origin_of(&user, api_key.is_some());
    write_tag_as(&state, &user, origin, tag_id, body).await
}

/// `POST /tags/{id}/write` for a caller of the given origin (REST or MCP).
pub async fn write_tag_as(
    state: &AppState,
    user: &AuthUser,
    origin: crate::edition::ActionOrigin,
    tag_id: i64,
    body: WriteRequest,
) -> Result<axum::response::Response, AppError> {
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
    let actor = Actor {
        username: &user.username,
        role: &user.role,
        origin,
    };
    match submit_tag_write(state, &actor, submission, body.signature.as_ref()).await {
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
    /// The edition wants the write signed and it was not, or not validly
    /// (428 or 403, see [`super::signature`]).
    Signature(super::signature::SignatureError),
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
            WriteError::Signature(e) => e.message(),
            WriteError::Internal(e) => {
                tracing::error!("Tag write failed: {e:#}");
                "Internal error".to_string()
            }
        }
    }
}

/// Who is writing: the user or key, its role, and how the write came in.
pub struct Actor<'a> {
    pub username: &'a str,
    pub role: &'a str,
    pub origin: crate::edition::ActionOrigin,
}

impl From<anyhow::Error> for WriteError {
    fn from(e: anyhow::Error) -> Self {
        WriteError::Internal(e)
    }
}

impl IntoResponse for WriteError {
    fn into_response(self) -> axum::response::Response {
        if let WriteError::Signature(e) = self {
            return e.into_response();
        }
        let status = match &self {
            WriteError::Signature(e) => e.status(),
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
/// deadband, rate limit), get it signed when the edition asks for a signature,
/// and put it into the command queue with an audit entry naming the actor and
/// holding the previous and the new value. Every interactive write entry point
/// (REST, WebSocket, MCP) goes through here so none can skip the policy.
pub async fn submit_tag_write(
    state: &AppState,
    actor: &Actor<'_>,
    req: TagWriteSubmission,
    signature: Option<&SignatureInput>,
) -> Result<Command, WriteError> {
    let value = req.value;
    let tag_id = req.tag_id;
    let checked = check_write(&state.storage, &state.cache, &req).await?;

    let ctx = crate::edition::ActionContext {
        action: crate::edition::SignedAction::TagWrite,
        origin: actor.origin,
        username: actor.username.to_string(),
        role: actor.role.to_string(),
        resource: format!("data_source:{tag_id}"),
        details: checked.details(value),
    };
    let signed = super::signature::authorize(state, &ctx, signature)
        .await
        .map_err(WriteError::Signature)?;

    let previous = checked.previous;
    let reason = checked.reason.clone();
    let cmd = insert_write(&state.storage, actor.username, req, checked).await?;

    let mut details = format!("command={}", cmd.id);
    if let Some(previous) = previous {
        details.push_str(&format!(" previous={previous}"));
    }
    details.push_str(&format!(" value={value}"));
    if let Some(signed) = &signed {
        details.push_str(&signed.audit_suffix());
    }
    if let Some(r) = reason {
        details.push_str(&format!(" reason={r}"));
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "tag_write".to_string(),
        username: actor.username.to_string(),
        resource: format!("data_source:{tag_id}"),
        details: Some(details),
    });

    Ok(cmd)
}

/// The write policy and the queue insert, without the security audit event:
/// for writes that start inside the server (flows, recipes) as well as the API.
/// The command, its reason and the previous value still go into the command
/// audit log.
///
/// A flow (`username` `flow:<id>`) cannot sign: when the edition wants a
/// signature for the write (asked with origin [`ActionOrigin::Flow`]), it is
/// refused. A recipe load asks for its signature before it queues any write.
///
/// [`ActionOrigin::Flow`]: crate::edition::ActionOrigin::Flow
pub async fn enqueue_checked_write(
    storage: &crate::storage::Storage,
    cache: &crate::cache::DeviceCache,
    username: &str,
    req: TagWriteSubmission,
) -> Result<Command, WriteError> {
    let checked = check_write(storage, cache, &req).await?;
    if username.starts_with("flow:") {
        let ext = crate::edition::installed();
        unattended_write_allowed(ext.as_ref(), username, &req, &checked).await?;
    }
    insert_write(storage, username, req, checked).await
}

/// Refuse a flow's write the edition wants signed: nobody is there to sign.
pub(crate) async fn unattended_write_allowed(
    ext: &dyn crate::edition::Extension,
    username: &str,
    req: &TagWriteSubmission,
    checked: &CheckedWrite,
) -> Result<(), WriteError> {
    let ctx = crate::edition::ActionContext {
        action: crate::edition::SignedAction::TagWrite,
        origin: crate::edition::ActionOrigin::Flow,
        username: username.to_string(),
        role: "flow".to_string(),
        resource: format!("data_source:{}", req.tag_id),
        details: checked.details(req.value),
    };
    match ext.signature_required(&ctx).await {
        None => Ok(()),
        Some(requirement) => Err(WriteError::Signature(
            super::signature::SignatureError::NotPossible(format!(
                "This write needs an electronic signature ({}), which a flow cannot give",
                requirement.meaning
            )),
        )),
    }
}

/// A write that passed the tag's policy, ready to be queued.
pub struct CheckedWrite {
    tag_id: i64,
    reason: Option<String>,
    target: CommandTarget,
    command_id: String,
    /// The tag's value in the live cache now, if it has one.
    pub previous: Option<f64>,
    key: String,
    device_ref: String,
}

impl CheckedWrite {
    /// Details of the write for the edition's signature hook.
    pub fn details(&self, value: f64) -> serde_json::Value {
        json!({
            "tag_id": self.tag_id,
            "key": self.key,
            "device_id": self.device_ref,
            "value": value,
            "previous": self.previous,
        })
    }
}

/// Check a write against the tag's write policy without queuing it.
pub async fn check_write(
    storage: &crate::storage::Storage,
    cache: &crate::cache::DeviceCache,
    req: &TagWriteSubmission,
) -> Result<CheckedWrite, WriteError> {
    let tag_id = req.tag_id;
    let value = req.value;

    // 1. Get write policy — also validates that the data source exists
    let policy = storage
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

    // 6. The current value: the audit trail's previous value, and the deadband
    //    reference. The cache holds a metric under the bare register name,
    //    scoped by device (the data source key may be device-qualified,
    //    `plts_1.voltage`).
    let source = storage.get_data_source_by_id(tag_id).await?;
    let previous = match &source {
        Some(source) => {
            cache
                .get_device_metric(
                    &source.device_id,
                    super::data::metric_name(&source.key, &source.device_id),
                )
                .await
        }
        None => None,
    };
    if let Some(deadband) = policy.write_deadband
        && deadband > 0.0
        && let Some(current) = previous
        && (value - current).abs() <= deadband
    {
        return Err(WriteError::Invalid(format!(
            "Value {value} is within deadband {deadband} of current value {current}"
        )));
    }

    // 7. Check rate limit
    if let Some(max_rate) = policy.write_max_rate
        && max_rate > 0
    {
        let recent = storage.count_recent_writes(tag_id, 60).await?;
        if recent >= max_rate {
            return Err(WriteError::RateLimited(format!(
                "Rate limit exceeded: {recent} writes in the last 60s (max {max_rate})"
            )));
        }
    }

    // 8. Get device info
    let target = storage
        .get_data_source_device_id(tag_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Device not found for data source"))?;

    // 9. The command id
    let command_id = match req.command_id.clone() {
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
            if storage.get_command(&id).await?.is_some() {
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
    let (key, device_ref) = source.map(|s| (s.key, s.device_id)).unwrap_or_default();
    Ok(CheckedWrite {
        tag_id,
        reason: reason.map(str::to_string),
        target,
        command_id,
        previous,
        key,
        device_ref,
    })
}

/// Queue a checked write, with the command audit log's first entry.
async fn insert_write(
    storage: &crate::storage::Storage,
    username: &str,
    req: TagWriteSubmission,
    checked: CheckedWrite,
) -> Result<Command, WriteError> {
    let CheckedWrite {
        reason,
        target,
        command_id,
        previous,
        ..
    } = checked;
    let now = crate::timeutil::now_secs();
    let cmd = Command {
        id: command_id,
        data_source_id: req.tag_id,
        device_id: target.device_id,
        protocol: target.protocol,
        value: req.value.to_string(),
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

    storage
        .insert_command_audited(&cmd, reason.as_deref(), previous)
        .await?;

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
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(data_source_id): Path<i64>,
    Json(body): Json<WritePolicyPatch>,
) -> Result<axum::response::Response, AppError> {
    let WritePolicyPatch {
        update: body,
        signature,
    } = body;
    let ctx = crate::edition::ActionContext {
        action: crate::edition::SignedAction::ModeSwitch,
        origin: super::signature::origin_of(&user, api_key.is_some()),
        username: user.username.clone(),
        role: user.role.clone(),
        resource: format!("data_source:{data_source_id}:write_policy"),
        details: json!({"mode": "write_policy", "tag_id": data_source_id, "policy": body}),
    };
    let signed = match super::signature::authorize(&state, &ctx, signature.as_ref()).await {
        Ok(signed) => signed,
        Err(e) => return Ok(e.into_response()),
    };
    state
        .storage
        .update_write_policy(data_source_id, &body)
        .await?;
    // The write policy is the guard on setpoint writes; who loosened it matters.
    let mut details = serde_json::to_string(&body).unwrap_or_default();
    if let Some(signed) = &signed {
        details.push_str(&signed.audit_suffix());
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_write_policy".to_string(),
        username: user.username.clone(),
        resource: format!("data_source:{data_source_id}"),
        details: Some(details),
    });
    Ok(Json(json!({"ok": true})).into_response())
}

/// `PATCH /data-sources/{id}/write-policy` body: the policy fields and, when
/// the edition asks for one, a signature.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct WritePolicyPatch {
    #[serde(flatten)]
    pub update: WritePolicyUpdate,
    #[serde(default)]
    pub signature: Option<SignatureInput>,
}
