use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
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

/// `base_revision` and `base_seq` of a save body: absent/null for no check.
pub(crate) fn layout_base(
    body: &serde_json::Value,
) -> Result<crate::storage::LayoutBase, AppError> {
    let revision = match body.get("base_revision") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(AppError::bad_request("base_revision must be a string")),
    };
    let seq = match body.get("base_seq") {
        None | Some(serde_json::Value::Null) => None,
        Some(v) => Some(
            v.as_i64()
                .filter(|n| *n >= 0)
                .ok_or_else(|| AppError::bad_request("base_seq must be a whole number"))?,
        ),
    };
    Ok(crate::storage::LayoutBase { revision, seq })
}

/// The 409 of a stale save: the current revision and seq, and the batches
/// since the base (who changed what).
pub(crate) fn conflict_error(
    revision: String,
    seq: i64,
    since: Vec<serde_json::Value>,
) -> AppError {
    AppError(
        super::RejectedJson(
            axum::http::StatusCode::CONFLICT,
            json!({
                "error": "This screen was changed by someone else since you opened it",
                "code": "conflict",
                "revision": revision,
                "seq": seq,
                "changes_since": since,
            }),
        )
        .into(),
    )
}

/// Answer of a checked save: 200 with the new revision and seq (and the
/// change-log batch), or 409 with the current ones.
fn layout_saved(outcome: crate::storage::LayoutSave) -> Result<impl IntoResponse, AppError> {
    match outcome {
        crate::storage::LayoutSave::Saved {
            revision,
            seq,
            batch,
        } => {
            let mut resp =
                Json(json!({"ok": true, "revision": revision, "seq": seq, "batch": batch}))
                    .into_response();
            if let Some(b) = batch {
                resp.extensions_mut()
                    .insert(super::design_history::AuditBatch(b));
            }
            Ok(resp)
        }
        crate::storage::LayoutSave::Conflict {
            revision,
            seq,
            since,
        } => Err(conflict_error(revision, seq, since)),
    }
}

/// `?published=1` / `?draft=1` on the layout GETs.
#[derive(Debug, Default, serde::Deserialize)]
pub struct ViewQuery {
    pub published: Option<String>,
    pub draft: Option<String>,
}

fn flag(v: &Option<String>) -> bool {
    matches!(
        v.as_deref().map(str::to_ascii_lowercase).as_deref(),
        Some("1" | "true" | "yes")
    )
}

/// Whether a read gets the published rows (docs/version-history.md):
/// `?published=1` asks for them, `?draft=1` for the draft (roles that
/// configure only). Without either, roles that do not configure get the
/// published rows and roles that configure (sessions and integration keys
/// alike) the draft, as before phase 3; a runtime reader that configures
/// passes `?published=1`.
fn wants_published(auth: Option<&AuthUser>, q: &ViewQuery) -> bool {
    let configures =
        auth.is_none_or(|a| super::auth_middleware::role_allows_configuration(&a.role));
    if !configures || flag(&q.published) {
        return true;
    }
    // `?draft=1`, or nothing: the draft.
    let _explicit_draft = flag(&q.draft);
    false
}

/// GET /api/screens/{id}/scada-equipment
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-equipment",
    params(
        ("id" = String, Path, description = "Screen ID"),
        ("published" = Option<String>, Query, description = "1: the rows operators see (the published copy, or the draft of a screen never published). Roles that do not configure always get these"),
        ("draft" = Option<String>, Query, description = "1: the draft (the default for roles that configure, API keys included)"),
    ),
    responses((status = 200, body = Vec<crate::models::ScadaEquipment>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_equipment(
    State(state): State<AppState>,
    auth: Option<Extension<AuthUser>>,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<impl IntoResponse, AppError> {
    let published = wants_published(auth.as_ref().map(|a| &a.0), &q);
    let view = state.storage.get_scada_layout_view(&id, published).await?;
    Ok(Json(json!({"items": view.equipment, "view": view.view})))
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
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let items = parse_equipment(&id, items_array(&body, "items")?);
    let base = layout_base(&body)?;
    let actor = super::design_history::request_actor(&auth, api_key.is_some(), &headers, "save");
    let outcome = state
        .storage
        .save_scada_layout(&id, Some(items), None, base, &actor)
        .await?;
    layout_saved(outcome)
}

/// GET /api/screens/{id}/scada-connections
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-connections",
    params(
        ("id" = String, Path, description = "Screen ID"),
        ("published" = Option<String>, Query, description = "1: the rows operators see (the published copy, or the draft of a screen never published). Roles that do not configure always get these"),
        ("draft" = Option<String>, Query, description = "1: the draft (the default for roles that configure, API keys included)"),
    ),
    responses((status = 200, body = Vec<crate::models::ScadaConnection>)),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_connections(
    State(state): State<AppState>,
    auth: Option<Extension<AuthUser>>,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<impl IntoResponse, AppError> {
    let published = wants_published(auth.as_ref().map(|a| &a.0), &q);
    let view = state.storage.get_scada_layout_view(&id, published).await?;
    Ok(Json(json!({"items": view.connections, "view": view.view})))
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
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let connections = parse_connections(&id, items_array(&body, "items")?);
    let base = layout_base(&body)?;
    let actor = super::design_history::request_actor(&auth, api_key.is_some(), &headers, "save");
    let outcome = state
        .storage
        .save_scada_layout(&id, None, Some(connections), base, &actor)
        .await?;
    layout_saved(outcome)
}

/// GET /api/screens/{id}/scada-layout — equipment, connections and the
/// layout's revision, read together.
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/scada-layout",
    params(
        ("id" = String, Path, description = "Screen ID"),
        ("published" = Option<String>, Query, description = "1: the rows operators see (the published copy, or the draft of a screen never published). Roles that do not configure always get these"),
        ("draft" = Option<String>, Query, description = "1: the draft (the default for roles that configure, API keys included)"),
    ),
    responses((status = 200, description = "{equipment, connections, revision, seq, view: draft|published, draft_differs}")),
    tag = "Layouts",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scada_layout(
    State(state): State<AppState>,
    auth: Option<Extension<AuthUser>>,
    Path(id): Path<String>,
    Query(q): Query<ViewQuery>,
) -> Result<impl IntoResponse, AppError> {
    let published = wants_published(auth.as_ref().map(|a| &a.0), &q);
    let view = state.storage.get_scada_layout_view(&id, published).await?;
    Ok(Json(json!({
        "equipment": view.equipment,
        "connections": view.connections,
        "revision": view.revision,
        "seq": view.seq,
        "view": view.view,
        "draft_differs": view.draft_differs,
    })))
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
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let equipment = parse_equipment(&id, items_array(&body, "equipment")?);
    let connections = parse_connections(&id, items_array(&body, "connections")?);
    let base = layout_base(&body)?;
    let actor = super::design_history::request_actor(&auth, api_key.is_some(), &headers, "save");
    let outcome = state
        .storage
        .save_scada_layout(&id, Some(equipment), Some(connections), base, &actor)
        .await?;
    layout_saved(outcome)
}
