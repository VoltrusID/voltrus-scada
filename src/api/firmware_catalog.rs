use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::asset::models::{CreateFirmwareCatalogRequest, FirmwareCatalogEntry};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListFirmwareQuery {
    pub vendor: Option<String>,
}

// ── List Firmware Catalog ──

#[utoipa::path(
    get,
    path = "/api/v1/firmware-catalog",
    params(ListFirmwareQuery),
    responses(
        (status = 200, body = Vec<FirmwareCatalogEntry>)
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_firmware_catalog(
    State(state): State<AppState>,
    Query(params): Query<ListFirmwareQuery>,
) -> Result<impl IntoResponse, AppError> {
    let entries = state
        .storage
        .asset()
        .list_firmware_catalog(params.vendor.as_deref())
        .await?;
    Ok(Json(entries))
}

// ── Create Firmware Catalog Entry ──

#[utoipa::path(
    post,
    path = "/api/v1/firmware-catalog",
    request_body = CreateFirmwareCatalogRequest,
    responses(
        (status = 201, body = FirmwareCatalogEntry),
        (status = 400),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_firmware_catalog(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<CreateFirmwareCatalogRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.vendor.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Vendor is required")));
    }
    if body.model.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Model is required")));
    }
    if body.latest_version.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Latest version is required")));
    }

    let entry = state
        .storage
        .asset()
        .create_firmware_catalog_entry(&body)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_firmware_catalog".to_string(),
        username: auth.username.clone(),
        resource: format!("firmware_catalog:{}", entry.id),
        details: Some(format!("{} {}", entry.vendor, entry.model)),
    });

    Ok((axum::http::StatusCode::CREATED, Json(entry)))
}

// ── Update Firmware Catalog Entry ──

#[utoipa::path(
    patch,
    path = "/api/v1/firmware-catalog/{id}",
    params(("id" = String, Path, description = "Firmware catalog entry ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_firmware_catalog(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    state
        .storage
        .asset()
        .update_firmware_catalog_entry(&id, &body)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_firmware_catalog".to_string(),
        username: auth.username.clone(),
        resource: format!("firmware_catalog:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Delete Firmware Catalog Entry ──

#[utoipa::path(
    delete,
    path = "/api/v1/firmware-catalog/{id}",
    params(("id" = String, Path, description = "Firmware catalog entry ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_firmware_catalog(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    state
        .storage
        .asset()
        .delete_firmware_catalog_entry(&id)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_firmware_catalog".to_string(),
        username: auth.username.clone(),
        resource: format!("firmware_catalog:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}
