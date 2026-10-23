use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::models::*;

/// GET /api/screens — list all screens (devices)
#[utoipa::path(
    get,
    path = "/api/v1/screens",
    responses((status = 200, body = Vec<crate::models::Device>)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_screens(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let devices = state.storage.list_devices().await?;
    Ok(Json(devices))
}

/// POST /api/screens — create a new screen
#[utoipa::path(
    post,
    path = "/api/v1/screens",
    responses((status = 201, body = crate::models::Device)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_screen(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let name = body["name"].as_str().unwrap_or("New Screen").to_string();

    // Generate slug id from name
    let slug = name
        .to_lowercase()
        .replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    let slug = slug.trim_matches('_').to_string();
    let base_id = if slug.is_empty() {
        format!("screen_{}", crate::timeutil::now_millis())
    } else {
        slug
    };

    // Deduplicate id if it already exists
    let mut id = base_id.clone();
    let mut counter = 1;
    while state.storage.device_exists(&id).await? {
        id = format!("{}_{}", base_id, counter);
        counter += 1;
    }

    // Get max sort_order
    let max_sort = state.storage.max_device_sort_order().await?;

    state
        .storage
        .create_device(&id, &name, max_sort + 1)
        .await?;

    let device = Device {
        id: id.clone(),
        name: name.clone(),
        device_type: "screen".to_string(),
        sort_order: max_sort + 1,
    };

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_screen".to_string(),
        username: auth.username.clone(),
        resource: format!("screen:{}", id),
        details: Some(format!("name={}", name)),
    });

    Ok((StatusCode::CREATED, Json(device)))
}

/// PUT /api/screens/{id} — update a screen
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Screen updated")),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_screen(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let name = body["name"].as_str();
    let sort_order = body["sort_order"].as_i64();

    state.storage.update_device(&id, name, sort_order).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_screen".to_string(),
        username: auth.username.clone(),
        resource: format!("screen:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/screens/{id} — delete a screen and its data source associations
#[utoipa::path(
    delete,
    path = "/api/v1/screens/{id}",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Screen deleted")),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_screen(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Delete screen_data_sources first
    state
        .storage
        .delete_screen_data_sources_by_screen(&id)
        .await?;
    state.storage.delete_device(&id).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_screen".to_string(),
        username: auth.username.clone(),
        resource: format!("screen:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// GET /api/screens/{id}/data-sources — list data sources for a screen
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/data-sources",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, body = Vec<crate::models::DataSource>)),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_screen_sensors(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let data_sources = state.storage.list_data_sources(Some(&id)).await?;
    Ok(Json(data_sources))
}

/// POST /api/screens/{id}/data-sources — add a data source to a screen
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/data-sources",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 201, description = "Data source added to screen")),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_screen_sensor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let data_source_id = body["data_source_id"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("data_source_id is required"))?;

    // Get max sort_order for this screen's data sources
    let existing = state.storage.list_data_sources(Some(&id)).await?;
    let max_sort = existing.iter().map(|s| s.sort_order).max().unwrap_or(-1);

    state
        .storage
        .add_screen_data_source(&id, data_source_id, max_sort + 1)
        .await?;

    Ok((StatusCode::CREATED, Json(json!({"ok": true}))))
}

/// PUT /api/screens/{id}/data-sources/{data_source_id} — update screen data source sort order
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/data-sources/{sensor_id}",
    params(
        ("id" = String, Path, description = "Screen ID"),
        ("sensor_id" = i64, Path, description = "Data source ID")
    ),
    responses((status = 200, description = "Screen data source updated")),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_screen_sensor(
    State(state): State<AppState>,
    Path((id, data_source_id)): Path<(String, i64)>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let sort_order = body["sort_order"]
        .as_i64()
        .ok_or_else(|| anyhow::anyhow!("sort_order is required"))?;

    state
        .storage
        .update_screen_data_source_sort(&id, data_source_id, sort_order)
        .await?;

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/screens/{id}/data-sources/{data_source_id} — remove data source from screen
#[utoipa::path(
    delete,
    path = "/api/v1/screens/{id}/data-sources/{sensor_id}",
    params(
        ("id" = String, Path, description = "Screen ID"),
        ("sensor_id" = i64, Path, description = "Data source ID")
    ),
    responses((status = 200, description = "Data source removed from screen")),
    tag = "Screens",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_screen_sensor(
    State(state): State<AppState>,
    Path((id, data_source_id)): Path<(String, i64)>,
) -> Result<impl IntoResponse, AppError> {
    state
        .storage
        .remove_screen_data_source(&id, data_source_id)
        .await?;

    Ok(Json(json!({"ok": true})))
}
