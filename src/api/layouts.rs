use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState};
use crate::models::*;

/// GET /api/screens/{id}/layouts — get layouts for a screen, returned as {lg:[], md:[], sm:[]}
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Screen layouts by breakpoint")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let rows = state.storage.get_layouts(&id).await?;
    let mut result = serde_json::Map::new();
    for row in &rows {
        let layout: serde_json::Value =
            serde_json::from_str(&row.layout).unwrap_or(serde_json::json!([]));
        result.insert(row.breakpoint.clone(), layout);
    }
    // Ensure all breakpoints exist
    if !result.contains_key("lg") {
        result.insert("lg".to_string(), json!([]));
    }
    if !result.contains_key("md") {
        result.insert("md".to_string(), json!([]));
    }
    if !result.contains_key("sm") {
        result.insert("sm".to_string(), json!([]));
    }
    Ok(Json(serde_json::Value::Object(result)))
}

/// PUT /api/screens/{id}/layouts — accept {lg:[], md:[], sm:[]} and upsert per breakpoint
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Layouts saved")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    // Transform {lg:[], md:[], sm:[]} into array of {breakpoint, layout} for storage
    let mut layouts_array = vec![];
    for bp in &["lg", "md", "sm"] {
        if let Some(layout_val) = body.get(bp) {
            layouts_array.push(json!({
                "breakpoint": bp,
                "layout": layout_val
            }));
        }
    }
    state
        .storage
        .save_layouts(&id, &serde_json::Value::Array(layouts_array))
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/screens/{id}/twin-layouts — get twin layouts and camera for a screen
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/twin-layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Twin layouts and camera settings")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_twin_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let layouts = state.storage.get_twin_layouts(&id).await?;
    let camera_key = format!("twin_camera:{id}");
    let camera_settings = state
        .storage
        .get_settings(&[&camera_key])
        .await
        .unwrap_or_default();
    let camera: serde_json::Value = camera_settings
        .iter()
        .find(|s| s.key == camera_key)
        .and_then(|s| serde_json::from_str(&s.value).ok())
        .unwrap_or(json!({"x": 6, "y": 6, "z": 6}));
    Ok(Json(json!({"items": layouts, "camera": camera})))
}

/// PUT /api/screens/{id}/twin-layouts — accept {items: [...], camera?: {x,y,z}}
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/twin-layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Twin layouts saved")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_twin_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let arr = body
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected an array of twin layouts"))?;

    let mut layouts = Vec::new();
    for item in arr {
        let equipment_type = item["equipment_type"].as_str().unwrap_or("").to_string();
        layouts.push(TwinLayout {
            screen_id: id.clone(),
            equipment_id: item["equipment_id"].as_str().unwrap_or("").to_string(),
            equipment_type,
            equipment_name: item["equipment_name"].as_str().unwrap_or("").to_string(),
            position_x: item["position_x"].as_f64().unwrap_or(0.0),
            position_z: item["position_z"].as_f64().unwrap_or(0.0),
            rotation_y: item["rotation_y"].as_f64().unwrap_or(0.0),
        });
    }

    state.storage.save_twin_layouts(&id, &layouts).await?;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/screens/{id}/canvas-layouts — get canvas layouts for a screen
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/canvas-layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, body = Vec<crate::models::CanvasLayoutItem>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_canvas_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let layouts = state.storage.get_canvas_layouts(&id).await?;
    Ok(Json(layouts))
}

