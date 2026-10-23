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
    use sysinfo::{Disks, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new_all();
    sys.refresh_all();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_cpu(),
    );

    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
    let os = System::name().unwrap_or_else(|| "unknown".to_string());
    let uptime_secs = System::uptime();
    let cpu_usage = sys.global_cpu_usage();
    let mem_total = sys.total_memory() as f64 / (1024.0 * 1024.0);
    let mem_used = sys.used_memory() as f64 / (1024.0 * 1024.0);
    let mem_percent = if mem_total > 0.0 {
        (mem_used / mem_total) * 100.0
    } else {
        0.0
    };

    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .find(|d| d.mount_point().to_str().unwrap_or("") == "/")
        .or_else(|| disks.list().first());

    let (disk_total_gb, disk_used_gb, disk_percent) = if let Some(d) = disk {
        let total = d.total_space() as f64 / (1024.0 * 1024.0 * 1024.0);
        let available = d.available_space() as f64 / (1024.0 * 1024.0 * 1024.0);
        let used = total - available;
        let pct = if total > 0.0 {
            (used / total) * 100.0
        } else {
            0.0
        };
        (total, used, pct)
    } else {
        (0.0, 0.0, 0.0)
    };

    let db_size_mb = std::fs::metadata(state.storage.db_path())
        .map(|m| m.len() as f64 / (1024.0 * 1024.0))
        .unwrap_or(0.0);

    let my_pid = sysinfo::Pid::from(std::process::id() as usize);
    let process_info = if let Some(proc) = sys.process(my_pid) {
        let proc_start = proc.start_time();
        let now = crate::timeutil::now_secs() as u64;
        let proc_uptime = now.saturating_sub(proc_start);
        let binary_size_mb = std::fs::metadata(std::env::current_exe().unwrap_or_default())
            .map(|m| m.len() as f64 / (1024.0 * 1024.0))
            .unwrap_or(0.0);
        ProcessInfo {
            pid: std::process::id(),
            memory_mb: proc.memory() as f64 / (1024.0 * 1024.0),
            cpu_percent: proc.cpu_usage(),
            uptime_secs: proc_uptime,
            binary_size_mb,
        }
    } else {
        ProcessInfo {
            pid: std::process::id(),
            memory_mb: 0.0,
            cpu_percent: 0.0,
            uptime_secs: state.started_at.elapsed().as_secs(),
            binary_size_mb: 0.0,
        }
    };

    Ok(SystemResponse {
        hostname,
        os,
        uptime_secs,
        cpu_usage: (cpu_usage * 10.0).round() / 10.0,
        memory_total_mb: (mem_total * 10.0).round() / 10.0,
        memory_used_mb: (mem_used * 10.0).round() / 10.0,
        memory_percent: (mem_percent * 10.0).round() / 10.0,
        disk_total_gb: (disk_total_gb * 10.0).round() / 10.0,
        disk_used_gb: (disk_used_gb * 10.0).round() / 10.0,
        disk_percent: (disk_percent * 10.0).round() / 10.0,
        db_size_mb: (db_size_mb * 10.0).round() / 10.0,
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

pub async fn get_active_features() -> Json<serde_json::Value> {
    Json(serde_json::json!({
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
