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
    // 1. Get write policy — also validates that the data source exists
    let policy = state
        .storage
        .get_write_policy(tag_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Data source not found"))?;

    // 2. Check writable
    if !policy.writable {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Tag is not writable"})),
        )
            .into_response());
    }

    // 3. Check write_requires_reason
    if policy.write_requires_reason && body.reason.is_none() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Reason is required for writes on this tag"})),
        )
            .into_response());
    }

    // 4. Parse numeric value from body.value
    let value_f64 = body
        .value
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("Invalid numeric value"))?;

    // 5. Check value within write_min / write_max range
    if let Some(min) = policy.write_min
        && value_f64 < min
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Value {} is below minimum {}", value_f64, min)})),
        )
            .into_response());
    }
    if let Some(max) = policy.write_max
        && value_f64 > max
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Value {} is above maximum {}", value_f64, max)})),
        )
            .into_response());
    }

    // 6. Check deadband against current cached value
    if let Some(deadband) = policy.write_deadband
        && deadband > 0.0
    {
        let key = state.storage.get_data_source_key(tag_id).await?;
        if let Some(key) = key
            && let Some(current) = state.cache.get_metric_value(&key).await
            && (value_f64 - current).abs() <= deadband
        {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": format!(
                        "Value {} is within deadband {} of current value {}",
                        value_f64, deadband, current
                    )
                })),
            )
                .into_response());
        }
    }

    // 7. Check rate limit
    if let Some(max_rate) = policy.write_max_rate
        && max_rate > 0
    {
        let recent = state.storage.count_recent_writes(tag_id, 60).await?;
        if recent >= max_rate {
            return Ok((
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "error": format!(
                        "Rate limit exceeded: {} writes in the last 60s (max {})",
                        recent, max_rate
                    )
                })),
            )
                .into_response());
        }
    }

    // 8. Get device info
    let target = state
        .storage
        .get_data_source_device_id(tag_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Device not found for data source"))?;

    // 9. Create and insert Command
    let now = crate::timeutil::now_secs();
    let command_id: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(21)
        .map(char::from)
        .collect::<String>()
        .to_lowercase();
    let cmd = Command {
        id: command_id.clone(),
        data_source_id: tag_id,
        device_id: target.device_id,
        protocol: target.protocol,
        value: value_f64.to_string(),
        status: CommandStatus::Pending.as_str().to_string(),
        policy: body.policy.clone(),
        scheduled_at: body.scheduled_at,
        retry_count: 0,
        max_retries: 3,
        error_message: None,
        created_at: now,
        sent_at: None,
        acked_at: None,
        resolved_at: None,
        created_by: user.username.clone(),
        device_ref: target.device_ref,
        readback_status: None,
        readback_value: None,
        readback_attempts: 0,
        readback_ts: None,
    };

    state.storage.insert_command(&cmd).await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({
            "command_id": command_id,
            "status": "pending",
            "data_source_id": tag_id,
            "value": value_f64,
            "created_at": now,
        })),
    )
        .into_response())
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
        .ok_or_else(|| anyhow::anyhow!("Command not found"))?;
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
        .ok_or_else(|| anyhow::anyhow!("Command not found"))?;

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

// ── Update Write Policy ──

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