/// PUT /api/screens/{id}/canvas-layouts — accept array of {data_source_id, x, y, width, height}
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/canvas-layouts",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Canvas layouts saved")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_canvas_layouts(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let arr = body
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected an array of canvas layouts"))?;

    let mut layouts = Vec::new();
    for item in arr {
        layouts.push(CanvasLayoutItem {
            screen_id: id.clone(),
            data_source_id: item["data_source_id"]
                .as_i64()
                .ok_or_else(|| anyhow::anyhow!("data_source_id is required"))?,
            x: item["x"].as_f64().unwrap_or(0.0),
            y: item["y"].as_f64().unwrap_or(0.0),
            width: item["width"].as_f64().unwrap_or(300.0),
            height: item["height"].as_f64().unwrap_or(200.0),
        });
    }

    state.storage.save_canvas_layouts(&id, &layouts).await?;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/screens/{id}/twin-connections
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/twin-connections",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, body = Vec<crate::models::TwinConnection>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_twin_connections(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let connections = state.storage.get_twin_connections(&id).await?;
    Ok(Json(connections))
}

/// PUT /api/screens/{id}/twin-connections — accept array of {from_id, to_id}
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/twin-connections",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Twin connections saved")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_twin_connections(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let arr = body
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Expected an array of twin connections"))?;

    let mut connections = Vec::new();
    for item in arr {
        connections.push(TwinConnection {
            screen_id: id.clone(),
            from_id: item["from_id"].as_str().unwrap_or("").to_string(),
            to_id: item["to_id"].as_str().unwrap_or("").to_string(),
        });
    }

    state
        .storage
        .save_twin_connections(&id, &connections)
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// PUT /api/screens/{id}/twin-camera — save camera position {x, y, z}
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/twin-camera",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "Twin camera position saved")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_twin_camera(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let key = format!("twin_camera:{id}");
    let value = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    state.storage.upsert_setting(&key, &value).await?;
    Ok(Json(json!({"ok": true})))
}

fn parse_equipment(screen_id: &str, arr: &[serde_json::Value]) -> Vec<ScadaEquipment> {
    arr.iter()
        .map(|item| ScadaEquipment {
            screen_id: screen_id.to_string(),
            equipment_id: item["equipment_id"].as_str().unwrap_or("").to_string(),
            equipment_type: item["equipment_type"].as_str().unwrap_or("").to_string(),
            equipment_name: item["equipment_name"].as_str().unwrap_or("").to_string(),
            x: item["x"].as_f64().unwrap_or(0.0),
            y: item["y"].as_f64().unwrap_or(0.0),
            width: item["width"].as_f64().unwrap_or(100.0),
            height: item["height"].as_f64().unwrap_or(60.0),
            rotation: item["rotation"].as_f64().unwrap_or(0.0),
            sensor_bindings: item["sensor_bindings"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    serde_json::to_string(&item["sensor_bindings"]).unwrap_or("[]".to_string())
                }),
            config: item["config"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    serde_json::to_string(&item["config"]).unwrap_or("{}".to_string())
                }),
        })
        .collect()
}

fn parse_connections(screen_id: &str, arr: &[serde_json::Value]) -> Vec<ScadaConnection> {
    arr.iter()
        .map(|item| ScadaConnection {
            screen_id: screen_id.to_string(),
            connection_id: item["connection_id"].as_str().unwrap_or("").to_string(),
            from_equipment: item["from_equipment"].as_str().unwrap_or("").to_string(),
            to_equipment: item["to_equipment"].as_str().unwrap_or("").to_string(),
            waypoints: item["waypoints"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    serde_json::to_string(&item["waypoints"]).unwrap_or("[]".to_string())
                }),
            connection_type: item["connection_type"]
                .as_str()
                .unwrap_or("power")
                .to_string(),
        })
        .collect()
}

fn items_array<'a>(
    body: &'a serde_json::Value,
    key: &str,
) -> Result<&'a Vec<serde_json::Value>, AppError> {
    body.get(key)
        .and_then(|v| v.as_array())
        .ok_or_else(|| AppError::bad_request(format!("Expected {{\"{key}\": [...]}}")))
}

/// `base_revision` of a save body: a string, or absent/null for no check.
fn base_revision(body: &serde_json::Value) -> Result<Option<String>, AppError> {
    match body.get("base_revision") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(AppError::bad_request("base_revision must be a string")),
    }
}

