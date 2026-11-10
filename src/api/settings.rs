use axum::Json;
use axum::body::Body;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{AppError, AppState, AuthUser};

/// GET /api/settings/alarm-cooldown — get alarm cooldown in seconds
#[utoipa::path(
    get,
    path = "/api/v1/settings/alarm-cooldown",
    responses((status = 200)),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_alarm_cooldown(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let settings = state.storage.get_settings(&["alarm_cooldown_secs"]).await?;
    let secs: i64 = settings
        .into_iter()
        .next()
        .and_then(|s| s.value.parse().ok())
        .unwrap_or(300);
    Ok(Json(json!({ "secs": secs })))
}

/// PUT /api/settings/alarm-cooldown — set alarm cooldown in seconds
#[utoipa::path(
    put,
    path = "/api/v1/settings/alarm-cooldown",
    responses((status = 200)),
    tag = "Settings",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_alarm_cooldown(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let secs = body.get("secs").and_then(|v| v.as_i64()).unwrap_or(300);
    let clamped = secs.clamp(0, 86400);
    state
        .storage
        .upsert_setting("alarm_cooldown_secs", &clamped.to_string())
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_alarm_cooldown".to_string(),
        username: auth.username.clone(),
        resource: "settings:alarm-cooldown".to_string(),
        details: Some(format!("secs={}", clamped)),
    });

    Ok(Json(json!({"ok": true, "secs": clamped})))
}

/// GET /api/v1/settings/ui-theme
#[utoipa::path(
    get,
    path = "/api/v1/settings/ui-theme",
    responses((status = 200)),
    tag = "Settings",
    security(("cookie_auth" = []))
)]
pub async fn get_ui_theme(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Response<Body>, AppError> {
    let theme = state
        .storage
        .get_user_preference(&auth.username, "ui_theme")
        .await?
        .unwrap_or_else(|| "dark".to_string());
    Ok(Json(json!({ "theme": theme })).into_response())
}

/// PUT /api/v1/settings/ui-theme
#[utoipa::path(
    put,
    path = "/api/v1/settings/ui-theme",
    responses((status = 200)),
    tag = "Settings",
    security(("cookie_auth" = []))
)]
pub async fn update_ui_theme(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response<Body>, AppError> {
    let theme = body.get("theme").and_then(|v| v.as_str()).unwrap_or("dark");
    let valid = ["light", "dark", "system"].contains(&theme);
    if !valid {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid theme. Must be 'light', 'dark', or 'system'."})),
        )
            .into_response());
    }
    state
        .storage
        .set_user_preference(&auth.username, "ui_theme", theme)
        .await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_ui_theme".to_string(),
        username: auth.username.clone(),
        resource: "settings:ui-theme".to_string(),
        details: Some(format!("theme={}", theme)),
    });
    Ok(Json(json!({ "ok": true, "theme": theme })).into_response())
}

/// GET /api/v1/settings/ui-lang
#[utoipa::path(
    get,
    path = "/api/v1/settings/ui-lang",
    responses((status = 200)),
    tag = "Settings",
    security(("cookie_auth" = []))
)]
pub async fn get_ui_lang(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Response<Body>, AppError> {
    let lang = state
        .storage
        .get_user_preference(&auth.username, "ui_lang")
        .await?
        .unwrap_or_else(|| "en".to_string());
    Ok(Json(json!({ "lang": lang })).into_response())
}

/// PUT /api/v1/settings/ui-lang
#[utoipa::path(
    put,
    path = "/api/v1/settings/ui-lang",
    responses((status = 200)),
    tag = "Settings",
    security(("cookie_auth" = []))
)]
pub async fn update_ui_lang(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response<Body>, AppError> {
    let lang = body.get("lang").and_then(|v| v.as_str()).unwrap_or("en");
    let valid = ["en", "id", "zh", "de"].contains(&lang);
    if !valid {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid language. Must be one of: en, id, zh, de."})),
        )
            .into_response());
    }
    state
        .storage
        .set_user_preference(&auth.username, "ui_lang", lang)
        .await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_ui_lang".to_string(),
        username: auth.username.clone(),
        resource: "settings:ui-lang".to_string(),
        details: Some(format!("lang={}", lang)),
    });
    Ok(Json(json!({ "ok": true, "lang": lang })).into_response())
}

/// Setting an admin flips to put the MCP endpoint in read-only mode at runtime.
const MCP_READ_ONLY_SETTING: &str = "mcp_read_only";

/// Whether the MCP endpoint refuses every write tool: `mcp.read_only` in the
/// config, or the admin's runtime switch. Fails closed: a storage error reads
/// as read-only.
pub async fn mcp_read_only(state: &AppState) -> bool {
    if state.mcp_guard.config().read_only {
        return true;
    }
    match state.storage.get_settings(&[MCP_READ_ONLY_SETTING]).await {
        Ok(settings) => settings.iter().any(|s| s.value == "true"),
        Err(e) => {
            tracing::warn!("MCP read-only setting unreadable, treating MCP as read-only: {e}");
            true
        }
    }
}

async fn mcp_settings_body(state: &AppState) -> serde_json::Value {
    let config = state.mcp_guard.config();
    json!({
        "read_only": mcp_read_only(state).await,
        "read_only_locked_by_config": config.read_only,
        "require_confirmation": match config.require_confirmation {
            crate::models::ConfirmationMode::Off => "off",
            crate::models::ConfirmationMode::Destructive => "write_tag_and_destructive",
            crate::models::ConfirmationMode::All => "all",
        },
        "confirmation_ttl_secs": config.confirmation_ttl_secs,
        "calls_per_minute": config.calls_per_minute,
        "writes_per_minute": config.writes_per_minute,
    })
}

/// GET /api/v1/settings/mcp — the MCP guardrails in force
pub async fn get_mcp_settings(State(state): State<AppState>) -> impl IntoResponse {
    Json(mcp_settings_body(&state).await)
}

#[derive(serde::Deserialize)]
pub struct McpSettingsUpdate {
    pub read_only: bool,
}

/// PUT /api/v1/settings/mcp — switch MCP read-only mode at runtime
pub async fn update_mcp_settings(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<McpSettingsUpdate>,
) -> Result<Response, AppError> {
    if !body.read_only && state.mcp_guard.config().read_only {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({"error": "mcp.read_only is set in the config file; change it there"})),
        )
            .into_response());
    }
    state
        .storage
        .upsert_setting(
            MCP_READ_ONLY_SETTING,
            if body.read_only { "true" } else { "false" },
        )
        .await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_mcp_settings".to_string(),
        username: auth.username.clone(),
        resource: "mcp".to_string(),
        details: Some(format!("read_only={}", body.read_only)),
    });
    Ok(Json(mcp_settings_body(&state).await).into_response())
}
