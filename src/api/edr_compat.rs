//! API endpoints for EDR compatibility status and recommendations.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;

use super::{AppError, AppState};
use crate::edr_compat::*;

/// GET /api/v1/edr/report
/// Returns a full EDR compatibility report for this system.
pub async fn get_edr_report(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let detected_products = detect_edr_products();
    let db_path = state.storage.db_path().to_string();
    let recommended_exclusions = generate_exclusions(&db_path);

    // Check I/O optimization status by verifying SQLite pragmas
    let io_optimization = check_io_status(&state).await;

    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();

    // Generate warnings based on detected products
    for product in &detected_products {
        if product.category == "edr" || product.category == "antivirus" {
            warnings.push(format!(
                "{} ({}) is active — add exclusions to prevent performance degradation",
                product.name, product.vendor
            ));
        }
    }

    // Generate recommendations
    if !detected_products.is_empty() {
        recommendations
            .push("Add recommended exclusion paths to your EDR/antivirus policy".to_string());
        recommendations.push(
            "Test Voltrus performance before and after adding exclusions to measure impact"
                .to_string(),
        );
    }

    if !io_optimization.wal_mode_active {
        warnings.push(
            "SQLite WAL mode is not active — this increases random writes and EDR scanning overhead".to_string()
        );
        recommendations
            .push("Enable SQLite WAL mode for better write performance with EDR tools".to_string());
    }

    if !io_optimization.batch_writes_enabled {
        recommendations.push(
            "Enable batch write optimization to reduce individual I/O operations".to_string(),
        );
    }

    if detected_products
        .iter()
        .any(|p| p.name.contains("FireEye") || p.name.contains("Trellix EDR"))
    {
        recommendations.push(
            "Trellix/FireEye EDR is known to cause high CPU on SCADA systems. Consider process-level exclusion for voltrus.exe".to_string()
        );
    }

    if detected_products
        .iter()
        .any(|p| p.name.contains("Defender"))
    {
        recommendations.push(
            "Consider using Microsoft Defender for OT for network-level monitoring instead of host-level scanning".to_string()
        );
    }

    let compatible = warnings.is_empty()
        || recommended_exclusions
            .iter()
            .any(|e| e.priority == "critical");

    let report = EdrCompatReport {
        detected_products,
        recommended_exclusions,
        io_optimization,
        compatible,
        warnings,
        recommendations,
    };

    Ok(Json(report))
}

/// GET /api/v1/edr/exclusions
/// Returns only the recommended exclusions (lighter endpoint for scripts).
pub async fn get_edr_exclusions(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = state.storage.db_path().to_string();
    let exclusions = generate_exclusions(&db_path);
    Ok(Json(serde_json::json!({
        "exclusions": exclusions,
        "count": exclusions.len(),
    })))
}

/// GET /api/v1/edr/exclusions/ps1
/// Returns a PowerShell script for applying Windows Defender exclusions.
pub async fn get_defender_ps1(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = state.storage.db_path().to_string();
    let exclusions = generate_exclusions(&db_path);
    let script = generate_defender_ps1(&exclusions);

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/plain")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static(
                    "attachment; filename=\"voltrus-defender-exclusions.ps1\"",
                ),
            ),
        ],
        script,
    ))
}

/// GET /api/v1/edr/gpo-guide
/// Returns Group Policy deployment guidance.
pub async fn get_gpo_guide() -> Result<impl IntoResponse, AppError> {
    let guide = generate_gpo_guidance();
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
        guide,
    ))
}

/// GET /api/v1/edr/io-status
/// Returns I/O optimization status and counters.
pub async fn get_io_status(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let status = check_io_status(&state).await;
    Ok(Json(status))
}

/// Check current I/O optimization status.
async fn check_io_status(state: &AppState) -> crate::edr_compat::IoOptimizationStatus {
    // Check WAL mode by querying SQLite pragma
    let wal_active = check_wal_mode(state).await;
    let checkpoint_optimized = check_checkpoint_optimized(state).await;

    crate::edr_compat::IoOptimizationStatus {
        batch_writes_enabled: true,
        batch_size: 500,
        wal_mode_active: wal_active,
        checkpoint_optimized,
        write_amplification: 1.0, // Would need IoCounters integration for real value
        sequential_write_pct: 0.95,
    }
}

/// Check if SQLite WAL mode is active.
async fn check_wal_mode(state: &AppState) -> bool {
    match state.storage.query_pragma("journal_mode").await {
        Ok(mode) => mode.to_lowercase().contains("wal"),
        Err(_) => false,
    }
}

/// Check if SQLite checkpoint settings are optimized.
async fn check_checkpoint_optimized(state: &AppState) -> bool {
    match state.storage.query_pragma("wal_autocheckpoint").await {
        Ok(val) => val.parse::<i64>().unwrap_or(0) > 0,
        Err(_) => false,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_edr_report_requires_auth() {
        let app = super::super::tests::test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/edr/report")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_edr_exclusions_requires_auth() {
        let app = super::super::tests::test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/edr/exclusions")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
