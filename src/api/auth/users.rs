use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::*;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct UserCreatedResponse {
    pub ok: bool,
    pub id: i64,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct OkResponse {
    pub ok: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/users",
    responses(
        (status = 200, body = Vec<crate::models::User>),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_users(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let users = state.storage.list_users().await?;
    Ok(Json(users))
}

#[utoipa::path(
    post,
    path = "/api/v1/users",
    request_body = crate::models::CreateUserRequest,
    responses(
        (status = 200, body = UserCreatedResponse),
        (status = 400),
        (status = 409),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_user_handler(
    State(state): State<AppState>,
    Extension(auth): Extension<super::super::AuthUser>,
    Json(body): Json<CreateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.username.is_empty() || body.password.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "username and password are required"})),
        )
            .into_response());
    }
    if body.role != "admin"
        && body.role != "user"
        && body.role != "demo"
        && body.role != "operator"
        && body.role != "viewer"
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unknown role"})),
        )
            .into_response());
    }
    if state
        .storage
        .get_user_by_username(&body.username)
        .await?
        .is_some()
    {
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({"error": "username already exists"})),
        )
            .into_response());
    }
    let password_hash = hash_password(&body.password);
    let id = state
        .storage
        .create_user(&body.username, &password_hash, "", &body.role)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_user".to_string(),
        username: auth.username.clone(),
        resource: format!("user:{}", body.username),
        details: Some(format!("role={}", body.role)),
    });

    Ok(Json(json!({"ok": true, "id": id})).into_response())
}

#[utoipa::path(
    put,
    path = "/api/v1/users/{id}",
    params(
        ("id" = i64, Path, description = "User ID")
    ),
    request_body = crate::models::UpdateUserRequest,
    responses(
        (status = 200, body = OkResponse),
        (status = 400),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_user_handler(
    State(state): State<AppState>,
    Extension(auth): Extension<super::super::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_id(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;
    let mut details = Vec::new();
    if let Some(role) = body.role {
        if role != "admin"
            && role != "user"
            && role != "demo"
            && role != "operator"
            && role != "viewer"
        {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "unknown role"})),
            )
                .into_response());
        }
        state.storage.update_user_role(id, &role).await?;
        details.push(format!("role={}", role));
    }
    if let Some(password) = body.password {
        if password.is_empty() {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "password cannot be empty"})),
            )
                .into_response());
        }
        let hash = hash_password(&password);
        state.storage.update_user_password(id, &hash).await?;
        details.push("password_changed".to_string());
    }
    if !details.is_empty() {
        // End the user's sessions: the middleware already reads the role from
        // the database, and revoking refresh tokens stops a changed password
        // from being bypassed with an old session.
        state
            .storage
            .revoke_all_user_refresh_tokens(&user.username)
            .await?;
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_user".to_string(),
        username: auth.username.clone(),
        resource: format!("user:{}", user.username),
        details: Some(details.join(",")),
    });

    Ok(Json(json!({"ok": true})).into_response())
}

#[utoipa::path(
    delete,
    path = "/api/v1/users/{id}",
    params(
        ("id" = i64, Path, description = "User ID")
    ),
    responses(
        (status = 200, body = OkResponse),
        (status = 400),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_user_handler(
    State(state): State<AppState>,
    Extension(auth): Extension<super::super::AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let user = state
        .storage
        .get_user_by_id(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;
    if user.username == auth.username {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "cannot delete yourself"})),
        )
            .into_response());
    }
    state.storage.delete_user(id).await?;
    state
        .storage
        .revoke_all_user_refresh_tokens(&user.username)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_user".to_string(),
        username: auth.username.clone(),
        resource: format!("user:{}", user.username),
        details: None,
    });

    Ok(Json(json!({"ok": true})).into_response())
}
