use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::models::*;
use crate::s7::{S7Client, S7DeviceConfig};

// ── Device CRUD ──

/// GET /api/v1/s7/devices
#[utoipa::path(
    get,
    operation_id = "s7_list_devices",
    path = "/api/v1/s7/devices",
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
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let devices = state.storage.list_s7_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let tags = state.storage.list_s7_tags(device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "url": device.url,
            "rack": device.rack,
            "slot": device.slot,
            "interval_ms": device.interval_ms,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "tag_count": tags.len(),
        }));
    }
    Ok(Json(result))
}

/// POST /api/v1/s7/devices
#[utoipa::path(
    post,
    operation_id = "s7_create_device",
    path = "/api/v1/s7/devices",
    request_body = CreateS7DeviceRequest,
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
    Json(req): Json<CreateS7DeviceRequest>,
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
        .create_s7_device(
            &req.name,
            &req.url,
            req.rack,
            req.slot,
            req.interval_ms,
            req.enabled,
        )
        .await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "s7",
        &id.to_string(),
        &req.name,
        None,
        "plc",
    )
    .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_s7_device".to_string(),
        username: user.username.clone(),
        resource: format!("s7_device:{}", id),
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

/// PUT /api/v1/s7/devices/{id}
#[utoipa::path(
    put,
    operation_id = "s7_update_device",
    path = "/api/v1/s7/devices/{id}",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    request_body = UpdateS7DeviceRequest,
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
    Json(req): Json<UpdateS7DeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    state
        .storage
        .update_s7_device(
            id,
            req.name.as_deref(),
            req.url.as_deref(),
            req.rack,
            req.slot,
            req.interval_ms,
            req.enabled,
        )
        .await?;

    // Restart poller if running
    let device_key = format!("s7_{}", id);
    if state.poller_manager.is_running(&device_key).await
        && let Ok(Some(device)) = state.storage.get_s7_device(id).await
    {
        let tags = state.storage.list_s7_tags(id).await?;
        let mut s7_tags = Vec::new();
        for tag_row in &tags {
            match S7Client::parse_tag_address(&tag_row.tag_address) {
                Ok(mut s7_tag) => {
                    s7_tag.name = Some(tag_row.name.clone());
                    s7_tags.push(s7_tag)
                }
                Err(_) => continue,
            }
        }
        let config = S7DeviceConfig {
            id: device_key.clone(),
            url: device.url,
            rack: device.rack,
            slot: device.slot,
            interval_ms: device.interval_ms,
            tags: s7_tags,
        };
        state.poller_manager.start_s7(config).await;
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_s7_device".to_string(),
        username: user.username.clone(),
        resource: format!("s7_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/v1/s7/devices/{id}
#[utoipa::path(
    delete,
    operation_id = "s7_delete_device",
    path = "/api/v1/s7/devices/{id}",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
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
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    // Stop poller if running
    let device_key = format!("s7_{}", id);
    state.poller_manager.stop(&device_key).await;
    state.cache.remove(&device_key).await;

    state.storage.delete_s7_device(id).await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "s7", &id.to_string()).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_s7_device".to_string(),
        username: user.username.clone(),
        resource: format!("s7_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Tag CRUD ──

/// GET /api/v1/s7/devices/{id}/tags
#[utoipa::path(
    get,
    operation_id = "s7_list_tags",
    path = "/api/v1/s7/devices/{id}/tags",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Vec<S7TagRow>),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_tags(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let tags = state.storage.list_s7_tags(id).await?;
    Ok(Json(tags))
}

/// POST /api/v1/s7/devices/{id}/tags
#[utoipa::path(
    post,
    operation_id = "s7_add_tag",
    path = "/api/v1/s7/devices/{id}/tags",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    request_body = CreateS7TagRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 404),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(device_id): Path<i64>,
    Json(req): Json<CreateS7TagRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    if req.tag_address.is_empty() {
        return Err(AppError(anyhow::anyhow!("Tag address is required")));
    }
    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Tag name is required")));
    }

    // Validate tag address can be parsed
    if let Err(e) = S7Client::parse_tag_address(&req.tag_address) {
        return Err(AppError(anyhow::anyhow!(
            "Invalid S7 tag address '{}': {}",
            req.tag_address,
            e
        )));
    }

    // Look up device name for data source key generation
    let device = state
        .storage
        .get_s7_device(device_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let id = state
        .storage
        .create_s7_tag(
            device_id,
            &device.name,
            &req.tag_address,
            &req.name,
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
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// DELETE /api/v1/s7/devices/{id}/tags/{tag_id}
#[utoipa::path(
    delete,
    operation_id = "s7_remove_tag",
    path = "/api/v1/s7/devices/{id}/tags/{tag_id}",
    params(
        ("id" = i64, Path, description = "Device ID"),
        ("tag_id" = i64, Path, description = "Tag ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_tag(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_device_id, tag_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    state.storage.delete_s7_tag(tag_id).await?;

    Ok(Json(json!({"ok": true})))
}

// ── Start / Stop ──

/// POST /api/v1/s7/devices/{id}/start
#[utoipa::path(
    post,
    operation_id = "s7_start_device",
    path = "/api/v1/s7/devices/{id}/start",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 404),
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
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    // Load device from DB
    let device = state
        .storage
        .get_s7_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!(
            "Device is disabled. Enable it first before starting."
        )));
    }

    // Load tags for device
    let tags = state.storage.list_s7_tags(device.id).await?;

    if tags.is_empty() {
        return Err(AppError(anyhow::anyhow!(
            "Device has no tags configured. Add tags before starting."
        )));
    }

    // Convert to runtime config
    let device_key = format!("s7_{}", device.id);
    let mut s7_tags = Vec::new();
    for tag_row in &tags {
        match S7Client::parse_tag_address(&tag_row.tag_address) {
            Ok(mut s7_tag) => {
                s7_tag.name = Some(tag_row.name.clone());
                s7_tags.push(s7_tag)
            }
            Err(e) => {
                return Err(AppError(anyhow::anyhow!(
                    "Failed to parse tag '{}': {}",
                    tag_row.tag_address,
                    e
                )));
            }
        }
    }

    let config = S7DeviceConfig {
        id: device_key.clone(),
        url: device.url,
        rack: device.rack,
        slot: device.slot,
        interval_ms: device.interval_ms,
        tags: s7_tags,
    };

    state.poller_manager.start_s7(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_s7_device".to_string(),
        username: user.username.clone(),
        resource: format!("s7_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/s7/devices/{id}/stop
#[utoipa::path(
    post,
    operation_id = "s7_stop_device",
    path = "/api/v1/s7/devices/{id}/stop",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
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
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let device_key = format!("s7_{}", id);
    state.poller_manager.stop(&device_key).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "stop_s7_device".to_string(),
        username: user.username.clone(),
        resource: format!("s7_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Browse ──

/// POST /api/v1/s7/devices/{id}/browse
///
/// Lists common DB blocks and tag address patterns for the S7 device.
/// Since S7 doesn't have a standard browse protocol like OPC-UA,
/// we attempt to connect and list known DB areas.
#[utoipa::path(
    post,
    operation_id = "s7_browse_device",
    path = "/api/v1/s7/devices/{id}/browse",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
        (status = 404),
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
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    // Load device from DB
    let device = state
        .storage
        .get_s7_device(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    // Attempt to connect and list common areas
    let mut client = S7Client::new(&device.url, device.rack, device.slot);

    let connected = client.connect().await.is_ok();
    let _ = client.disconnect().await;

    // Provide common S7 address patterns as browse results
    let areas = vec![
        json!({
            "area": "DB",
            "description": "Data Block",
            "address_format": "DB<n>.DBX<byte>.<bit> (Bool), DB<n>.DBB<byte> (Byte), DB<n>.DBW<byte> (Word), DB<n>.DBD<byte> (DWord)",
            "examples": ["DB1.DBX0.0", "DB1.DBB10", "DB1.DBW100", "DB1.DBD200"]
        }),
        json!({
            "area": "I",
            "description": "Inputs",
            "address_format": "I<byte>.<bit> (Bool), IB<byte> (Byte), IW<byte> (Word), ID<byte> (DWord)",
            "examples": ["I0.0", "IB0", "IW100"]
        }),
        json!({
            "area": "Q",
            "description": "Outputs",
            "address_format": "Q<byte>.<bit> (Bool), QB<byte> (Byte), QW<byte> (Word), QD<byte> (DWord)",
            "examples": ["Q0.0", "QB0", "QW200"]
        }),
        json!({
            "area": "M",
            "description": "Markers (Memory)",
            "address_format": "M<byte>.<bit> (Bool), MB<byte> (Byte), MW<byte> (Word), MD<byte> (DWord)",
            "examples": ["M0.0", "MB10", "MW100", "MD200"]
        }),
        json!({
            "area": "T",
            "description": "Timers",
            "address_format": "T<timer_number>",
            "examples": ["T0", "T1", "T10"]
        }),
        json!({
            "area": "C",
            "description": "Counters",
            "address_format": "C<counter_number>",
            "examples": ["C0", "C1", "C10"]
        }),
    ];

    Ok(Json(json!({
        "connected": connected,
        "url": device.url,
        "rack": device.rack,
        "slot": device.slot,
        "areas": areas,
        "count": areas.len(),
    })))
}
