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
