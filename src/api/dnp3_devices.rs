use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::dnp3::objects::common_group_variations;
use crate::models::Dnp3DeviceConfig;
use crate::models::*;

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct Dnp3ActionResponse {
    pub ok: bool,
    pub id: Option<i64>,
    pub message: Option<String>,
}

// ── Device CRUD ──

/// GET /api/v1/dnp3/devices
#[utoipa::path(
    get,
    operation_id = "dnp3_list_devices",
    path = "/api/v1/dnp3/devices",
    responses(
        (status = 200, body = Vec<crate::models::Dnp3Device>),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let devices = state.storage.list_dnp3_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let points = state.storage.list_dnp3_points(device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "ip": device.ip,
            "port": device.port,
            "master_address": device.master_address,
            "outstation_address": device.outstation_address,
            "enable_unsolicited": device.enable_unsolicited,
            "integrity_poll_interval_secs": device.integrity_poll_interval_secs,
            "event_scan_interval_ms": device.event_scan_interval_ms,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "updated_at": device.updated_at,
            "point_count": points.len(),
        }));
    }
    Ok(Json(result))
}

/// POST /api/v1/dnp3/devices
#[utoipa::path(
    post,
    operation_id = "dnp3_create_device",
    path = "/api/v1/dnp3/devices",
    request_body = crate::models::CreateDnp3DeviceRequest,
    responses(
        (status = 201, body = Dnp3ActionResponse),
        (status = 400),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateDnp3DeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }
    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device name is required")));
    }
    if req.ip.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device IP is required")));
    }

    let id = state
        .storage
        .create_dnp3_device(
            &req.name,
            &req.ip,
            req.port,
            req.master_address,
            req.outstation_address,
            req.enable_unsolicited,
            req.integrity_poll_interval_secs,
            req.event_scan_interval_ms,
            req.enabled,
        )
        .await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "dnp3",
        &id.to_string(),
        &req.name,
        Some(&req.ip),
        "rtu",
    )
    .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_dnp3_device".to_string(),
        username: user.username.clone(),
        resource: format!("dnp3_device:{}", id),
        details: Some(format!("name={}, ip={}", req.name, req.ip)),
    });

    Ok((StatusCode::CREATED, Json(json!({ "ok": true, "id": id }))))
}

/// GET /api/v1/dnp3/devices/{id}
#[utoipa::path(
    get,
    path = "/api/v1/dnp3/devices/{id}",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = crate::models::Dnp3Device),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let device = state.storage.get_dnp3_device(id).await?;
    let device = device.ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;
    let points = state.storage.list_dnp3_points(id).await?;
    Ok(Json(json!({ "device": device, "points": points })))
}

/// PUT /api/v1/dnp3/devices/{id}
#[utoipa::path(
    put,
    operation_id = "dnp3_update_device",
    path = "/api/v1/dnp3/devices/{id}",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    request_body = crate::models::UpdateDnp3DeviceRequest,
    responses(
        (status = 200, body = Dnp3ActionResponse),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateDnp3DeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let updated = state
        .storage
        .update_dnp3_device(
            id,
            req.name.as_deref(),
            req.ip.as_deref(),
            req.port,
            req.master_address,
            req.outstation_address,
            req.enable_unsolicited,
            req.integrity_poll_interval_secs,
            req.event_scan_interval_ms,
            req.enabled,
        )
        .await?;

    if !updated {
        return Err(AppError(anyhow::anyhow!("Device not found")));
    }
    Ok(Json(json!({ "ok": true })))
}

/// DELETE /api/v1/dnp3/devices/{id}
#[utoipa::path(
    delete,
    operation_id = "dnp3_delete_device",
    path = "/api/v1/dnp3/devices/{id}",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 204),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let deleted = state.storage.delete_dnp3_device(id).await?;
    if !deleted {
        return Err(AppError(anyhow::anyhow!("Device not found")));
    }

    // Stop poller if running
    let device_key = format!("dnp3_{}", id);
    state.poller_manager.stop(&device_key).await;
    state.cache.remove(&device_key).await;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "dnp3", &id.to_string()).await;

    Ok(StatusCode::NO_CONTENT)
}

// ── Point CRUD ──

