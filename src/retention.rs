use std::fs::{File, OpenOptions};
use std::path::PathBuf;

use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;

use crate::models::RetentionConfig;
use crate::storage::{ArchiveSink, Sample, Storage};
use crate::timeutil;

/// Tag-hours packed per call before the worker takes a breath.
const PACK_BATCH: usize = 2000;

/// Background historian maintenance: the compaction worker packs complete
/// hours older than `pack_after_secs` into compressed blocks every
/// `pack_interval_secs`, and the retention cycle rolls expired tiers up every
/// `run_interval_secs`. Both run on this one task so they never overlap.
pub async fn run_retention_task(storage: Storage, config: RetentionConfig) {
    if !config.enabled {
        tracing::info!("Data retention disabled");
        return;
    }

    tracing::info!(
        "Historian maintenance started (pack: {} every {}s after {}s, retention every {}s, archive_dir: {})",
        if config.pack_enabled { "on" } else { "off" },
        config.pack_interval_secs,
        config.pack_after_secs,
        config.run_interval_secs,
        config.archive_dir,
    );

    let mut retention_tick = tokio::time::interval(std::time::Duration::from_secs(
        config.run_interval_secs.max(1),
    ));
    retention_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pack_tick = tokio::time::interval(std::time::Duration::from_secs(
        config.pack_interval_secs.max(1),
    ));
    pack_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = pack_tick.tick(), if config.pack_enabled => {
                if let Err(e) = run_pack_cycle(&storage, &config).await {
                    tracing::error!("Historian packing failed: {e:#}");
                }
            }
            _ = retention_tick.tick() => {
                if let Err(e) = run_retention_cycle(&storage, &config).await {
                    tracing::error!("Retention cycle failed: {e:#}");
                }
            }
        }
    }
}

