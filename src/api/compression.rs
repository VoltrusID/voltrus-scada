use axum::{Json, extract::State, response::IntoResponse};
use serde::Serialize;

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CompressionStatsResponse {
    /// Full-resolution rows not packed yet (the hot window).
    pub hot_rows: i64,
    /// Hourly aggregate rows.
    pub warm_rows: i64,
    /// Daily aggregate rows.
    pub cold_rows: i64,
    /// Compressed blocks of full-resolution history (one per tag and hour).
    pub packed_blocks: i64,
    /// Samples held in those blocks.
    pub packed_samples: i64,
    /// Compressed payload of those blocks in bytes.
    pub packed_payload_bytes: i64,
    /// Payload bytes per packed sample; 0 when nothing is packed yet.
    pub packed_bytes_per_sample: f64,
    pub db_size_bytes: u64,
    pub db_size_mb: f64,
}

/// GET /api/v1/compression/stats -- historian tier sizes
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
        packed_blocks: stats.packed.blocks,
        packed_samples: stats.packed.samples,
        packed_payload_bytes: stats.packed.payload_bytes,
        packed_bytes_per_sample: if stats.packed.samples > 0 {
            stats.packed.payload_bytes as f64 / stats.packed.samples as f64
        } else {
            0.0
        },
        db_size_bytes: stats.db_size_bytes,
        db_size_mb: stats.db_size_bytes as f64 / 1_048_576.0,
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CompressionConfigResponse {
    pub enabled: bool,
    pub pack_enabled: bool,
    /// Complete hours older than this are packed.
    pub pack_after_secs: u64,
    pub pack_interval_secs: u64,
    /// Full-resolution window; `null` keeps it forever.
    pub raw_keep_secs: Option<u64>,
    /// Hourly window past the raw window; `null` keeps it forever.
    pub hourly_keep_secs: Option<u64>,
    /// Daily window past the hourly window; `null` keeps it forever.
    pub daily_keep_secs: Option<u64>,
    /// Whether expiring full-resolution data is exported to CSV first.
    pub archive_raw: bool,
}

/// GET /api/v1/compression/config -- the running retention and packing settings
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
    State(state): State<crate::api::AppState>,
) -> Result<impl IntoResponse, crate::api::AppError> {
    let cfg = &state.retention_config;
    Ok(Json(CompressionConfigResponse {
        enabled: cfg.enabled,
        pack_enabled: cfg.pack_enabled,
        pack_after_secs: cfg.pack_after_secs,
        pack_interval_secs: cfg.pack_interval_secs,
        raw_keep_secs: cfg.tier_keep("raw"),
        hourly_keep_secs: cfg.tier_keep("hourly"),
        daily_keep_secs: cfg.tier_keep("daily"),
        archive_raw: cfg.tier_archive("raw"),
    }))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct DeadbandEntryResponse {
    pub device_id: String,
    pub metric: String,
    /// `absolute` (engineering units) or `percent` of the last stored value.
    pub deadband_type: String,
    pub deadband_value: f64,
    /// Longest gap between stored samples of a flat signal; `null` = none.
    pub max_interval_secs: Option<i64>,
}

/// GET /api/v1/compression/deadband -- tags that filter their history writes
///
/// A tag opts in with `history_deadband`, `history_deadband_type` and
/// `history_max_interval_secs` in its `source_config`; every other tag stores
/// each sample.
#[utoipa::path(
    get,
    path = "/api/v1/compression/deadband",
    responses(
        (status = 200, body = Vec<DeadbandEntryResponse>),
    ),
    tag = "Compression",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_compression_deadband(
    State(state): State<crate::api::AppState>,
) -> Result<impl IntoResponse, crate::api::AppError> {
    use crate::deadband::{DeadbandKind, HistoryRule};

    let sources = state.storage.list_data_sources(None).await?;
    let entries: Vec<DeadbandEntryResponse> = sources
        .iter()
        .filter_map(|source| {
            let cfg = serde_json::from_str::<serde_json::Value>(&source.source_config).ok()?;
            let rule = HistoryRule::from_source_config(&cfg)?;
            Some(DeadbandEntryResponse {
                device_id: source.device_id.clone(),
                metric: crate::api::data::metric_name(&source.key, &source.device_id).to_string(),
                deadband_type: match rule.kind {
                    DeadbandKind::Absolute => "absolute",
                    DeadbandKind::Percent => "percent",
                }
                .to_string(),
                deadband_value: rule.deadband,
                max_interval_secs: rule.max_interval_secs,
            })
        })
        .collect();
    Ok(Json(entries))
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
