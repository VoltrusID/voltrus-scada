use std::sync::LazyLock;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::asset::models::{CreateAssetRequest, SnmpScanRequest};
use crate::asset::snmp::{self, ScanState};

/// Global scan state shared across all SNMP scan API calls.
static SCAN_STATE: LazyLock<ScanState> = LazyLock::new(ScanState::new);

// ── Start Scan ──

#[utoipa::path(
    post,
    path = "/api/v1/snmp/scan",
    request_body = SnmpScanRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 409),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn start_scan(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<SnmpScanRequest>,
) -> Result<impl IntoResponse, AppError> {
    if SCAN_STATE.is_scanning() {
        return Err(AppError(anyhow::anyhow!("SNMP scan already in progress")));
    }

    if body.subnet.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Subnet (CIDR) is required")));
    }

    // Clear previous scan results
    state.storage.asset().clear_scan_results().await?;

    // Spawn background scan task
    let subnet = body.subnet.clone();
    let community = body.community.clone();
    let timeout_ms = body.timeout_ms;
    let scan_state = SCAN_STATE.clone();
    let asset_storage = state.storage.asset();

    tokio::spawn(async move {
        match snmp::run_scan(&subnet, &community, timeout_ms, &scan_state).await {
            Ok(results) => {
                for r in &results {
                    if let Err(e) = asset_storage.insert_scan_result(r).await {
                        tracing::warn!("Failed to persist scan result for {}: {}", r.ip, e);
                    }
                }
                tracing::info!("SNMP scan complete: {} device(s) found", results.len());
            }
            Err(e) => {
                tracing::error!("SNMP scan failed: {}", e);
            }
        }
    });

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_snmp_scan".to_string(),
        username: auth.username.clone(),
        resource: format!("subnet:{}", body.subnet),
        details: None,
    });

    Ok(Json(json!({
        "ok": true,
        "message": "SNMP scan started",
        "subnet": body.subnet,
    })))
}

// ── Get Scan Status ──

#[utoipa::path(
    get,
    path = "/api/v1/snmp/scan/status",
    responses(
        (status = 200, body = serde_json::Value)
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_scan_status(
    State(_state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let (progress, total) = SCAN_STATE.get_progress();
    Ok(Json(json!({
        "scanning": SCAN_STATE.is_scanning(),
        "progress": progress,
        "total": total,
    })))
}

// ── List Scan Results ──

#[utoipa::path(
    get,
    path = "/api/v1/snmp/scan/results",
    responses(
        (status = 200, body = Vec<crate::asset::models::SnmpScanResult>)
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_scan_results(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let results = state.storage.asset().list_scan_results().await?;
    Ok(Json(results))
}

// ── Import Scanned Device ──

#[utoipa::path(
    post,
    path = "/api/v1/snmp/scan/{ip}/import",
    params(("ip" = String, Path, description = "IP address of scanned device")),
    responses(
        (status = 201, body = crate::asset::models::Asset),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn import_scanned_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(ip): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let result = state
        .storage
        .asset()
        .get_scan_result(&ip)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("No scan result for IP {}", ip)))?;

    let asset_type = detect_asset_type(result.vendor.as_deref(), result.model.as_deref());

    let req = CreateAssetRequest {
        name: result
            .sys_name
            .clone()
            .unwrap_or_else(|| format!("Device {}", ip)),
        asset_type,
        vendor: result.vendor.clone(),
        model: result.model.clone(),
        firmware_version: result.firmware.clone(),
        serial_number: None,
        ip_address: Some(ip.clone()),
        mac_address: result.mac_address.clone(),
        location: None,
        criticality: "medium".to_string(),
        tags: vec!["snmp-discovered".to_string()],
        notes: result.sys_descr.clone(),
    };

    let asset = state.storage.asset().create_asset(&req).await?;

    // Recompute risk score
    let asset_storage = state.storage.asset();
    let _ = crate::asset::risk::recompute_and_update(&asset_storage, &asset.id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "import_snmp_device".to_string(),
        username: auth.username.clone(),
        resource: format!("asset:{}", asset.id),
        details: Some(format!("ip={}, name={}", ip, asset.name)),
    });

    Ok((axum::http::StatusCode::CREATED, Json(asset)))
}

/// Auto-detect asset_type from vendor and model info.
fn detect_asset_type(vendor: Option<&str>, model: Option<&str>) -> String {
    let v = vendor.map(|s| s.to_lowercase()).unwrap_or_default();
    let m = model.map(|s| s.to_lowercase()).unwrap_or_default();

    if v.contains("siemens") || m.contains("s7") || m.contains("simatic") {
        return "plc".to_string();
    }
    if v.contains("rockwell") || v.contains("allen-bradley") || m.contains("logix") {
        return "plc".to_string();
    }
    if v.contains("schneider") || v.contains("modicon") {
        return "plc".to_string();
    }
    if v.contains("cisco") || v.contains("juniper") {
        return "switch".to_string();
    }
    if m.contains("sensor") {
        return "sensor".to_string();
    }
    if m.contains("drive") || m.contains("vfd") {
        return "vfd".to_string();
    }

    "other".to_string()
}
