use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::naming_convention::validate_tag_name;
use super::{AppError, AppState};
use crate::models::*;

/// GET /api/data-sources?screen=X — list data sources, optionally filtered by screen
#[utoipa::path(
    get,
    path = "/api/v1/data-sources",
    params(crate::models::DataQuery),
    responses((status = 200, body = Vec<crate::models::DataSource>)),
    tag = "Data Sources",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_data_sources(
    State(state): State<AppState>,
    Query(params): Query<DataQuery>,
) -> Result<impl IntoResponse, AppError> {
    let screen_id = params.screen.as_deref();
    let data_sources = state.storage.list_data_sources(screen_id).await?;
    Ok(Json(data_sources))
}

/// POST /api/data-sources — create a new data source
#[utoipa::path(
    post,
    path = "/api/v1/data-sources",
    responses((status = 201, description = "Data source created")),
    tag = "Data Sources",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_data_source(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response<Body>, AppError> {
    let key = body["key"].as_str().unwrap_or("data_source").to_string();
    let name = body["name"].as_str().unwrap_or("DataSource").to_string();
    let unit = body["unit"].as_str().unwrap_or("").to_string();
    let color = body["color"].as_str().unwrap_or("#f97316").to_string();
    let min_val = body["min_val"].as_f64().unwrap_or(0.0);
    let max_val = body["max_val"].as_f64().unwrap_or(100.0);
    let warn_val = body["warn_val"].as_f64().unwrap_or(0.0);
    let crit_val = body["crit_val"].as_f64().unwrap_or(0.0);
    let base_val = body["base_val"].as_f64().unwrap_or(50.0);
    let decimals = body["decimals"].as_i64().unwrap_or(1);
    let source_type = body["source_type"]
        .as_str()
        .unwrap_or("random_walk")
        .to_string();
    let source_config = body
        .get("source_config")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".to_string());
    let device_id = body["device_id"].as_str().unwrap_or("default").to_string();
    let equipment_id = body["equipment_id"].as_str();
    let chart_type = body["chart_type"]
        .as_str()
        .unwrap_or("sparkline")
        .to_string();

    // Validate name against naming convention
    if let Err(msg) = validate_tag_name(&state, &name).await {
        return Ok((StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response());
    }

    // Get max sort_order
    let max_sort = state.storage.max_data_source_sort_order().await?;

    let id = state
        .storage
        .create_data_source(
            &key,
            &name,
            &unit,
            &color,
            min_val,
            max_val,
            warn_val,
            crit_val,
            base_val,
            max_sort + 1,
            decimals,
            &source_type,
            &source_config,
            &device_id,
            equipment_id,
            &chart_type,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(json!({"id": id}))).into_response())
}

/// PUT /api/data-sources/{id} — update data source fields dynamically
#[utoipa::path(
    put,
    path = "/api/v1/data-sources/{id}",
    params(("id" = i64, Path, description = "Data source ID")),
    responses((status = 200, description = "Data source updated")),
    tag = "Data Sources",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_data_source(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<serde_json::Value>,
) -> Result<Response<Body>, AppError> {
    if let Some(name) = body.get("name").and_then(|v| v.as_str())
        && let Err(msg) = validate_tag_name(&state, name).await
    {
        return Ok((StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response());
    }
    state.storage.update_data_source(id, &body).await?;
    Ok(Json(json!({"ok": true})).into_response())
}

/// DELETE /api/data-sources/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/data-sources/{id}",
    params(("id" = i64, Path, description = "Data source ID")),
    responses((status = 200, description = "Data source deleted")),
    tag = "Data Sources",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_data_source(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.delete_data_source(id).await?;
    Ok(Json(json!({"ok": true})))
}

/// PATCH /api/data-sources/reorder — accept {order: [id1, id2, ...]}
#[utoipa::path(
    patch,
    path = "/api/v1/data-sources/reorder",
    responses((status = 200, description = "Data sources reordered")),
    tag = "Data Sources",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn reorder_data_sources(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let order: Vec<i64> = body["order"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("order array is required"))?
        .iter()
        .map(|v| {
            v.as_i64()
                .ok_or_else(|| anyhow::anyhow!("order must contain integer IDs"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    state.storage.reorder_data_sources(&order).await?;
    Ok(Json(json!({"ok": true})))
}
