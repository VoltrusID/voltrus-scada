//! Collaboration over REST (docs/collaboration.md): who edits which screen,
//! and comments pinned to a screen's objects. The live document itself goes
//! over the WebSocket (`crate::collab`).

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::storage::design_comments::{self as dc, CommentRefusal};
use crate::storage::{Comment, CommentPatch, NewComment};
use crate::ws_types::ServerMessage;

/// GET /api/v1/design/sessions — who edits which screen now.
#[utoipa::path(
    get,
    path = "/api/v1/design/sessions",
    responses((status = 200, description = "`{sessions: [{screen, users: [{user, color}]}], enabled}`")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_sessions(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let enabled = state.storage.design_settings().await?.collaboration;
    Ok(Json(json!({
        "enabled": enabled,
        "protocol": voltrus_collab::PROTOCOL,
        "sessions": state.collab.sessions_info(),
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct CommentQuery {
    pub resolved: Option<String>,
}

fn comment_error(e: anyhow::Error) -> AppError {
    match e.downcast_ref::<CommentRefusal>() {
        Some(CommentRefusal::Invalid(m)) => AppError::bad_request(m.clone()),
        Some(CommentRefusal::Forbidden(m)) => AppError::forbidden(m.clone()),
        Some(CommentRefusal::NotFound) => AppError::not_found("comment not found"),
        Some(CommentRefusal::NoScreen) => AppError::not_found("screen not found"),
        Some(CommentRefusal::TooMany(m)) => AppError::too_many_requests(m.clone()),
        None => AppError(e),
    }
}

/// Tell the people in the screen's session.
fn announce(state: &AppState, screen: &str, event: &str, c: &Comment) {
    let Some(session) = state.collab.session(screen) else {
        return;
    };
    let msg = ServerMessage::DesignComment {
        screen: screen.to_string(),
        event: event.to_string(),
        comment: serde_json::to_value(c).unwrap_or_default(),
    };
    for conn in session.member_conns() {
        state.ws_manager.send_to(&conn, msg.clone());
    }
}

/// GET /api/v1/screens/{id}/comments — the open threads of a screen, oldest
/// first; `?resolved=1` adds resolved ones.
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/comments",
    params(
        ("id" = String, Path, description = "Screen id"),
        ("resolved" = Option<String>, Query, description = "1: resolved threads too"),
    ),
    responses((status = 200, description = "`{comments: [...]}`")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_comments(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<CommentQuery>,
) -> Result<impl IntoResponse, AppError> {
    let resolved = matches!(q.resolved.as_deref(), Some("1" | "true" | "yes"));
    let comments = state.storage.design_comments(&id, resolved).await?;
    Ok(Json(json!({ "comments": comments })))
}

/// POST /api/v1/screens/{id}/comments
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/comments",
    params(("id" = String, Path, description = "Screen id")),
    request_body = NewComment,
    responses((status = 200, body = Comment), (status = 400)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn post_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<NewComment>,
) -> Result<impl IntoResponse, AppError> {
    let (screen, author) = (id.clone(), user.username.clone());
    let c = state
        .storage
        .design_tx(move |conn| dc::create(conn, &screen, &author, &body))
        .await
        .map_err(comment_error)?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "design_comment".into(),
        username: user.username.clone(),
        resource: format!("screen:{id}"),
        details: Some(format!(
            "comment={} object={}",
            c.id,
            c.object_id.as_deref().unwrap_or("-")
        )),
    });
    announce(&state, &id, "created", &c);
    Ok(Json(c))
}

/// PATCH /api/v1/screens/{id}/comments/{cid} — edit the text (author) or
/// resolve/reopen the thread.
#[utoipa::path(
    patch,
    path = "/api/v1/screens/{id}/comments/{cid}",
    params(("id" = String, Path, description = "Screen id"), ("cid" = i64, Path, description = "Comment id")),
    request_body = CommentPatch,
    responses((status = 200, body = Comment), (status = 400), (status = 403), (status = 404)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn patch_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((id, cid)): Path<(String, i64)>,
    Json(body): Json<CommentPatch>,
) -> Result<impl IntoResponse, AppError> {
    let (screen, actor) = (id.clone(), user.username.clone());
    let c = state
        .storage
        .design_tx(move |conn| dc::update(conn, &screen, cid, &actor, &body))
        .await
        .map_err(comment_error)?;
    announce(&state, &id, "updated", &c);
    Ok(Json(c))
}

/// DELETE /api/v1/screens/{id}/comments/{cid} — its author or an
/// administrator; a thread's first comment takes the replies with it.
#[utoipa::path(
    delete,
    path = "/api/v1/screens/{id}/comments/{cid}",
    params(("id" = String, Path, description = "Screen id"), ("cid" = i64, Path, description = "Comment id")),
    responses((status = 200), (status = 403), (status = 404)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((id, cid)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    let (screen, actor, admin) = (id.clone(), user.username.clone(), user.role == "admin");
    let c = state
        .storage
        .design_tx(move |conn| dc::delete(conn, &screen, cid, &actor, admin))
        .await
        .map_err(comment_error)?;
    announce(&state, &id, "deleted", &c);
    Ok(Json(json!({"ok": true, "id": c.id})))
}
