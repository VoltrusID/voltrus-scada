use super::system_sampler::SystemSampler;
use axum::Json;
use axum::extract::State;

use super::{AppError, AppState};
use crate::models::*;

#[utoipa::path(
    get,
    path = "/api/v1/version",
    responses((status = 200, description = "Version info")),
    tag = "System",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/live",
    responses((status = 200, body = crate::models::LiveResponse)),
    tag = "System",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_live(State(state): State<AppState>) -> Json<LiveResponse> {
    let devices = state.cache.get_all().await;
    Json(LiveResponse { devices })
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses((status = 200, description = "Health check", body = crate::models::HealthResponse)),
    tag = "System"
)]
pub async fn get_health(State(state): State<AppState>) -> Json<HealthResponse> {
    let statuses = state.cache.get_statuses().await;
    Json(HealthResponse {
        status: "ok".to_string(),
        devices: statuses,
    })
}

pub async fn compute_system_info(state: &AppState) -> Result<SystemResponse, AppError> {
    compute_system_info_from(SystemSampler::global(), state).await
}

/// [`compute_system_info`] over a given sampler, so tests can count refreshes.
pub(super) async fn compute_system_info_from(
    sampler: &SystemSampler,
    state: &AppState,
) -> Result<SystemResponse, AppError> {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let round1 = |v: f64| (v * 10.0).round() / 10.0;

    let host = sampler.sample().await?;

    let mem_total = host.memory_total_bytes as f64 / MB;
    let mem_used = host.memory_used_bytes as f64 / MB;
    let mem_percent = if mem_total > 0.0 {
        (mem_used / mem_total) * 100.0
    } else {
        0.0
    };

    let (disk_total_gb, disk_used_gb, disk_percent) = match host.disk {
        Some(d) => {
            let total = d.total_bytes as f64 / GB;
            let used = total - d.available_bytes as f64 / GB;
            let pct = if total > 0.0 {
                (used / total) * 100.0
            } else {
                0.0
            };
            (total, used, pct)
        }
        None => (0.0, 0.0, 0.0),
    };

    let db_size_mb = std::fs::metadata(state.storage.db_path())
        .map(|m| m.len() as f64 / MB)
        .unwrap_or(0.0);

    let proc = &host.process;
    let process_info = ProcessInfo {
        pid: proc.pid,
        memory_mb: proc.rss_bytes.unwrap_or(0) as f64 / MB,
        cpu_percent: proc.cpu_percent,
        uptime_secs: match proc.start_time_secs {
            Some(start) => (crate::timeutil::now_secs() as u64).saturating_sub(start),
            None => state.started_at.elapsed().as_secs(),
        },
        binary_size_mb: proc.binary_size_bytes as f64 / MB,
    };

    Ok(SystemResponse {
        hostname: host.hostname.clone(),
        os: host.os.clone(),
        uptime_secs: host.uptime_secs,
        cpu_usage: (host.cpu_usage * 10.0).round() / 10.0,
        memory_total_mb: round1(mem_total),
        memory_used_mb: round1(mem_used),
        memory_percent: round1(mem_percent),
        disk_total_gb: round1(disk_total_gb),
        disk_used_gb: round1(disk_used_gb),
        disk_percent: round1(disk_percent),
        db_size_mb: round1(db_size_mb),
        process: process_info,
    })
}

#[utoipa::path(
    get,
    path = "/api/v1/system",
    responses((status = 200, body = crate::models::SystemResponse)),
    tag = "System",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_system(State(state): State<AppState>) -> Result<Json<SystemResponse>, AppError> {
    compute_system_info(&state).await.map(Json)
}

pub async fn get_active_features(State(state): State<AppState>) -> Json<serde_json::Value> {
    let license = {
        let status = state
            .license_status
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let mut license = status.site_support_json();
        license["tier"] = serde_json::to_value(status.tier()).unwrap();
        license
    };
    Json(serde_json::json!({
        "license": license,
        "protocols": {
            "modbus": cfg!(feature = "modbus"),
            "opcua": cfg!(feature = "opcua"),
            "mqtt_broker": cfg!(feature = "mqtt-broker"),
            "bacnet": cfg!(feature = "bacnet"),
            "dnp3": cfg!(feature = "dnp3"),
            "ethernet_ip": cfg!(feature = "ethernet-ip"),
            "s7": cfg!(feature = "s7"),
        },
        "snmp_scan": cfg!(feature = "snmp_scan"),
    }))
}