/// Pack every complete hour older than the hot window. Works through a large
/// backlog (a database from before packing existed) in batches.
pub async fn run_pack_cycle(storage: &Storage, config: &RetentionConfig) -> Result<()> {
    let before = timeutil::now_secs() - config.pack_after_secs as i64;
    let mut total = crate::storage::PackReport::default();
    loop {
        let report = storage.pack_history(before, PACK_BATCH).await?;
        total.buckets += report.buckets;
        total.samples += report.samples;
        if !report.more {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if total.buckets > 0 {
        tracing::info!(
            "Historian packed {} samples from {} tag-hours",
            total.samples,
            total.buckets
        );
    }
    Ok(())
}

/// Roll the tiers up: expired full-resolution hours become hourly aggregates,
/// expired hourly rows become daily aggregates, and expired daily rows go.
pub async fn run_retention_cycle(storage: &Storage, config: &RetentionConfig) -> Result<()> {
    let now = timeutil::now_secs();
    tracing::info!("Starting retention cycle");

    let raw_window = config.raw_window_secs();
    let hourly_window = config.hourly_window_secs();

    // Full resolution -> hourly (archive first if asked).
    if config.tier_keep("raw").is_some() {
        let sink: Option<Box<dyn ArchiveSink>> = if config.tier_archive("raw") {
            Some(Box::new(CsvGzArchive::new(&config.archive_dir)))
        } else {
            None
        };
        let report = storage
            .expire_full_resolution(now - raw_window, sink)
            .await
            .context("Expiring full-resolution data failed")?;
        if report.buckets > 0 {
            tracing::info!(
                "Raw -> hourly: {} samples of {} tag-hours",
                report.samples,
                report.buckets
            );
        }
    }

    // Hourly -> daily. Whole days only, so a day is never rewritten from half
    // of its hours.
    if config.tier_keep("hourly").is_some() {
        let cutoff = (now - raw_window - hourly_window).div_euclid(86400) * 86400;
        let days = storage
            .downsample_hourly_to_daily(cutoff)
            .await
            .context("Hourly to daily failed")?;
        let removed = storage
            .delete_hourly_older_than(cutoff)
            .await
            .context("Deleting expired hourly rows failed")?;
        if removed > 0 {
            tracing::info!("Hourly -> daily: {removed} rows into {days} days");
        }
    }

    if let Some(keep) = config.tier_keep("daily") {
        let cutoff = now - raw_window - hourly_window - keep as i64;
        let removed = storage.delete_daily_older_than(cutoff).await?;
        if removed > 0 {
            tracing::info!("Expired {removed} daily rows");
        }
    }

    tracing::info!("Retention cycle complete");
    Ok(())
}

/// Appends expiring samples to `<archive_dir>/<YYYY-MM-DD>.csv.gz`, one gzip
/// member per hour (concatenated members are a valid gzip stream, so `zcat`
/// and any gzip reader see one file). The header row is written when the file
/// is created.
struct CsvGzArchive {
    dir: PathBuf,
    open: Option<(GzEncoder<File>, bool)>,
}

impl CsvGzArchive {
    fn new(dir: &str) -> Self {
        Self {
            dir: PathBuf::from(dir),
            open: None,
        }
    }
}

impl ArchiveSink for CsvGzArchive {
    fn begin(&mut self, bucket_start: i64) -> Result<()> {
        let date = time::OffsetDateTime::from_unix_timestamp(bucket_start)
            .ok()
            .and_then(|dt| {
                dt.format(&time::macros::format_description!("[year]-[month]-[day]"))
                    .ok()
            })
            .unwrap_or_else(|| "unknown".to_string());
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("Failed to create archive dir: {}", self.dir.display()))?;
        let path = self.dir.join(format!("{date}.csv.gz"));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open archive file: {}", path.display()))?;
        let is_new = file.metadata()?.len() == 0;
        self.open = Some((GzEncoder::new(file, Compression::default()), is_new));
        Ok(())
    }

    fn series(&mut self, device_id: &str, metric: &str, samples: &[Sample]) -> Result<()> {
        let (encoder, is_new) = self.open.as_mut().context("Archive hour not begun")?;
        let mut csv = csv::WriterBuilder::new()
            .has_headers(false)
            .from_writer(&mut *encoder);
        if *is_new {
            csv.write_record(["device_id", "metric", "timestamp", "value"])?;
            *is_new = false;
        }
        for (ts, value) in samples {
            csv.write_record([device_id, metric, &ts.to_string(), &value.to_string()])?;
        }
        csv.flush()?;
        Ok(())
    }

    fn commit(&mut self) -> Result<()> {
        if let Some((encoder, _)) = self.open.take() {
            let file = encoder.finish().context("Failed to finish gzip archive")?;
            file.sync_all().context("Failed to sync archive file")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RetentionTier;
    use std::io::Read;

    fn tier(name: &str, keep: Option<u64>, archive: bool) -> RetentionTier {
        RetentionTier {
            name: name.into(),
            keep,
            downsample_to: None,
            archive,
        }
    }

    fn config(raw_keep: u64, archive: bool, dir: &str) -> RetentionConfig {
        RetentionConfig {
            enabled: true,
            run_interval_secs: 60,
            archive_dir: dir.into(),
            tiers: vec![
                tier("raw", Some(raw_keep), archive),
                tier("hourly", Some(30 * 86400), false),
                tier("daily", None, false),
            ],
            pack_enabled: true,
            pack_after_secs: 3600,
            pack_interval_secs: 60,
        }
    }

    /// One tag sampled once a minute for `hours` hours ending an hour from now.
    fn minutes(hours: i64) -> (Vec<(String, i64, String, f64)>, i64) {
        let end = timeutil::now_secs() / 3600 * 3600;
        let start = end - hours * 3600;
        let rows = (0..hours * 60)
            .map(|i| {
                (
                    "plc".to_string(),
                    start + i * 60,
                    "flow".to_string(),
                    (i % 90) as f64,
                )
            })
            .collect();
        (rows, start)
    }

    #[tokio::test]
    async fn pack_cycle_leaves_only_the_hot_window_as_rows() {
        let storage = Storage::new_in_memory().unwrap();
        let (rows, start) = minutes(10);
        storage.insert_batch(&rows).await.unwrap();

        let cfg = config(86400 * 30, false, "unused");
        run_pack_cycle(&storage, &cfg).await.unwrap();

        let stats = storage.get_compression_stats().await.unwrap();
        // Hours that ended an hour or more ago are packed; the latest hour
        // (and anything inside pack_after) is still raw.
        assert_eq!(stats.packed.samples + stats.hot_rows, rows.len() as i64);
        assert!(
            stats.packed.samples >= 9 * 60,
            "packed {}",
            stats.packed.samples
        );
        assert!(stats.hot_rows > 0 && stats.hot_rows <= 120);

        // Still all there for readers.
        let all = storage
            .query_history("plc", "flow", start, start + 11 * 3600)
            .await
            .unwrap();
        assert_eq!(all.len(), rows.len());
    }

    #[tokio::test]
    async fn retention_cycle_rolls_expired_hours_into_the_hourly_tier() {
        let dir = tempfile::tempdir().unwrap();
        let archive_dir = dir.path().join("archive");
        let storage = Storage::new_in_memory().unwrap();
        let (rows, start) = minutes(10);
        storage.insert_batch(&rows).await.unwrap();

        // Keep five hours of full resolution; archive what leaves.
        let cfg = config(5 * 3600, true, archive_dir.to_str().unwrap());
        run_pack_cycle(&storage, &cfg).await.unwrap();
        run_retention_cycle(&storage, &cfg).await.unwrap();

        let cutoff = (timeutil::now_secs() - 5 * 3600) / 3600 * 3600;
        let old = rows.iter().filter(|r| r.1 < cutoff).count() as i64;
        assert!(old > 0);
        assert_eq!(
            storage
                .count_telemetry(None, None, Some(start), Some(cutoff - 1))
                .await
                .unwrap(),
            0,
            "expired full resolution is gone"
        );
        assert_eq!(
            storage
                .count_telemetry(None, None, Some(cutoff), None)
                .await
                .unwrap(),
            rows.len() as i64 - old
        );
        let hourly = storage
            .count_telemetry_table("telemetry_hourly", "bucket_start", None, None, None, None)
            .await
            .unwrap();
        assert_eq!(hourly, (cutoff - start) / 3600);

        // What left was archived, readable as one gzip stream with one header.
        let mut text = String::new();
        let mut files = 0;
        for entry in std::fs::read_dir(&archive_dir).unwrap() {
            let path = entry.unwrap().path();
            assert!(path.to_string_lossy().ends_with(".csv.gz"));
            let mut dec = flate2::read::MultiGzDecoder::new(std::fs::File::open(path).unwrap());
            dec.read_to_string(&mut text).unwrap();
            files += 1;
        }
        assert!(files >= 1);
        assert_eq!(
            text.matches("device_id,metric,timestamp,value").count(),
            files
        );
        assert_eq!(text.lines().count() as i64, old + files as i64);
        assert!(text.contains(&format!("plc,flow,{start},0")));

        // A second cycle finds nothing to do.
        run_retention_cycle(&storage, &cfg).await.unwrap();
        assert_eq!(
            std::fs::read_dir(&archive_dir).unwrap().count(),
            files,
            "no new archive files"
        );
    }

    #[tokio::test]
    async fn hourly_rows_roll_into_weighted_daily_rows() {
        let storage = Storage::new_in_memory().unwrap();
        // Two hourly rows of one old day: 100 samples averaging 10, 300 averaging 20.
        let day = (timeutil::now_secs() - 400 * 86400) / 86400 * 86400;
        storage
            .with_conn(move |conn| {
                for (hour, avg, count) in [(day, 10.0, 100), (day + 3600, 20.0, 300)] {
                    conn.execute(
                        "INSERT INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
                         VALUES ('d', 'm', ?1, ?2, ?2, ?2, ?3)",
                        rusqlite::params![hour, avg, count],
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let mut cfg = config(86400, false, "unused");
        cfg.tiers[1].keep = Some(86400);
        run_retention_cycle(&storage, &cfg).await.unwrap();

        let stats = storage.get_compression_stats().await.unwrap();
        assert_eq!(stats.warm_rows, 0, "expired hourly rows removed");
        assert_eq!(stats.cold_rows, 1);
        let (avg, count): (f64, i64) = storage
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT avg_value, sample_count FROM telemetry_daily WHERE device_id = 'd'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(count, 400);
        assert!((avg - 17.5).abs() < 1e-9, "weighted mean, got {avg}");
    }
}