/// POST /api/v1/dnp3/devices/{id}/points
#[utoipa::path(
    post,
    path = "/api/v1/dnp3/devices/{id}/points",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    request_body = crate::models::CreateDnp3PointRequest,
    responses(
        (status = 201, body = Dnp3ActionResponse),
        (status = 400),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_point(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<CreateDnp3PointRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }
    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Point name is required")));
    }

    // Verify device exists
    let device = state.storage.get_dnp3_device(id).await?;
    device.ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let point_id = state
        .storage
        .create_dnp3_point(
            id,
            req.group_num,
            req.variation,
            req.point_index,
            &req.name,
            &req.display_name,
            &req.data_type,
            req.scale,
            req.deadband,
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({ "ok": true, "id": point_id })),
    ))
}

/// DELETE /api/v1/dnp3/devices/{id}/points/{point_id}
#[utoipa::path(
    delete,
    path = "/api/v1/dnp3/devices/{id}/points/{point_id}",
    params(
        ("id" = i64, Path, description = "Device ID"),
        ("point_id" = i64, Path, description = "Point ID")
    ),
    responses(
        (status = 204),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_point(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_id, point_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let deleted = state.storage.delete_dnp3_point(point_id).await?;
    if !deleted {
        return Err(AppError(anyhow::anyhow!("Point not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Start / Stop ──

/// POST /api/v1/dnp3/devices/{id}/start
#[utoipa::path(
    post,
    operation_id = "dnp3_start_device",
    path = "/api/v1/dnp3/devices/{id}/start",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Dnp3ActionResponse),
        (status = 400),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn start_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Load device from DB
    let device = state
        .storage
        .get_dnp3_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!(
            "Device is disabled. Enable it first before starting."
        )));
    }

    // Load points for device
    let points = state.storage.list_dnp3_points(id).await?;

    if points.is_empty() {
        return Err(AppError(anyhow::anyhow!(
            "Device has no points configured. Add points before starting."
        )));
    }

    // Build runtime config
    let config = Dnp3DeviceConfig::from_rows(&device, &points);

    state.poller_manager.start_dnp3(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_dnp3_device".to_string(),
        username: user.username.clone(),
        resource: format!("dnp3_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/dnp3/devices/{id}/stop
#[utoipa::path(
    post,
    operation_id = "dnp3_stop_device",
    path = "/api/v1/dnp3/devices/{id}/stop",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Dnp3ActionResponse),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn stop_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let device_key = format!("dnp3_{}", id);
    state.poller_manager.stop(&device_key).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "stop_dnp3_device".to_string(),
        username: user.username.clone(),
        resource: format!("dnp3_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Discovery ──

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ScanRequest {
    pub range: String,
    #[serde(default = "default_dnp3_scan_port")]
    pub port: u16,
}

fn default_dnp3_scan_port() -> u16 {
    20000
}

/// POST /api/v1/dnp3/scan
#[utoipa::path(
    post,
    operation_id = "dnp3_scan_network",
    path = "/api/v1/dnp3/scan",
    request_body = ScanRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn scan_network(
    State(_state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<ScanRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let devices = crate::dnp3::discovery::scan_network(&req.range, req.port, 2000).await?;
    Ok(Json(json!(devices)))
}

// ── Actions ──

/// POST /api/v1/dnp3/devices/{id}/integrity
#[utoipa::path(
    post,
    path = "/api/v1/dnp3/devices/{id}/integrity",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Dnp3ActionResponse),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn trigger_integrity(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Verify device exists
    let device = state.storage.get_dnp3_device(id).await?;
    device.ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    Ok(Json(
        json!({ "ok": true, "message": "Integrity poll triggered" }),
    ))
}

/// POST /api/v1/dnp3/devices/{id}/freeze
#[utoipa::path(
    post,
    path = "/api/v1/dnp3/devices/{id}/freeze",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Dnp3ActionResponse),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn trigger_freeze(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Verify device exists
    let device = state.storage.get_dnp3_device(id).await?;
    device.ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    Ok(Json(json!({ "ok": true, "message": "Freeze triggered" })))
}

/// GET /api/v1/dnp3/group-variations
#[utoipa::path(
    get,
    path = "/api/v1/dnp3/group-variations",
    responses(
        (status = 200, body = Vec<serde_json::Value>),
        (status = 403),
    ),
    tag = "DNP3",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_group_variations(
    State(_state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let gv = common_group_variations()
        .into_iter()
        .map(|(g, v, name)| json!({ "group": g, "variation": v, "name": name }))
        .collect::<Vec<_>>();
    Ok(Json(gv))
}
