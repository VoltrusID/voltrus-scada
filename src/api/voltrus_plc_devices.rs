use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::voltrus_plc::{
    CreateVoltrusPlcDeviceRequest, CreateVoltrusPlcTagRequest, UpdateVoltrusPlcDeviceRequest,
    VoltrusPlcDeviceConfig, write_tag,
};

// ── Device CRUD ──

/// GET /api/v1/voltrus-plc/devices
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let devices = state.storage.list_voltrus_plc_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let tags = state.storage.list_voltrus_plc_tags(device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "url": device.url,
            "transport": device.transport,
            "interval_ms": device.interval_ms,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "tag_count": tags.len(),
        }));
    }
    Ok(Json(result))
}

/// POST /api/v1/voltrus-plc/devices
pub async fn create_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateVoltrusPlcDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device name is required")));
    }
    if req.url.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device URL is required")));
    }

    let id = state
        .storage
        .create_voltrus_plc_device(
            &req.name,
            &req.url,
            &req.transport,
            req.interval_ms,
            req.enabled,
        )
        .await?;

    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "voltrus-plc",
        &id.to_string(),
        &req.name,
        None,
        "plc",
    )
    .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: Some(format!(
            "name={}, url={}, transport={}",
            req.name, req.url, req.transport
        )),
    });

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "name": req.name, "url": req.url, "transport": req.transport})),
    ))
}

/// GET /api/v1/voltrus-plc/devices/:id
pub async fn get_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device = state
        .storage
        .get_voltrus_plc_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let tags = state.storage.list_voltrus_plc_tags(id).await?;
    Ok(Json(json!({
        "device": device,
        "tags": tags,
    })))
}

/// PUT /api/v1/voltrus-plc/devices/:id
pub async fn update_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateVoltrusPlcDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    state
        .storage
        .update_voltrus_plc_device(
            id,
            req.name.as_deref(),
            req.url.as_deref(),
            req.transport.as_deref(),
            req.interval_ms,
            req.enabled,
        )
        .await?;

    // Restart poller if running with updated config
    let device_id = id.to_string();
    if state.poller_manager.is_running(&device_id).await
        && let Ok(Some(device)) = state.storage.get_voltrus_plc_device(id).await
    {
        if device.enabled {
            let tags = state.storage.list_voltrus_plc_tags(id).await?;
            let config = VoltrusPlcDeviceConfig {
                id: device_id,
                url: device.url,
                transport: device.transport,
                interval_ms: device.interval_ms,
                tags,
            };
            state.poller_manager.start_voltrus_plc(config).await;
        } else {
            state.poller_manager.stop(&id.to_string()).await;
        }
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"success": true})))
}

/// DELETE /api/v1/voltrus-plc/devices/:id
pub async fn delete_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device_id = id.to_string();
    state.poller_manager.stop(&device_id).await;
    state.cache.remove(&device_id).await;
    state.storage.delete_voltrus_plc_device(id).await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "voltrus-plc", &id.to_string())
        .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"success": true})))
}

// ── Tag CRUD ──

/// GET /api/v1/voltrus-plc/devices/:id/tags
pub async fn list_tags(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let tags = state.storage.list_voltrus_plc_tags(id).await?;
    Ok(Json(tags))
}

/// POST /api/v1/voltrus-plc/devices/:id/tags
pub async fn create_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(device_id): Path<i64>,
    Json(req): Json<CreateVoltrusPlcTagRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    if req.variable_name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Variable name is required")));
    }

    // Validate variable name — no path traversal or injection characters
    if req
        .variable_name
        .contains(|c: char| c.is_whitespace() || c == '/' || c == '\\' || c == '\0' || c == '&')
    {
        return Err(AppError(anyhow::anyhow!(
            "Variable name contains invalid characters"
        )));
    }

    // Look up device name for data source key generation
    let device = state
        .storage
        .get_voltrus_plc_device(device_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let id = state
        .storage
        .create_voltrus_plc_tag(
            device_id,
            &device.name,
            &req.variable_name,
            &req.display_name,
            &req.data_type,
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError(anyhow::anyhow!("Tag already exists on this device"))
            } else {
                AppError(e)
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"id": id, "variable_name": req.variable_name})),
    ))
}

/// DELETE /api/v1/voltrus-plc/devices/:device_id/tags/:tag_id
pub async fn delete_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_device_id, tag_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    state.storage.delete_voltrus_plc_tag(tag_id).await?;
    Ok(Json(json!({"success": true})))
}

// ── Poller control ──

/// POST /api/v1/voltrus-plc/devices/:id/start
pub async fn start_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device = state
        .storage
        .get_voltrus_plc_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!("Device is disabled")));
    }

    let tags = state.storage.list_voltrus_plc_tags(id).await?;
    let config = VoltrusPlcDeviceConfig {
        id: device.id.to_string(),
        url: device.url,
        transport: device.transport,
        interval_ms: device.interval_ms,
        tags,
    };

    state.poller_manager.start_voltrus_plc(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"success": true, "device_id": id})))
}

/// POST /api/v1/voltrus-plc/devices/:id/stop
pub async fn stop_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device_id = id.to_string();
    state.poller_manager.stop(&device_id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "stop_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"success": true, "device_id": id})))
}

/// POST /api/v1/voltrus-plc/devices/:id/restart
pub async fn restart_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device_id = id.to_string();
    state.poller_manager.stop(&device_id).await;

    let device = state
        .storage
        .get_voltrus_plc_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!("Device is disabled")));
    }

    let tags = state.storage.list_voltrus_plc_tags(id).await?;
    let config = VoltrusPlcDeviceConfig {
        id: device.id.to_string(),
        url: device.url,
        transport: device.transport,
        interval_ms: device.interval_ms,
        tags,
    };

    state.poller_manager.start_voltrus_plc(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "restart_voltrus_plc_device".to_string(),
        username: user.username.clone(),
        resource: format!("voltrus_plc_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"success": true, "device_id": id})))
}

/// POST /api/v1/voltrus-plc/devices/:id/write
pub async fn write_variable(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device = state
        .storage
        .get_voltrus_plc_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let variable_name = body["variable"]
        .as_str()
        .ok_or_else(|| AppError(anyhow::anyhow!("'variable' field is required")))?;
    let value = body["value"]
        .as_f64()
        .ok_or_else(|| AppError(anyhow::anyhow!("'value' field must be a number")))?;

    let result = write_tag(&device.url, &device.transport, variable_name, value).await;
    Ok(Json(json!({
        "success": result.success,
        "device_response": result.device_response,
        "error": result.error,
        "latency_ms": result.latency_ms,
    })))
}
