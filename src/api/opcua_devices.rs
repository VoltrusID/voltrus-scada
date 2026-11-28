use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::models::*;
use crate::opcua::{PKI_DIR, ServerTarget, browse_objects};

// ── Device CRUD ──

/// GET /api/v1/opcua/devices
#[utoipa::path(
    get,
    operation_id = "opcua_list_devices",
    path = "/api/v1/opcua/devices",
    responses(
        (status = 200, body = Vec<serde_json::Value>),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let devices = state.storage.list_opcua_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let nodes = state.storage.list_opcua_nodes(device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "url": device.url,
            "security_policy": device.security_policy,
            "security_mode": device.security_mode,
            "username": device.username,
            "interval_ms": device.interval_ms,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "node_count": nodes.len(),
        }));
    }
    Ok(Json(result))
}

/// POST /api/v1/opcua/devices
#[utoipa::path(
    post,
    operation_id = "opcua_create_device",
    path = "/api/v1/opcua/devices",
    request_body = CreateOpcUaDeviceRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateOpcUaDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device name is required")));
    }
    if req.url.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device URL is required")));
    }
    if !req.url.starts_with("opc.tcp://") {
        return Err(AppError(anyhow::anyhow!(
            "Device URL must start with opc.tcp://"
        )));
    }

    super::check_poll_interval(req.interval_ms)?;
    super::check_device_name_free(&state.storage, "opcua", &req.name, None).await?;

    let id = state
        .storage
        .create_opcua_device(
            &req.name,
            &req.url,
            &req.security_policy,
            &req.security_mode,
            req.username.as_deref(),
            req.password.as_deref(),
            req.interval_ms,
            req.enabled,
        )
        .await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "opcua",
        &id.to_string(),
        &req.name,
        None,
        "gateway",
    )
    .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: Some(format!("name={}, url={}", req.name, req.url)),
    });

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// PUT /api/v1/opcua/devices/{id}
#[utoipa::path(
    put,
    operation_id = "opcua_update_device",
    path = "/api/v1/opcua/devices/{id}",
    params(("id" = i64, Path, description = "Device ID")),
    request_body = UpdateOpcUaDeviceRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateOpcUaDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if let Some(ref url) = req.url
        && !url.is_empty()
        && !url.starts_with("opc.tcp://")
    {
        return Err(AppError(anyhow::anyhow!(
            "Device URL must start with opc.tcp://"
        )));
    }

    if let Some(interval_ms) = req.interval_ms {
        super::check_poll_interval(interval_ms)?;
    }

    if let Some(name) = req.name.as_deref() {
        super::check_device_name_free(&state.storage, "opcua", name, Some(id)).await?;
    }
    state
        .storage
        .update_opcua_device(
            id,
            req.name.as_deref(),
            req.url.as_deref(),
            req.security_policy.as_ref(),
            req.security_mode.as_ref(),
            req.username.as_ref().map(|o| o.as_deref()),
            req.password.as_ref().map(|o| o.as_deref()),
            req.interval_ms,
            req.enabled,
        )
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/v1/opcua/devices/{id}
#[utoipa::path(
    delete,
    operation_id = "opcua_delete_device",
    path = "/api/v1/opcua/devices/{id}",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
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

    // Stop poller if running
    let device_key = format!("opcua_{}", id);
    state.poller_manager.stop(&device_key).await;
    state.cache.remove(&device_key).await;

    state.storage.delete_opcua_device(id).await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "opcua", &id.to_string()).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Node CRUD ──

/// GET /api/v1/opcua/devices/{id}/nodes
#[utoipa::path(
    get,
    path = "/api/v1/opcua/devices/{id}/nodes",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = Vec<OpcUaNode>),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let nodes = state.storage.list_opcua_nodes(id).await?;
    Ok(Json(nodes))
}

