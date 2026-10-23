use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::{AppError, AppState};
use crate::models::*;

// ── Permission check helper ──

/// Check if an auth user has a specific permission.
/// Admin users always pass.
pub fn has_permission(permissions: &[String], role: &str, perm: &str) -> bool {
    if role == "admin" {
        return true;
    }
    permissions.iter().any(|p| p == perm)
}

// ── List roles ──

#[utoipa::path(
    get,
    path = "/api/v1/roles",
    responses(
        (status = 200, body = Vec<crate::models::Role>),
        (status = 403),
    ),
    tag = "RBAC",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_roles(State(state): State<AppState>) -> Result<Response, AppError> {
    let roles = state.storage.list_roles().await?;
    Ok(Json(roles).into_response())
}

// ── Create role ──

#[utoipa::path(
    post,
    path = "/api/v1/roles",
    request_body = crate::models::CreateRoleRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "RBAC",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_role(
    State(state): State<AppState>,
    Extension(auth): Extension<super::AuthUser>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<Response, AppError> {
    if body.name.trim().is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "role name is required"})),
        )
            .into_response());
    }

    // Validate that permissions is a valid JSON array
    let perms: Vec<String> = match serde_json::from_str(&body.permissions) {
        Ok(p) => p,
        Err(_) => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "permissions must be a valid JSON array of strings"})),
            )
                .into_response());
        }
    };

    // Validate each permission string
    for p in &perms {
        if !ALL_PERMISSIONS.contains(&p.as_str()) {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("unknown permission: {}", p)})),
            )
                .into_response());
        }
    }

    let id = state
        .storage
        .create_role(&body.name, &body.permissions)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_role".to_string(),
        username: auth.username.clone(),
        resource: format!("role:{}", body.name),
        details: Some(format!("id={}, permissions={}", id, body.permissions)),
    });

    Ok(Json(json!({"ok": true, "id": id})).into_response())
}

// ── Update role ──

#[utoipa::path(
    put,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = i64, Path, description = "Role ID")
    ),
    request_body = crate::models::UpdateRoleRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "RBAC",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_role(
    State(state): State<AppState>,
    Extension(auth): Extension<super::AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateRoleRequest>,
) -> Result<Response, AppError> {
    // Validate permissions if provided
    if let Some(ref perms_json) = body.permissions {
        let perms: Vec<String> = match serde_json::from_str(perms_json) {
            Ok(p) => p,
            Err(_) => {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "permissions must be a valid JSON array of strings"})),
                )
                    .into_response());
            }
        };
        for p in &perms {
            if !ALL_PERMISSIONS.contains(&p.as_str()) {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("unknown permission: {}", p)})),
                )
                    .into_response());
            }
        }
    }

    state
        .storage
        .update_role(id, body.name.as_deref(), body.permissions.as_deref())
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_role".to_string(),
        username: auth.username.clone(),
        resource: format!("role:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})).into_response())
}

// ── Delete role ──

#[utoipa::path(
    delete,
    path = "/api/v1/roles/{id}",
    params(
        ("id" = i64, Path, description = "Role ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
        (status = 403),
    ),
    tag = "RBAC",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_role(
    State(state): State<AppState>,
    Extension(auth): Extension<super::AuthUser>,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let role = state.storage.get_role_by_id(id).await?;
    if role.is_none() {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "role not found"})),
        )
            .into_response());
    }

    state.storage.delete_role(id).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_role".to_string(),
        username: auth.username.clone(),
        resource: format!("role:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})).into_response())
}

// ── Assign role to user ──

#[utoipa::path(
    put,
    path = "/api/v1/users/{id}/role",
    params(
        ("id" = i64, Path, description = "User ID")
    ),
    request_body = crate::models::AssignRoleRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 404),
        (status = 403),
    ),
    tag = "RBAC",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn assign_user_role(
    State(state): State<AppState>,
    Extension(auth): Extension<super::AuthUser>,
    Path(user_id): Path<i64>,
    Json(body): Json<AssignRoleRequest>,
) -> Result<Response, AppError> {
    // Verify the role exists
    let role = state.storage.get_role_by_id(body.role_id).await?;
    if role.is_none() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "role not found"})),
        )
            .into_response());
    }

    // Verify the user exists
    let user = state.storage.get_user_by_id(user_id).await?;
    if user.is_none() {
        return Ok((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "user not found"})),
        )
            .into_response());
    }

    state
        .storage
        .assign_user_role(user_id, body.role_id)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "assign_role".to_string(),
        username: auth.username.clone(),
        resource: format!("user:{}", user_id),
        details: Some(format!("role_id={}", body.role_id)),
    });

    Ok(Json(json!({"ok": true})).into_response())
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_permission_admin_bypass() {
        let perms = vec![];
        assert!(has_permission(&perms, "admin", "screens:write"));
        assert!(has_permission(&perms, "admin", "users:manage"));
    }

    #[test]
    fn test_has_permission_with_perms() {
        let perms = vec!["screens:read".to_string(), "alarms:read".to_string()];
        assert!(has_permission(&perms, "user", "screens:read"));
        assert!(has_permission(&perms, "user", "alarms:read"));
        assert!(!has_permission(&perms, "user", "screens:write"));
        assert!(!has_permission(&perms, "user", "users:manage"));
    }

    #[test]
    fn test_has_permission_empty_perms() {
        let perms: Vec<String> = vec![];
        assert!(!has_permission(&perms, "user", "screens:read"));
    }
}
