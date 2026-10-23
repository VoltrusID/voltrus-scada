use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::IntoResponse;

use crate::api::{AppError, AppState, AuthUser};
use crate::asset::models::CreateMaintenanceRequest;

// ── List Maintenance Logs ──

#[utoipa::path(
    get,
    path = "/api/v1/assets/{id}/maintenance",
    params(("id" = String, Path, description = "Asset ID")),
    responses(
        (status = 200, body = Vec<crate::asset::models::MaintenanceLog>),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_maintenance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Verify asset exists
    state
        .storage
        .asset()
        .get_asset(&id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Asset not found")))?;

    let logs = state.storage.asset().list_maintenance_logs(&id).await?;
    Ok(Json(logs))
}

// ── Create Maintenance Log ──

#[utoipa::path(
    post,
    path = "/api/v1/assets/{id}/maintenance",
    params(("id" = String, Path, description = "Asset ID")),
    request_body = CreateMaintenanceRequest,
    responses(
        (status = 201, body = crate::asset::models::MaintenanceLog),
        (status = 400),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_maintenance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<CreateMaintenanceRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Verify asset exists
    state
        .storage
        .asset()
        .get_asset(&id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Asset not found")))?;

    if body.description.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Description is required")));
    }

    if body
        .technician
        .as_ref()
        .map(|s| s.trim())
        .unwrap_or("")
        .is_empty()
    {
        return Err(AppError(anyhow::anyhow!("Technician is required")));
    }

    if body.performed_at.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Performed-at date is required")));
    }

    let log = state
        .storage
        .asset()
        .create_maintenance_log(&id, &body, &auth.username)
        .await?;

    // Recompute risk score (maintenance date changed)
    let asset_storage = state.storage.asset();
    let _ = crate::asset::risk::recompute_and_update(&asset_storage, &id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_maintenance_log".to_string(),
        username: auth.username.clone(),
        resource: format!("asset:{}", id),
        details: Some(format!("log_id={}", log.id)),
    });

    Ok((axum::http::StatusCode::CREATED, Json(log)))
}
