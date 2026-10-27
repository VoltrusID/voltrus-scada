use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::models::*;

/// GET /api/alarms/active/count
#[utoipa::path(
    get,
    path = "/api/v1/alarms/active/count",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn active_alarm_count(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let count = state.storage.count_active_alarms().await?;
    Ok(Json(json!({ "count": count })))
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams, utoipa::ToSchema)]
pub struct AlarmListQuery {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub state: Option<String>,
    pub shelved: Option<bool>,
    pub limit: Option<i64>,
}

/// GET /api/v1/alarms?status=active|acknowledged|shelved|all&priority=...&state=...&shelved=true|false&limit=50
///
/// `status` narrows by lifecycle, so the names are narrower than they read:
/// `active` is the alarms still waiting for an operator (state `active`, not
/// acknowledged); an acknowledged alarm that has not cleared is not in it, but
/// in `acknowledged`, which also holds acknowledged alarms that have since
/// resolved. `shelved` is the shelved alarms. Anything else, `all` included,
/// is every alarm. Resolved alarms carry `resolved_at`.
#[utoipa::path(
    get,
    path = "/api/v1/alarms",
    params(AlarmListQuery),
    responses(
        (status = 200, body = Vec<Alarm>),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_alarms(
    State(state): State<AppState>,
    Query(params): Query<AlarmListQuery>,
) -> Result<impl IntoResponse, AppError> {
    let status = params.status.as_deref();
    let limit = params.limit.unwrap_or(50);
    let mut alarms = state.storage.list_alarms(status, limit).await?;

    // Resolve source_name from data_sources
    let sources = state.storage.list_data_sources(None).await?;
    let name_map: std::collections::HashMap<String, String> = sources
        .iter()
        .map(|s| (s.key.clone(), s.name.clone()))
        .collect();

    for alarm in &mut alarms {
        if let Some(name) = name_map.get(&alarm.source_key) {
            alarm.source_name = name.clone();
        } else {
            alarm.source_name = alarm.source_key.clone();
        }
    }

    // Apply additional in-memory filters (priority, state, shelved)
    if params.priority.is_some() || params.state.is_some() || params.shelved.is_some() {
        alarms.retain(|a| {
            if let Some(ref p) = params.priority
                && &a.priority != p
            {
                return false;
            }
            if let Some(ref s) = params.state
                && &a.state != s
            {
                return false;
            }
            if let Some(shelved) = params.shelved {
                let is_shelved = a
                    .shelved_until
                    .map(|t| t > crate::timeutil::now_secs())
                    .unwrap_or(false);
                if is_shelved != shelved {
                    return false;
                }
            }
            true
        });
    }

    Ok(Json(alarms))
}

async fn get_cooldown_secs(storage: &crate::storage::Storage) -> i64 {
    storage
        .get_settings(&["alarm_cooldown_secs"])
        .await
        .ok()
        .and_then(|s| s.into_iter().next())
        .and_then(|s| s.value.parse().ok())
        .unwrap_or(300)
}

/// POST /api/alarms/{id}/acknowledge
#[utoipa::path(
    post,
    path = "/api/v1/alarms/{id}/acknowledge",
    params(
        ("id" = i64, Path, description = "Alarm ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn acknowledge_alarm(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let cooldown = get_cooldown_secs(&state.storage).await;
    let changed = state
        .storage
        .acknowledge_alarm(id, &user.username, cooldown)
        .await?;
    state.ws_manager.broadcast_alarm_updates(
        "acknowledged",
        Some(&user.username),
        changed.as_slice(),
    );
    Ok(Json(json!({"ok": true})))
}

/// POST /api/alarms/acknowledge-all
#[utoipa::path(
    post,
    path = "/api/v1/alarms",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn acknowledge_all(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let cooldown = get_cooldown_secs(&state.storage).await;
    let changed = state
        .storage
        .acknowledge_all(&user.username, cooldown)
        .await?;
    state
        .ws_manager
        .broadcast_alarm_updates("acknowledged", Some(&user.username), &changed);
    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/alarms/{id}/shelf
#[utoipa::path(
    post,
    path = "/api/v1/alarms/{id}/shelf",
    params(
        ("id" = i64, Path, description = "Alarm ID")
    ),
    request_body = ShelfAlarmRequest,
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn shelf_alarm(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<ShelfAlarmRequest>,
) -> Result<impl IntoResponse, AppError> {
    let now = crate::timeutil::now_secs();
    let shelved_until = now + body.duration_secs as i64;
    let changed = state
        .storage
        .shelf_alarm(id, shelved_until, &user.username, &body.reason)
        .await?;
    state
        .ws_manager
        .broadcast_alarm_updates("shelved", Some(&user.username), changed.as_slice());
    Ok(Json(json!({"ok": true, "shelved_until": shelved_until})))
}

/// POST /api/v1/alarms/{id}/unshelf
#[utoipa::path(
    post,
    path = "/api/v1/alarms/{id}/unshelf",
    params(
        ("id" = i64, Path, description = "Alarm ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn unshelf_alarm(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let changed = state.storage.unshelf_alarm(id).await?;
    state
        .ws_manager
        .broadcast_alarm_updates("unshelved", None, changed.as_slice());
    Ok(Json(json!({"ok": true})))
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct AuditQuery {
    pub alarm_id: Option<i64>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub limit: Option<u32>,
}

/// GET /api/v1/alarms/audit?alarm_id=X&from=TS&to=TS
#[utoipa::path(
    get,
    path = "/api/v1/alarms/audit",
    params(AuditQuery),
    responses(
        (status = 200, body = Vec<AlarmAuditEntry>),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_alarm_audit(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.unwrap_or(100);
    let entries = state
        .storage
        .list_alarm_audit(params.alarm_id, params.from, params.to, limit)
        .await?;
    Ok(Json(entries))
}

#[derive(Debug, serde::Deserialize)]
pub struct AnalyticsQuery {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// GET /api/v1/alarms/analytics?from=TS&to=TS
#[utoipa::path(
    get,
    path = "/api/v1/alarms/analytics",
    responses(
        (status = 200, body = Vec<AlarmAnalyticsEntry>),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_alarm_analytics(
    State(state): State<AppState>,
    Query(params): Query<AnalyticsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let entries = state
        .storage
        .get_alarm_analytics(params.from, params.to)
        .await?;
    Ok(Json(entries))
}

/// GET /api/v1/alarms/summary
#[utoipa::path(
    get,
    path = "/api/v1/alarms/summary",
    responses(
        (status = 200, body = AlarmSummary),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_alarm_summary(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let summary = state.storage.get_alarm_summary().await?;
    Ok(Json(summary))
}

/// GET /api/v1/alarms/{id}/journal — append-only ISA-18.2 lifecycle history
#[utoipa::path(
    get,
    path = "/api/v1/alarms/{id}/journal",
    params(
        ("id" = i64, Path, description = "Alarm ID")
    ),
    responses(
        (status = 200, body = Vec<AlarmJournalEntry>),
    ),
    tag = "Alarms",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_alarm_journal(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let entries = state.storage.list_alarm_journal(id, 200).await?;
    Ok(Json(entries))
}
