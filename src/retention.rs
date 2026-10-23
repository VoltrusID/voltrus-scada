use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::models::RetentionConfig;
use crate::storage::Storage;
use crate::timeutil;

pub async fn run_retention_task(storage: Storage, config: RetentionConfig) {
    if !config.enabled {
        tracing::info!("Data retention disabled");
        return;
    }

    tracing::info!(
        "Data retention task started (interval: {}s, archive_dir: {})",
        config.run_interval_secs,
        config.archive_dir,
    );

    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(config.run_interval_secs));

    loop {
        interval.tick().await;

        if let Err(e) = run_retention_cycle(&storage, &config).await {
            tracing::error!("Retention cycle failed: {e}");
        }
    }
}

async fn run_retention_cycle(storage: &Storage, config: &RetentionConfig) -> Result<()> {
    let now = timeutil::now_secs();
    tracing::info!("Starting retention/compression cycle");

    // Hot → Warm compression (data older than 24 hours)
    let hot_cutoff = (now - 86400) * 1000; // convert to ms
    match storage.compress_hot_to_warm(hot_cutoff).await {
        Ok(count) => tracing::info!("Hot→Warm: compressed {count} rows"),
        Err(e) => tracing::error!("Hot→Warm failed: {e}"),
    }

    // Warm → Cold compression (data older than 30 days)
    let warm_cutoff = (now - 2592000) * 1000; // convert to ms
    match storage.compress_warm_to_cold(warm_cutoff).await {
        Ok(count) => tracing::info!("Warm→Cold: compressed {count} rows"),
        Err(e) => tracing::error!("Warm→Cold failed: {e}"),
    }

    // Archive old cold data if configured
    let cold_keep = get_tier_keep(&config.tiers, "daily");
    if let Some(cold_keep_secs) = cold_keep {
        let cutoff = now - cold_keep_secs as i64;
        if get_tier_archive(&config.tiers, "daily") {
            archive_raw(storage, cutoff, &config.archive_dir).await?;
        }
    }

    tracing::info!("Retention/compression cycle complete");
    Ok(())
}

async fn archive_raw(storage: &Storage, cutoff: i64, archive_dir: &str) -> Result<()> {
    let rows = storage.query_raw_for_archive(cutoff).await?;
    if rows.is_empty() {
        return Ok(());
    }

    // Grouping + gzip file writes are blocking I/O — run them off the async
    // runtime worker so a large archive can't stall the broker/pollers.
    let archive_dir = archive_dir.to_string();
    tokio::task::spawn_blocking(move || {
        // Group rows by date
        let mut by_date: BTreeMap<String, Vec<(String, String, i64, f64)>> = BTreeMap::new();
        for (device_id, metric, ts, value) in &rows {
            let date = time::OffsetDateTime::from_unix_timestamp(*ts)
                .ok()
                .and_then(|dt| {
                    dt.format(&time::macros::format_description!("[year]-[month]-[day]"))
                        .ok()
                })
                .unwrap_or_else(|| "unknown".to_string());
            by_date
                .entry(date)
                .or_default()
                .push((device_id.clone(), metric.clone(), *ts, *value));
        }

        std::fs::create_dir_all(&archive_dir)
            .with_context(|| format!("Failed to create archive dir: {archive_dir}"))?;

        for (date, day_rows) in &by_date {
            let path = PathBuf::from(&archive_dir).join(format!("{date}.csv.gz"));
            let file = std::fs::File::create(&path)
                .with_context(|| format!("Failed to create archive file: {}", path.display()))?;
            let mut encoder = GzEncoder::new(file, Compression::default());
            writeln!(encoder, "device_id,metric,timestamp,value")?;
            for (device_id, metric, ts, value) in day_rows {
                writeln!(encoder, "{device_id},{metric},{ts},{value}")?;
            }
            encoder.finish().context("Failed to finish gzip archive")?;
            tracing::info!("Archived {} rows to {}", day_rows.len(), path.display());
        }

        Ok(())
    })
    .await
    .context("Archive task join failed")?
}

fn get_tier_keep(tiers: &[crate::models::RetentionTier], name: &str) -> Option<u64> {
    tiers.iter().find(|t| t.name == name).and_then(|t| t.keep)
}

fn get_tier_archive(tiers: &[crate::models::RetentionTier], name: &str) -> bool {
    tiers
        .iter()
        .find(|t| t.name == name)
        .map(|t| t.archive)
        .unwrap_or(false)
}
