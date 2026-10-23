use anyhow::Context;
use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::models::{Endianness, ModbusRegisterInput, ScanRegisterRequest};

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ScanNetworkRequest {
    pub range: String,
    #[serde(default = "default_scan_port")]
    pub port: u16,
}

fn default_scan_port() -> u16 {
    502
}

/// GET /api/v1/modbus-devices
#[utoipa::path(
    get,
    path = "/api/v1/modbus-devices",
    responses(
        (status = 200, body = Vec<serde_json::Value>),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_modbus_devices(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let devices = state.storage.list_modbus_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let registers = state.storage.list_modbus_registers(&device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "ip": device.ip,
            "port": device.port,
            "unit_id": device.unit_id,
            "interval_ms": device.interval_ms,
            "endianness": device.endianness,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "updated_at": device.updated_at,
            "register_count": registers.len(),
        }));
    }
    Ok(Json(result))
}

/// GET /api/v1/modbus-devices/{id}
#[utoipa::path(
    get,
    path = "/api/v1/modbus-devices/{id}",
    tag = "Devices",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, description = "Device with its registers", body = serde_json::Value),
        (status = 404, description = "Device not found"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_modbus_device(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let device = state.storage.get_modbus_device(&id).await?;
    let device = device.context("Device not found")?;
    let registers = state.storage.list_modbus_registers(&id).await?;
    Ok(Json(json!({
        "device": device,
        "registers": registers,
    })))
}

/// POST /api/v1/modbus-devices
#[utoipa::path(
    post,
    path = "/api/v1/modbus-devices",
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_modbus_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let id = body["id"].as_str().unwrap_or("").to_string();
    let name = body["name"].as_str().unwrap_or("").to_string();
    let ip = body["ip"].as_str().unwrap_or("").to_string();
    let port = body["port"].as_u64().unwrap_or(502) as u16;
    let unit_id = body["unit_id"].as_u64().unwrap_or(1) as u8;
    let interval_ms = body["interval_ms"].as_u64().unwrap_or(1000);
    let endianness = body["endianness"].as_str().unwrap_or("big").to_string();

    if id.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device ID is required")));
    }
    if ip.is_empty() {
        return Err(AppError(anyhow::anyhow!("IP address is required")));
    }

    let registers =
        ModbusRegisterInput::parse_all(&body).map_err(|e| AppError(anyhow::anyhow!(e)))?;

    state
        .storage
        .create_modbus_device(
            &id,
            &name,
            &ip,
            port,
            unit_id,
            interval_ms,
            &endianness,
            &registers,
        )
        .await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "modbus",
        &id,
        &name,
        Some(&ip),
        "plc",
    )
    .await;

    // Start poller
    let config = state.storage.get_all_device_configs().await?;
    if let Some(dc) = config.iter().find(|c| c.id == id) {
        state.poller_manager.start(dc.clone()).await;
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_modbus_device".to_string(),
        username: auth.username.clone(),
        resource: format!("modbus_device:{}", id),
        details: Some(format!("ip={}:{}", ip, port)),
    });

    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({"ok": true, "id": id})),
    ))
}

/// PUT /api/v1/modbus-devices/{id}
#[utoipa::path(
    put,
    path = "/api/v1/modbus-devices/{id}",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_modbus_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.update_modbus_device(&id, &body).await?;

    // If registers array is present, replace registers too
    if body.get("registers").and_then(|v| v.as_array()).is_some() {
        let registers =
            ModbusRegisterInput::parse_all(&body).map_err(|e| AppError(anyhow::anyhow!(e)))?;

        state
            .storage
            .replace_modbus_registers(&id, &registers)
            .await?;
    }

    // Restart poller with updated config
    let configs = state.storage.get_all_device_configs().await?;
    if let Some(dc) = configs.iter().find(|c| c.id == id) {
        state.poller_manager.restart(dc.clone()).await;
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_modbus_device".to_string(),
        username: auth.username.clone(),
        resource: format!("modbus_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/v1/modbus-devices/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/modbus-devices/{id}",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_modbus_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.delete_modbus_device(&id).await?;
    state.poller_manager.stop(&id).await;
    state.cache.remove(&id).await;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "modbus", &id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_modbus_device".to_string(),
        username: auth.username.clone(),
        resource: format!("modbus_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// PUT /api/v1/modbus-devices/{id}/registers
#[utoipa::path(
    put,
    path = "/api/v1/modbus-devices/{id}/registers",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn replace_registers(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let registers =
        ModbusRegisterInput::parse_all(&body).map_err(|e| AppError(anyhow::anyhow!(e)))?;

    state
        .storage
        .replace_modbus_registers(&id, &registers)
        .await?;

    // Restart poller with updated registers
    let configs = state.storage.get_all_device_configs().await?;
    if let Some(dc) = configs.iter().find(|c| c.id == id) {
        state.poller_manager.restart(dc.clone()).await;
    }

    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/modbus-devices/{id}/restart
#[utoipa::path(
    post,
    path = "/api/v1/modbus-devices/{id}/restart",
    params(("id" = String, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn restart_poller(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let configs = state.storage.get_all_device_configs().await?;
    let dc = match configs.iter().find(|c| c.id == id) {
        Some(c) => c,
        None => return Err(AppError(anyhow::anyhow!("Device not found"))),
    };
    state.poller_manager.restart(dc.clone()).await;
    Ok(Json(json!({"ok": true})))
}

/// GET /api/v1/modbus-devices/status
#[utoipa::path(
    get,
    path = "/api/v1/modbus-devices/status",
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_poller_statuses(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let statuses = state.poller_manager.get_statuses().await;
    Ok(Json(statuses))
}

/// POST /api/v1/modbus-devices/scan
#[utoipa::path(
    post,
    path = "/api/v1/modbus-devices/scan",
    request_body = ScanRegisterRequest,
    responses(
        (status = 200, body = Vec<serde_json::Value>),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn scan_modbus_registers(
    Json(body): Json<ScanRegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let endianness = match body.endianness.as_str() {
        "little" => Endianness::Little,
        "swapped" => Endianness::Swapped,
        _ => Endianness::Big,
    };
    let results = crate::modbus::scan_registers(
        &body.ip,
        body.port,
        body.unit_id,
        body.start_addr,
        body.end_addr,
        &endianness,
        &body.register_area,
        body.scan_delay_ms,
    )
    .await?;
    Ok(Json(results))
}

/// POST /api/v1/modbus-devices/scan-network
/// Scan a subnet for Modbus TCP devices (Starter+ license required).
#[utoipa::path(
    post,
    operation_id = "modbus_scan_network",
    path = "/api/v1/modbus-devices/scan-network",
    request_body = ScanNetworkRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]

pub async fn scan_network(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<ScanNetworkRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.range.is_empty() {
        return Err(AppError(anyhow::anyhow!("Range (CIDR) is required")));
    }

    let devices = crate::discovery::scan_network(&body.range, body.port, 500).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "scan_network".to_string(),
        username: auth.username.clone(),
        resource: format!("range:{}", body.range),
        details: Some(format!("found {} device(s)", devices.len())),
    });

    Ok(Json(json!({
        "devices": devices,
        "count": devices.len(),
    })))
}
