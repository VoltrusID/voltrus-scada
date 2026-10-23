use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

// ── Query params ──

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListNotificationsQuery {
    pub unread_only: Option<bool>,
}

// ── Handlers ──

#[utoipa::path(
    get,
    path = "/api/v1/user-notifications",
    tag = "Notifications",
    params(ListNotificationsQuery),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_notifications(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Query(params): Query<ListNotificationsQuery>,
) -> Result<impl IntoResponse, AppError> {
    // Resolve current user ID from authenticated username
    let user = state
        .storage
        .get_user_by_username(&auth_user.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Authenticated user not found"))?;
    let user_id = user.id;

    let unread_only = params.unread_only.unwrap_or(false);

    // Auto-cleanup old notifications (best-effort, ignore errors)
    let _ = state.storage.cleanup_old_notifications().await;

    let notifications = state
        .storage
        .list_user_notifications(user_id, unread_only)
        .await?;

    Ok(Json(json!({
        "notifications": notifications,
        "count": notifications.len(),
    })))
}

#[utoipa::path(
    put,
    path = "/api/v1/user-notifications/{id}/read",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn mark_read(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    // Resolve current user ID from authenticated username
    let user = state
        .storage
        .get_user_by_username(&auth_user.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Authenticated user not found"))?;
    let user_id = user.id;

    // Verify ownership: list user's notifications and find the target
    let user_notifications = state
        .storage
        .list_user_notifications(user_id, false)
        .await?;

    let owned = user_notifications.iter().any(|n| n.id == id);

    if !owned {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Notification not found"})),
        )
            .into_response());
    }

    state.storage.mark_notification_read(id).await?;

    Ok(Json(json!({ "ok": true })).into_response())
}

#[utoipa::path(
    delete,
    path = "/api/v1/user-notifications/{id}",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification ID")
    ),
    responses(
        (status = 204),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn dismiss_notification(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    // Resolve current user ID from authenticated username
    let user = state
        .storage
        .get_user_by_username(&auth_user.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Authenticated user not found"))?;
    let user_id = user.id;

    // Verify ownership: list user's notifications and find the target
    let user_notifications = state
        .storage
        .list_user_notifications(user_id, false)
        .await?;

    let owned = user_notifications.iter().any(|n| n.id == id);

    if !owned {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Notification not found"})),
        )
            .into_response());
    }

    state.storage.dismiss_notification(id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// ── Tests ──