/// Answer of a revision-checked save: 200 with the new revision, or 409 with the current one.
fn layout_saved(outcome: crate::storage::LayoutSave) -> Result<Json<serde_json::Value>, AppError> {
    match outcome {
        crate::storage::LayoutSave::Saved { revision } => {
            Ok(Json(json!({"ok": true, "revision": revision})))
        }
        crate::storage::LayoutSave::Conflict { revision } => Err(AppError(
            super::RejectedJson(
                axum::http::StatusCode::CONFLICT,
                json!({
                    "error": "This screen was changed by someone else since you opened it",
                    "code": "conflict",
                    "revision": revision,
                }),
            )
            .into(),
        )),
    }
}

/// GET /api/screens/{id}/scada-equipment
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-equipment",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, body = Vec<crate::models::ScadaEquipment>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_equipment(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let items = state.storage.get_scada_equipment(&id).await?;
    Ok(Json(json!({"items": items})))
}

/// PUT /api/screens/{id}/scada-equipment
///
/// Replaces the screen's equipment. With `base_revision` (from GET
/// scada-layout or an earlier save), answers 409 when the screen changed since.
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/scada-equipment",
    params(("id" = String, Path, description = "Screen ID")),
    responses(
        (status = 200, description = "SCADA equipment saved; body has the new `revision`"),
        (status = 409, description = "`base_revision` is stale; body has the current `revision`")
    ),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_scada_equipment(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let items = parse_equipment(&id, items_array(&body, "items")?);
    let base = base_revision(&body)?;
    let outcome = state
        .storage
        .save_scada_layout(&id, Some(items), None, base)
        .await?;
    layout_saved(outcome)
}

/// GET /api/screens/{id}/scada-connections
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-connections",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, body = Vec<crate::models::ScadaConnection>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_connections(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let connections = state.storage.get_scada_connections(&id).await?;
    Ok(Json(json!({"items": connections})))
}

/// PUT /api/screens/{id}/scada-connections
///
/// Replaces the screen's connections; `base_revision` as for scada-equipment.
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/scada-connections",
    params(("id" = String, Path, description = "Screen ID")),
    responses(
        (status = 200, description = "SCADA connections saved; body has the new `revision`"),
        (status = 409, description = "`base_revision` is stale; body has the current `revision`")
    ),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_scada_connections(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let connections = parse_connections(&id, items_array(&body, "items")?);
    let base = base_revision(&body)?;
    let outcome = state
        .storage
        .save_scada_layout(&id, None, Some(connections), base)
        .await?;
    layout_saved(outcome)
}

/// GET /api/screens/{id}/scada-layout — equipment, connections and the
/// layout's revision, read together.
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-layout",
    params(("id" = String, Path, description = "Screen ID")),
    responses((status = 200, description = "{equipment, connections, revision}")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_layout(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let (equipment, connections, revision) = state.storage.get_scada_layout(&id).await?;
    Ok(Json(
        json!({"equipment": equipment, "connections": connections, "revision": revision}),
    ))
}

/// PUT /api/screens/{id}/scada-layout — replace equipment and connections in
/// one transaction. With `base_revision`, answers 409 (and writes nothing)
/// when the screen changed since that revision.
#[utoipa::path(
    put,
    path = "/api/v1/screens/{id}/scada-layout",
    params(("id" = String, Path, description = "Screen ID")),
    request_body(description = "{equipment: [...], connections: [...], base_revision?: string}"),
    responses(
        (status = 200, description = "Saved; body has the new `revision`"),
        (status = 409, description = "`base_revision` is stale; body has the current `revision`")
    ),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn save_scada_layout(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let equipment = parse_equipment(&id, items_array(&body, "equipment")?);
    let connections = parse_connections(&id, items_array(&body, "connections")?);
    let base = base_revision(&body)?;
    let outcome = state
        .storage
        .save_scada_layout(&id, Some(equipment), Some(connections), base)
        .await?;
    layout_saved(outcome)
}
