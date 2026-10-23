use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CompressionStatsResponse {
    pub hot_rows: i64,
    pub warm_rows: i64,
    pub cold_rows: i64,
    pub db_size_bytes: u64,
    pub db_size_mb: f64,
    pub estimated_hot_mb: f64,
    pub estimated_warm_mb: f64,
    pub estimated_cold_mb: f64,
}

/// GET /api/v1/compression/stats -- get compression statistics
#[utoipa::path(
    get,
    path = "/api/v1/compression/stats",
    responses(
        (status = 200, body = CompressionStatsResponse),
    ),
    tag = "Compression",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_compression_stats(
    State(state): State<crate::api::AppState>,
) -> Result<impl IntoResponse, crate::api::AppError> {
    let stats = state.storage.get_compression_stats().await?;

    Ok(Json(CompressionStatsResponse {
        hot_rows: stats.hot_rows,
        warm_rows: stats.warm_rows,
        cold_rows: stats.cold_rows,
        db_size_bytes: stats.db_size_bytes,
        db_size_mb: stats.db_size_bytes as f64 / 1_048_576.0,
        estimated_hot_mb: stats.hot_rows as f64 * 48.0 / 1_048_576.0,
        estimated_warm_mb: stats.warm_rows as f64 * 80.0 / 1_048_576.0,
        estimated_cold_mb: stats.cold_rows as f64 * 200.0 / 1_048_576.0,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CompressionConfigResponse {
    pub enabled: bool,
    pub hot_retention_secs: u64,
    pub warm_retention_secs: u64,
    pub cold_retention_secs: u64,
}

/// GET /api/v1/compression/config -- get compression configuration
#[utoipa::path(
    get,
    path = "/api/v1/compression/config",
    responses(
        (status = 200, body = CompressionConfigResponse),
    ),
    tag = "Compression",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_compression_config(
    State(_state): State<crate::api::AppState>,
) -> Result<impl IntoResponse, crate::api::AppError> {
    Ok(Json(CompressionConfigResponse {
        enabled: true,
        hot_retention_secs: 86400,
        warm_retention_secs: 2_592_000,
        cold_retention_secs: 31_536_000,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct MemoryStatsResponse {
    pub rss_mb: f64,
    pub hot_cache_entries: usize,
    pub connection_pool_size: usize,
}

/// GET /api/v1/system/memory -- get system memory usage
#[utoipa::path(
    get,
    path = "/api/v1/system/memory",
    responses(
        (status = 200, body = MemoryStatsResponse),
    ),
    tag = "Compression",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_system_memory(
    State(_state): State<crate::api::AppState>,
) -> Result<impl IntoResponse, crate::api::AppError> {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();

    Ok(Json(MemoryStatsResponse {
        rss_mb: sys.used_memory() as f64 / 1_048_576.0,
        hot_cache_entries: 0,
        connection_pool_size: 0,
    }))
}