/// POST /api/v1/opcua/devices/{id}/nodes
#[utoipa::path(
    post,
    path = "/api/v1/opcua/devices/{id}/nodes",
    params(("id" = i64, Path, description = "Device ID")),
    request_body = CreateOpcUaNodeRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_node(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(device_id): Path<i64>,
    Json(req): Json<CreateOpcUaNodeRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if req.node_id.is_empty() {
        return Err(AppError(anyhow::anyhow!("Node ID is required")));
    }
    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Node name is required")));
    }

    // Look up device name for data source key generation
    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let id = state
        .storage
        .create_opcua_node(
            device_id,
            &device.name,
            &req.node_id,
            &req.name,
            &req.display_name,
            &req.data_type,
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError(anyhow::anyhow!("Node already exists on this device"))
            } else {
                AppError(e)
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// DELETE /api/v1/opcua/devices/{id}/nodes/{node_id}
#[utoipa::path(
    delete,
    path = "/api/v1/opcua/devices/{id}/nodes/{node_id}",
    params(
        ("id" = i64, Path, description = "Device ID"),
        ("node_id" = i64, Path, description = "Node ID"),
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_node(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_device_id, node_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    state.storage.delete_opcua_node(node_id).await?;

    Ok(Json(json!({"ok": true})))
}

// ── Browse ──

/// POST /api/v1/opcua/devices/{id}/browse
///
/// Connects to the OPC-UA server using the device's stored config and returns
/// the objects and variables in the ObjectsFolder. The server certificate is
/// checked as the poller checks it, and the session is closed afterwards.
#[utoipa::path(
    post,
    operation_id = "opcua_browse_device",
    path = "/api/v1/opcua/devices/{id}/browse",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 404),
        (status = 500, description = "Connection or browse failed; `error` says why, e.g. an untrusted server certificate and where it was saved"),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn browse_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError::not_found("Device not found"))?;

    // A short-lived session with the poller's certificate trust: a secure
    // endpoint whose certificate is not in pki/trusted is refused, and the
    // error says where the refused certificate went.
    let nodes = browse_objects(&ServerTarget::from(device), std::path::Path::new(PKI_DIR)).await?;
    let nodes: Vec<_> = nodes
        .into_iter()
        .map(|n| {
            json!({
                "node_id": n.node_id,
                "browse_name": n.browse_name,
                "display_name": n.display_name,
                "node_class": n.node_class,
            })
        })
        .collect();

    Ok(Json(json!({
        "nodes": nodes,
        "count": nodes.len(),
    })))
}

// ── Start / Stop ──

/// POST /api/v1/opcua/devices/{id}/start
///
/// Starts the OPC-UA poller for the device. Converts the DB-stored device + nodes
/// into an OpcUaDeviceConfig and starts the poller.
/// POST /api/v1/opcua/devices/{id}/start
#[utoipa::path(
    post,
    operation_id = "opcua_start_device",
    path = "/api/v1/opcua/devices/{id}/start",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
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
    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!(
            "Device is disabled. Enable it first before starting."
        )));
    }

    // Load nodes for device
    let nodes = state.storage.list_opcua_nodes(device.id).await?;

    if nodes.is_empty() {
        return Err(AppError(anyhow::anyhow!(
            "Device has no nodes configured. Add nodes before starting."
        )));
    }

    // Convert to runtime config
    let device_key = format!("opcua_{}", device.id);
    let config = OpcUaDeviceConfig {
        id: device_key.clone(),
        url: device.url.clone(),
        security_policy: device.security_policy.clone(),
        security_mode: device.security_mode.clone(),
        username: device.username.clone(),
        password: device.password.clone(),
        interval_ms: device.interval_ms,
        nodes: nodes
            .iter()
            .map(|n| OpcUaNodeConfig {
                node_id: n.node_id.clone(),
                name: n.name.clone(),
                display_name: n.display_name.clone(),
                data_type: n.data_type.clone(),
            })
            .collect(),
    };

    state.poller_manager.start_opcua(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/opcua/devices/{id}/stop
#[utoipa::path(
    post,
    operation_id = "opcua_stop_device",
    path = "/api/v1/opcua/devices/{id}/stop",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
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

    let device_key = format!("opcua_{}", id);
    state.poller_manager.stop(&device_key).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "stop_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}
