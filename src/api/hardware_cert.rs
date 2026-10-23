//! API endpoints for hardware certification and edge deployment.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;

use super::{AppError, AppState};
use crate::hardware_cert::*;

/// GET /api/v1/hw/report
/// Returns full hardware certification report for the current system.
pub async fn get_hw_report(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let db_path = state.storage.db_path().to_string();
    let platform = tokio::task::spawn_blocking(move || {
        crate::hardware_cert::platform::detect_platform(&db_path)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Platform detection failed: {e}"))?;

    let compatibility = crate::hardware_cert::compat_matrix::assess_compatibility(&platform);
    let storage_assessment = crate::hardware_cert::compat_matrix::assess_storage(&platform);

    let mut warnings = Vec::new();
    let mut recommendations = Vec::new();

    // Generate warnings
    if platform.ram_total_mb < 1024 {
        warnings.push(format!(
            "Low RAM: {} MB — may limit performance with many tags",
            platform.ram_total_mb
        ));
    }

    if platform.storage_type == "sd_card" {
        warnings.push("SD card storage detected — enable compression to extend card life".into());
        recommendations.push("Consider upgrading to SSD or NVMe for production deployments".into());
    }

    if platform.is_raspberry_pi && platform.cpu_cores < 4 {
        warnings.push(
            "Raspberry Pi with fewer than 4 cores — limit tag count for best performance".into(),
        );
    }

    if !compatibility.enterprise.compatible && compatibility.professional.compatible {
        recommendations.push(
            "Hardware supports Professional tier. For Enterprise, consider upgrading to 4+ cores and 2GB+ RAM.".into(),
        );
    }

    // Determine certified tiers
    let certified_tiers = vec![
        ("engineering".into(), compatibility.engineering.compatible),
        ("starter".into(), compatibility.starter.compatible),
        ("professional".into(), compatibility.professional.compatible),
        ("enterprise".into(), compatibility.enterprise.compatible),
    ]
    .into_iter()
    .filter_map(|(tier, ok)| if ok { Some(tier) } else { None })
    .collect::<Vec<String>>();

    let certified = !certified_tiers.is_empty();

    let report = HardwareCertReport {
        platform,
        compatibility,
        storage_assessment,
        certified,
        certified_tiers,
        warnings,
        recommendations,
    };

    Ok(Json(report))
}

/// GET /api/v1/hw/compatibility-matrix
/// Returns the official hardware compatibility matrix for documentation.
pub async fn get_compatibility_matrix() -> Result<impl IntoResponse, AppError> {
    let matrix = crate::hardware_cert::compat_matrix::get_compatibility_matrix();
    Ok(Json(serde_json::json!({
        "matrix": matrix,
        "last_updated": "2026-05",
        "disclaimer": "Recommendations based on testing with typical SCADA workloads. Actual performance depends on tag count, polling rate, and enabled features."
    })))
}

#[derive(Debug, Deserialize)]
pub struct BenchmarkQuery {
    /// Duration in seconds (default: 10, max: 60)
    pub duration: Option<u64>,
}

/// POST /api/v1/hw/benchmark
/// Run performance benchmarks on the current hardware.
pub async fn run_hw_benchmark(
    State(state): State<AppState>,
    Query(params): Query<BenchmarkQuery>,
) -> Result<impl IntoResponse, AppError> {
    let duration = params.duration.unwrap_or(10).min(60);
    let db_path = state.storage.db_path().to_string();

    let results = tokio::task::spawn_blocking(move || {
        let platform = crate::hardware_cert::platform::detect_platform(&db_path);
        crate::hardware_cert::benchmark::run_benchmarks(&platform, Some(duration))
    })
    .await
    .map_err(|e| anyhow::anyhow!("Benchmark task failed: {e}"))?
    .map_err(|e| anyhow::anyhow!("Benchmark failed: {e}"))?;

    Ok(Json(results))
}

/// GET /api/v1/hw/edge-deploy
/// Returns edge deployment guide for the current platform.
pub async fn get_edge_deploy_guide(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let db_path = state.storage.db_path().to_string();
    let guide = tokio::task::spawn_blocking(move || {
        let platform = crate::hardware_cert::platform::detect_platform(&db_path);
        crate::hardware_cert::edge_deploy::generate_edge_guide(&platform)
    })
    .await
    .map_err(|e| anyhow::anyhow!("Guide generation failed: {e}"))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
        guide,
    ))
}

/// GET /api/v1/hw/edge-deploy/rpi-quickstart
/// Returns Raspberry Pi quick-start guide.
pub async fn get_rpi_quickstart() -> Result<impl IntoResponse, AppError> {
    let guide = crate::hardware_cert::edge_deploy::generate_rpi_quickstart();
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"))],
        guide,
    ))
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::test_app;
    use axum::body::Body;
    use http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_hw_report_requires_auth() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hw/report")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_hw_compat_matrix_requires_auth() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hw/compatibility-matrix")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_hw_benchmark_requires_auth() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hw/benchmark")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_edge_deploy_requires_auth() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/hw/edge-deploy")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
