use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use crate::license::LicenseStatus;
use crate::models::{ComplianceHold, RetentionConfig};
use crate::storage::{ArchiveSink, PurgeKind, Sample, Storage, VacuumReport};
use crate::timeutil;
use anyhow::{Context, Result};
use flate2::Compression;
use flate2::write::GzEncoder;

/// Tag-hours packed per call before the worker takes a breath.
const PACK_BATCH: usize = 2000;

/// Background maintenance: the compaction worker packs complete hours older
/// than `pack_after_secs` into compressed blocks every `pack_interval_secs`,
/// and the retention cycle rolls expired tiers up, trims the event and audit
/// tables and returns freed pages to the filesystem every `run_interval_secs`.
/// All of it runs on this one task so the passes never overlap.
///
/// `retention.enabled: false` stops the historian tiers and the event and audit
/// trimming, but expired refresh tokens are still removed: they are not data
/// anyone keeps, and nothing else would ever delete them.
pub async fn run_retention_task(
    storage: Storage,
    config: RetentionConfig,
    license_status: Arc<RwLock<LicenseStatus>>,
) {
    if config.enabled {
        tracing::info!(
            "Historian maintenance started (pack: {} every {}s after {}s, retention every {}s, archive_dir: {})",
            if config.pack_enabled { "on" } else { "off" },
            config.pack_interval_secs,
            config.pack_after_secs,
            config.run_interval_secs,
            config.archive_dir,
        );
    } else {
        tracing::info!(
            "Data retention disabled; expired refresh tokens are still removed every {}s",
            config.run_interval_secs
        );
    }

    let mut retention_tick = tokio::time::interval(std::time::Duration::from_secs(
        config.run_interval_secs.max(1),
    ));
    retention_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pack_tick = tokio::time::interval(std::time::Duration::from_secs(
        config.pack_interval_secs.max(1),
    ));
    pack_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        // A passive node keeps whatever its active peer replicates; packing,
        // deleting or rolling up history locally would diverge from it.
        tokio::select! {
            _ = pack_tick.tick(), if config.enabled && config.pack_enabled => {
                if crate::node_role::is_active()
                    && let Err(e) = run_pack_cycle(&storage, &config).await
                {
                    tracing::error!("Historian packing failed: {e:#}");
                }
            }
            _ = retention_tick.tick() => {
                if !crate::node_role::is_active() {
                    continue;
                }
                if config.enabled
                    && let Err(e) = run_retention_cycle(&storage, &config).await
                {
                    tracing::error!("Retention cycle failed: {e:#}");
                }
                let hold = ComplianceHold::from_tier(
                    &license_status.read().unwrap_or_else(|e| e.into_inner()).tier(),
                );
                if let Err(e) = run_purge_cycle(&storage, &config, hold).await {
                    tracing::error!("Event retention failed: {e:#}");
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

/// What one [`run_purge_cycle`] removed.
#[derive(Debug, Default)]
pub struct PurgeSummary {
    pub removed: Vec<(PurgeKind, u64)>,
    /// `None` when `vacuum_step_pages` is 0.
    pub vacuum: Option<VacuumReport>,
}

impl PurgeSummary {
    /// Rows removed from the tables of `kind`.
    pub fn removed(&self, kind: PurgeKind) -> u64 {
        self.removed
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|(_, n)| n)
            .sum()
    }
}

/// Trim the event and audit tables to their configured keep, then hand the
/// freed pages back to the filesystem.
///
/// * Expired and revoked refresh tokens always go.
/// * With `retention.enabled`, rows older than the effective keep of each class
///   go (see [`RetentionConfig::effective_keep`]): cleared alarms, downtime
///   events, the security log, run history and, only when configured and not
///   held by the licence, the audit trails.
///
/// One failing table is logged and does not stop the others; the first error
/// is returned at the end.
pub async fn run_purge_cycle(
    storage: &Storage,
    config: &RetentionConfig,
    hold: ComplianceHold,
) -> Result<PurgeSummary> {
    let now = timeutil::now_secs();
    let mut plan = vec![(PurgeKind::RefreshTokens, now)];

    if config.enabled {
        let keep = config.effective_keep(hold);
        let cutoff = |secs: u64| now.saturating_sub(i64::try_from(secs).unwrap_or(i64::MAX));
        if let Some(secs) = keep.events {
            plan.push((PurgeKind::ClearedAlarms, cutoff(secs)));
            plan.push((PurgeKind::DowntimeEvents, cutoff(secs)));
        }
        if let Some(secs) = keep.security {
            plan.push((PurgeKind::SecurityLog, cutoff(secs)));
        }
        if let Some(secs) = keep.run_logs {
            plan.push((PurgeKind::FlowRuns, cutoff(secs)));
            plan.push((PurgeKind::ScriptExecutions, cutoff(secs)));
        }
        if let Some(secs) = keep.audit {
            plan.push((PurgeKind::AuditLog, cutoff(secs)));
            plan.push((PurgeKind::AlarmAuditLog, cutoff(secs)));
            plan.push((PurgeKind::FinishedCommands, cutoff(secs)));
        }
        if hold.audit && config.audit_keep_secs.is_some_and(|s| s > 0) {
            tracing::info!(
                "retention.audit_keep_secs is ignored: the licence includes 21 CFR Part 11 or IEC 62443 mode, so audit trails are kept"
            );
        }
        if hold.security && config.security_keep_secs.is_some_and(|s| s > 0) {
            tracing::info!(
                "retention.security_keep_secs is ignored: the licence includes IEC 62443 mode, so the security log is kept"
            );
        }
    }

    let mut summary = PurgeSummary::default();
    let mut first_error = None;
    for (kind, cutoff) in plan {
        match storage.purge(kind, cutoff, config.purge_batch_rows).await {
            Ok(0) => {}
            Ok(removed) => {
                tracing::info!("Retention removed {removed} rows ({kind:?})");
                summary.removed.push((kind, removed));
            }
            Err(e) => {
                tracing::error!("Retention of {kind:?} failed: {e:#}");
                first_error.get_or_insert(e);
            }
        }
    }

    if config.vacuum_step_pages > 0 {
        match storage.incremental_vacuum(config.vacuum_step_pages).await {
            Ok(report) => {
                if report.freed_pages > 0 {
                    tracing::info!(
                        "Returned {} free database pages to the filesystem",
                        report.freed_pages
                    );
                }
                summary.vacuum = Some(report);
            }
            Err(e) => {
                tracing::error!("Incremental vacuum failed: {e:#}");
                first_error.get_or_insert(e);
            }
        }
    }

    match first_error {
        Some(e) => Err(e),
        None => Ok(summary),
    }
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
            ..RetentionConfig::default()
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

    // ── Event and audit retention ──

    const DAY: i64 = 86_400;

    async fn count(storage: &Storage, table: &str) -> i64 {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        storage
            .with_conn(move |conn| Ok(conn.query_row(&sql, [], |r| r.get(0))?))
            .await
            .unwrap()
    }

    /// Every table retention touches, with one row ~900 days old and one fresh
    /// row, plus rows that must survive any keep: an unresolved old alarm, an
    /// ongoing downtime event, an unfinished command, an e-signature.
    async fn seed_history(storage: &Storage) {
        let t = timeutil::now_secs();
        let old = t - 900 * DAY;
        let recent = t - DAY;
        storage
            .with_conn(move |conn| {
                for created in [old, recent] {
                    conn.execute(
                        "INSERT INTO alarms (source_key, severity, value, threshold, message, triggered_at, resolved_at, state)
                         VALUES ('cleared', 'warning', 1, 1, 'm', ?1, ?1, 'resolved')",
                        [created],
                    )?;
                    let alarm = conn.last_insert_rowid();
                    conn.execute(
                        "INSERT INTO alarm_journal (alarm_id, event, at) VALUES (?1, 'triggered', ?2)",
                        rusqlite::params![alarm, created],
                    )?;
                    conn.execute(
                        "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, created_at) VALUES (?1, 'created', 'system', ?2)",
                        rusqlite::params![alarm, created],
                    )?;
                    conn.execute(
                        "INSERT INTO downtime_events (device_id, state, started_at, ended_at, created_at) VALUES ('d', 's', ?1, ?1 + 60, ?1)",
                        [created],
                    )?;
                    conn.execute(
                        "INSERT INTO security_log (event_type, severity, created_at) VALUES ('login_failed', 'info', ?1)",
                        [created],
                    )?;
                    conn.execute(
                        "INSERT INTO audit_log (action, username, resource, created_at) VALUES ('update', 'admin', 'device', ?1)",
                        [created],
                    )?;
                    conn.execute(
                        "INSERT INTO flow_runs (flow_id, started_at, duration_ms, success) VALUES ('f', ?1, 5, 1)",
                        [created],
                    )?;
                    conn.execute(
                        "INSERT INTO electronic_signatures (user_id, action, resource_type, resource_id, reason, meaning, signature_hash, created_at)
                         VALUES ('admin', 'approve', 'recipe', '1', 'r', 'm', 'h', ?1)",
                        [created],
                    )?;
                }
                conn.execute(
                    "INSERT INTO scripts (id, name, source) VALUES (1, 's', '')",
                    [],
                )?;
                for created in [old, recent] {
                    conn.execute(
                        "INSERT INTO script_executions (script_id, duration_ms, status, created_at) VALUES (1, 5, 'success', ?1)",
                        [created],
                    )?;
                    let id = format!("done{created}");
                    conn.execute(
                        "INSERT INTO commands (id, data_source_id, device_id, protocol, value, status, policy, created_at, resolved_at, created_by)
                         VALUES (?1, 1, 1, 'modbus', '1', 'acked', 'immediate', ?2, ?2, 'op')",
                        rusqlite::params![id, created],
                    )?;
                    conn.execute(
                        "INSERT INTO command_audit_log (command_id, old_status, new_status, changed_by, created_at) VALUES (?1, 'sent', 'acked', 'system', ?2)",
                        rusqlite::params![id, created],
                    )?;
                }
                // Survivors, however old.
                conn.execute(
                    "INSERT INTO alarms (source_key, severity, value, threshold, message, triggered_at, state)
                     VALUES ('active', 'critical', 1, 1, 'm', ?1, 'active')",
                    [old],
                )?;
                conn.execute(
                    "INSERT INTO downtime_events (device_id, state, started_at, created_at) VALUES ('d', 's', ?1, ?1)",
                    [old],
                )?;
                conn.execute(
                    "INSERT INTO commands (id, data_source_id, device_id, protocol, value, status, policy, created_at, created_by)
                     VALUES ('inflight', 1, 1, 'modbus', '1', 'sent', 'immediate', ?1, 'op')",
                    [old],
                )?;
                // Tokens: expired, live.
                conn.execute(
                    "INSERT INTO refresh_tokens (token_hash, username, expires_at, created_at) VALUES ('expired', 'u', ?1, ?2)",
                    rusqlite::params![t - 10, old],
                )?;
                conn.execute(
                    "INSERT INTO refresh_tokens (token_hash, username, expires_at, created_at) VALUES ('live', 'u', ?1, ?2)",
                    rusqlite::params![t + 1000, t],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// Row counts of every table seeded above.
    async fn counts(storage: &Storage) -> std::collections::BTreeMap<&'static str, i64> {
        let mut out = std::collections::BTreeMap::new();
        for table in [
            "alarms",
            "alarm_journal",
            "alarm_audit_log",
            "downtime_events",
            "security_log",
            "audit_log",
            "flow_runs",
            "script_executions",
            "electronic_signatures",
            "commands",
            "command_audit_log",
            "refresh_tokens",
        ] {
            out.insert(table, count(storage, table).await);
        }
        out
    }

    const NO_HOLD: ComplianceHold = ComplianceHold {
        audit: false,
        security: false,
    };

    #[tokio::test]
    async fn defaults_trim_events_and_keep_every_audit_trail() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;

        let summary = run_purge_cycle(&storage, &RetentionConfig::default(), NO_HOLD)
            .await
            .unwrap();

        let c = counts(&storage).await;
        // Events: the 900-day-old cleared alarm, ended downtime, security row,
        // flow run and script execution are gone (2 years / 90 days); recent
        // ones, the active alarm and the ongoing downtime stay.
        assert_eq!(c["alarms"], 2, "recent cleared + old active");
        assert_eq!(c["alarm_journal"], 1, "the old alarm's journal follows it");
        assert_eq!(c["downtime_events"], 2, "recent ended + old ongoing");
        assert_eq!(c["security_log"], 1);
        assert_eq!(c["flow_runs"], 1);
        assert_eq!(c["script_executions"], 1);
        // Audit trails and signatures: nothing deleted by default.
        assert_eq!(c["audit_log"], 2);
        assert_eq!(c["alarm_audit_log"], 2, "audit rows outlive their alarm");
        assert_eq!(c["commands"], 3);
        assert_eq!(c["command_audit_log"], 2);
        assert_eq!(c["electronic_signatures"], 2);
        // Tokens: expired gone, live stays.
        assert_eq!(c["refresh_tokens"], 1);

        assert_eq!(summary.removed(PurgeKind::RefreshTokens), 1);
        assert_eq!(summary.removed(PurgeKind::ClearedAlarms), 1);
        assert_eq!(summary.removed(PurgeKind::AuditLog), 0);
        assert!(summary.vacuum.is_some());
    }

    #[tokio::test]
    async fn audit_retention_is_opt_in_and_never_touches_signatures() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        let cfg = RetentionConfig {
            audit_keep_secs: Some(730 * DAY as u64),
            ..RetentionConfig::default()
        };

        run_purge_cycle(&storage, &cfg, NO_HOLD).await.unwrap();

        let c = counts(&storage).await;
        assert_eq!(c["audit_log"], 1);
        assert_eq!(c["alarm_audit_log"], 1);
        assert_eq!(
            c["commands"], 2,
            "finished old command gone, in-flight and recent stay"
        );
        assert_eq!(c["command_audit_log"], 1);
        assert_eq!(
            c["electronic_signatures"], 2,
            "Part 11 records are never purged"
        );
    }

    #[tokio::test]
    async fn a_licence_hold_overrides_configured_audit_and_security_keep() {
        let hold = ComplianceHold::from_tier(&crate::license::LicenseTier::Enterprise);
        assert_eq!(
            hold,
            ComplianceHold {
                audit: true,
                security: true
            }
        );
        assert_eq!(
            ComplianceHold::from_tier(&crate::license::LicenseTier::Community),
            NO_HOLD
        );
        assert_eq!(
            ComplianceHold::from_tier(&crate::license::LicenseTier::Integrator),
            NO_HOLD
        );

        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        let before = counts(&storage).await;
        let cfg = RetentionConfig {
            audit_keep_secs: Some(DAY as u64),
            security_keep_secs: Some(DAY as u64),
            ..RetentionConfig::default()
        };

        run_purge_cycle(&storage, &cfg, hold).await.unwrap();

        let c = counts(&storage).await;
        for table in [
            "audit_log",
            "alarm_audit_log",
            "commands",
            "command_audit_log",
            "security_log",
            "electronic_signatures",
        ] {
            assert_eq!(c[table], before[table], "{table} is held");
        }
        // Event history is not held: the old cleared alarm still goes.
        assert_eq!(c["alarms"], 2);
    }

    #[test]
    fn effective_keep_treats_zero_and_null_as_forever_and_applies_the_hold() {
        let cfg = RetentionConfig {
            event_keep_secs: Some(0),
            security_keep_secs: None,
            audit_keep_secs: Some(5),
            run_log_keep_secs: Some(7),
            ..RetentionConfig::default()
        };
        let keep = cfg.effective_keep(NO_HOLD);
        assert_eq!(keep.events, None);
        assert_eq!(keep.security, None);
        assert_eq!(keep.audit, Some(5));
        assert_eq!(keep.run_logs, Some(7));

        let held = cfg.effective_keep(ComplianceHold {
            audit: true,
            security: false,
        });
        assert_eq!(held.audit, None);
        assert_eq!(held.run_logs, Some(7));
    }

    #[test]
    fn config_defaults_apply_when_the_keys_are_absent() {
        let cfg: RetentionConfig = serde_yaml_neo::from_str("run_interval_secs: 60\n").unwrap();
        let keep = cfg.effective_keep(NO_HOLD);
        assert_eq!(keep.events, Some(2 * 365 * 86_400));
        assert_eq!(keep.security, Some(2 * 365 * 86_400));
        assert_eq!(keep.audit, None, "audit trails default to forever");
        assert_eq!(keep.run_logs, Some(90 * 86_400));
        assert_eq!(cfg.purge_batch_rows, 5_000);
        assert_eq!(cfg.vacuum_step_pages, 2_000);

        let cfg: RetentionConfig = serde_yaml_neo::from_str(
            "event_keep_secs: null\nsecurity_keep_secs: 0\naudit_keep_secs: 100\nrun_log_keep_secs: 50\n",
        )
        .unwrap();
        let keep = cfg.effective_keep(NO_HOLD);
        assert_eq!(keep.events, None, "explicit null keeps forever");
        assert_eq!(keep.security, None, "0 keeps forever");
        assert_eq!(keep.audit, Some(100));
        assert_eq!(keep.run_logs, Some(50));
    }

    #[tokio::test]
    async fn forever_keeps_every_event_but_still_drops_expired_tokens() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        let cfg = RetentionConfig {
            event_keep_secs: Some(0),
            security_keep_secs: None,
            run_log_keep_secs: Some(0),
            ..RetentionConfig::default()
        };

        run_purge_cycle(&storage, &cfg, NO_HOLD).await.unwrap();

        let c = counts(&storage).await;
        assert_eq!(c["alarms"], 3);
        assert_eq!(c["downtime_events"], 3);
        assert_eq!(c["security_log"], 2);
        assert_eq!(c["flow_runs"], 2);
        assert_eq!(c["script_executions"], 2);
        assert_eq!(c["refresh_tokens"], 1);
    }

    #[tokio::test]
    async fn disabled_retention_still_purges_expired_tokens_and_nothing_else() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        let before = counts(&storage).await;
        let cfg = RetentionConfig {
            enabled: false,
            audit_keep_secs: Some(DAY as u64),
            ..RetentionConfig::default()
        };

        run_purge_cycle(&storage, &cfg, NO_HOLD).await.unwrap();

        let c = counts(&storage).await;
        assert_eq!(c["refresh_tokens"], 1);
        for table in before.keys().filter(|t| **t != "refresh_tokens") {
            assert_eq!(c[table], before[table], "{table}");
        }
    }

    #[tokio::test]
    async fn a_one_second_keep_still_spares_unresolved_alarms() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        let cfg = RetentionConfig {
            event_keep_secs: Some(1),
            ..RetentionConfig::default()
        };

        run_purge_cycle(&storage, &cfg, NO_HOLD).await.unwrap();

        let active: i64 = storage
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM alarms WHERE source_key = 'active' AND resolved_at IS NULL",
                    [],
                    |r| r.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(active, 1);
        assert_eq!(
            count(&storage, "alarms").await,
            1,
            "every cleared alarm is gone"
        );
        assert_eq!(
            count(&storage, "downtime_events").await,
            1,
            "the ongoing downtime event stays"
        );
    }

    #[tokio::test]
    async fn one_failing_table_does_not_stop_the_others() {
        let storage = Storage::new_in_memory().unwrap();
        seed_history(&storage).await;
        storage
            .with_conn(|conn| Ok(conn.execute_batch("DROP TABLE security_log;")?))
            .await
            .unwrap();

        let err = run_purge_cycle(&storage, &RetentionConfig::default(), NO_HOLD)
            .await
            .unwrap_err();

        assert!(format!("{err:#}").contains("security_log"), "{err:#}");
        assert_eq!(count(&storage, "refresh_tokens").await, 1);
        assert_eq!(
            count(&storage, "flow_runs").await,
            1,
            "later tables still ran"
        );
    }
}
