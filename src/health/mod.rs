//! Subsystem health monitoring (watchdog) and write read-back tracking.

pub mod readback;
pub mod watchdog;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

use crate::api::AppState;

/// GET /api/v1/health/detailed
pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let (overall, subsystems) = state.health_monitor.get_health().await;
    let uptime = state.health_monitor.uptime_secs();

    let status_code = match overall {
        watchdog::HealthStatus::Healthy => StatusCode::OK,
        watchdog::HealthStatus::Degraded => StatusCode::OK,
        watchdog::HealthStatus::Critical => StatusCode::SERVICE_UNAVAILABLE,
        watchdog::HealthStatus::Stopped => StatusCode::SERVICE_UNAVAILABLE,
    };

    (
        status_code,
        Json(json!({
            "status": serde_json::to_value(overall).unwrap_or_default(),
            "uptime_secs": uptime,
            "subsystems": subsystems,
        })),
    )
}
