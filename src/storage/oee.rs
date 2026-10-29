use super::Storage;
use anyhow::{Context, Result};

use crate::models::{DataPoint, DowntimeEvent, OeeMetrics, ShiftConfig};

impl Storage {
    // ── OEE / Downtime methods ──

    /// Create an open downtime event.
    pub async fn create_downtime_event(&self, device_id: &str, state: &str) -> Result<i64> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let state = state.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO downtime_events (device_id, state, started_at, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![device_id, state, now, now],
            ).context("Insert downtime event failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    /// Close a downtime event by setting ended_at and calculating duration.
    pub async fn close_downtime_event(&self, id: i64, reason: Option<&str>) -> Result<()> {
        let conn = self.conn.clone();
        let reason = reason.map(|s| s.to_string());
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Calculate duration from started_at
            let started_at: i64 = conn.query_row(
                "SELECT started_at FROM downtime_events WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            ).context("Failed to get downtime started_at")?;
            let duration = now - started_at;
            conn.execute(
                "UPDATE downtime_events SET ended_at = ?1, duration_secs = ?2, reason = ?3 WHERE id = ?4 AND ended_at IS NULL",
                rusqlite::params![now, duration, reason, id],
            ).context("Close downtime event failed")?;
            Ok(())
        }).await.context("Task join failed")?
    }

    /// List downtime events with optional filters.
    pub async fn list_downtime_events(
        &self,
        device_id: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<Vec<DowntimeEvent>> {
        let conn = self.conn.clone();
        let device_id = device_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from("SELECT id, device_id, state, started_at, ended_at, duration_secs, reason FROM downtime_events WHERE 1=1");
            if let Some(ref did) = device_id {
                sql.push_str(&format!(" AND device_id = '{}'", did.replace('\'', "''")));
            }
            if let Some(from) = from {
                sql.push_str(&format!(" AND started_at >= {}", from));
            }
            if let Some(to) = to {
                sql.push_str(&format!(" AND started_at <= {}", to));
            }
            sql.push_str(" ORDER BY started_at DESC");

            let mut stmt = conn.prepare(&sql).context("Prepare list downtime events failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(DowntimeEvent {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    state: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    duration_secs: row.get(5)?,
                    reason: row.get(6)?,
                })
            }).context("Query list downtime events failed")?;

            let mut events = Vec::new();
            for row in rows {
                events.push(row.context("Row parse downtime event failed")?);
            }
            Ok(events)
        }).await.context("Task join failed")?
    }

    /// Get the currently open (unclosed) downtime event for a device.
    pub async fn get_open_downtime_event(&self, device_id: &str) -> Result<Option<DowntimeEvent>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, device_id, state, started_at, ended_at, duration_secs, reason FROM downtime_events WHERE device_id = ?1 AND ended_at IS NULL ORDER BY started_at DESC LIMIT 1"
            ).context("Prepare get open downtime event failed")?;
            let mut rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(DowntimeEvent {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    state: row.get(2)?,
                    started_at: row.get(3)?,
                    ended_at: row.get(4)?,
                    duration_secs: row.get(5)?,
                    reason: row.get(6)?,
                })
            }).context("Query get open downtime event failed")?;
            match rows.next() {
                Some(row) => Ok(Some(row.context("Row parse open downtime event failed")?)),
                None => Ok(None),
            }
        }).await.context("Task join failed")?
    }

    /// Calculate OEE metrics for a device over a time range.
    /// Availability = total_running_secs / total_planned_secs
    /// Performance and Quality default to 1.0 (to be enriched by user configuration).
    /// OEE = Availability * Performance * Quality
    pub async fn calculate_oee(&self, device_id: &str, from: i64, to: i64) -> Result<OeeMetrics> {
        let events = self
            .list_downtime_events(Some(device_id), Some(from), Some(to))
            .await?;

        let total_planned_secs = if to > from { to - from } else { 0 };

        // Total downtime = sum of all downtime durations (including currently open events)
        let mut total_downtime_secs: i64 = 0;
        let now = crate::timeutil::now_secs();
        let mut downtime_count: i64 = 0;

        for event in &events {
            downtime_count += 1;
            if let Some(duration) = event.duration_secs {
                total_downtime_secs += duration;
            } else if event.ended_at.is_none() {
                // Still open — calculate from started_at to now
                total_downtime_secs += (now - event.started_at).max(0);
            }
        }

        let total_running_secs = (total_planned_secs - total_downtime_secs).max(0);

        let availability = if total_planned_secs > 0 {
            total_running_secs as f64 / total_planned_secs as f64
        } else {
            1.0
        };

        // Performance and quality default to 1.0 until enriched with production data
        let performance = 1.0;
        let quality = 1.0;
        let oee = availability * performance * quality;

        Ok(OeeMetrics {
            availability,
            performance,
            quality,
            oee,
            total_running_secs,
            total_planned_secs,
            downtime_count,
        })
    }

    // ── Shift methods ──

    /// Create a shift configuration.
    pub async fn create_shift(
        &self,
        name: &str,
        start_hour: u32,
        start_minute: u32,
        duration_hours: u32,
        days_mask: u32,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO shifts (name, start_hour, start_minute, duration_hours, days_mask, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![name, start_hour, start_minute, duration_hours, days_mask, now],
            ).context("Insert shift failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    /// List all shift configurations.
    pub async fn list_shifts(&self) -> Result<Vec<ShiftConfig>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, start_hour, start_minute, duration_hours, days_mask, created_at FROM shifts ORDER BY id"
            ).context("Prepare list shifts failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(ShiftConfig {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    start_hour: row.get(2)?,
                    start_minute: row.get(3)?,
                    duration_hours: row.get(4)?,
                    days_mask: row.get(5)?,
                    created_at: row.get(6)?,
                })
            }).context("Query list shifts failed")?;
            let mut shifts = Vec::new();
            for row in rows {
                shifts.push(row.context("Row parse shift failed")?);
            }
            Ok(shifts)
        }).await.context("Task join failed")?
    }

    /// Delete a shift configuration.
    pub async fn delete_shift(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM shifts WHERE id = ?1", rusqlite::params![id])
                .context("Delete shift failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Query distinct device IDs with full-resolution telemetry (raw or packed).
    pub async fn get_telemetry_device_ids(&self) -> Result<Vec<String>> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let mut stmt = conn
                .prepare(
                    "SELECT device_id FROM telemetry
                     UNION
                     SELECT device_id FROM telemetry_blocks
                     ORDER BY device_id",
                )
                .context("Prepare get_telemetry_device_ids failed")?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .context("Query get_telemetry_device_ids failed")?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.context("Row parse get_telemetry_device_ids failed")?);
            }
            Ok(ids)
        })
        .await
        .context("Task join failed")?
    }

    /// Query distinct metrics of a device with full-resolution telemetry (raw or packed).
    pub async fn get_telemetry_metrics(&self, device_id: &str) -> Result<Vec<String>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let mut stmt = conn
                .prepare(
                    "SELECT metric FROM telemetry WHERE device_id = ?1
                     UNION
                     SELECT metric FROM telemetry_blocks WHERE device_id = ?1
                     ORDER BY metric",
                )
                .context("Prepare get_telemetry_metrics failed")?;
            let rows = stmt
                .query_map(rusqlite::params![device_id], |row| row.get::<_, String>(0))
                .context("Query get_telemetry_metrics failed")?;
            let mut metrics = Vec::new();
            for row in rows {
                metrics.push(row.context("Row parse get_telemetry_metrics failed")?);
            }
            Ok(metrics)
        })
        .await
        .context("Task join failed")?
    }

    /// Query telemetry history for a device with optional metric filter, oldest
    /// first; raw rows and packed blocks merged. A tag with more than 5000
    /// samples in the window is thinned evenly over the whole window, keeping
    /// the lowest and highest sample of each stretch, so a minimum or maximum
    /// taken from the points is still the real one. Without a metric the
    /// samples of every tag are returned together and the call fails if there
    /// are more than 5000.
    pub async fn query_history_multi(
        &self,
        device_id: &str,
        from: i64,
        to: i64,
        metric: Option<&str>,
    ) -> Result<Vec<DataPoint>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let samples: Vec<(i64, f64)> = match &metric {
                Some(m) => super::historian::read_series_thinned(
                    &conn,
                    &device_id,
                    m,
                    from,
                    to,
                    5000,
                    super::historian::Thin::Extremes,
                )?,
                None => super::historian::read_device_all(&conn, &device_id, from, to, 5000)?
                    .into_iter()
                    .map(|(ts, _, v)| (ts, v))
                    .collect(),
            };
            Ok(samples
                .into_iter()
                .map(|(timestamp, value)| DataPoint { timestamp, value })
                .collect())
        })
        .await
        .context("Task join failed")?
    }

    // ── Analytics aggregation methods ──

    /// Aggregate full-resolution telemetry (raw rows and packed blocks) into
    /// time buckets of `interval_secs`.
    pub async fn aggregate_raw(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
        interval_secs: i64,
    ) -> Result<Vec<(i64, crate::analytics::AggregatedMetric)>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            anyhow::ensure!(interval_secs > 0, "interval_secs must be positive");

            // Samples arrive ascending, so each bucket is one contiguous run.
            // The standard deviation is left to the API layer, as before.
            let mut buckets: Vec<(i64, super::Agg)> = Vec::new();
            super::historian::scan_series(&conn, &device_id, &metric, from, to, |ts, v| {
                let bucket = (ts / interval_secs) * interval_secs;
                match buckets.last_mut() {
                    Some((b, agg)) if *b == bucket => agg.add(ts, v),
                    _ => {
                        let mut agg = super::Agg::new();
                        agg.add(ts, v);
                        buckets.push((bucket, agg));
                    }
                }
                true
            })
            .context("Read aggregate_raw samples failed")?;
            Ok(buckets
                .into_iter()
                .map(|(bucket, agg)| {
                    (
                        bucket,
                        crate::analytics::AggregatedMetric {
                            metric: metric.clone(),
                            min: agg.min,
                            max: agg.max,
                            avg: agg.mean(),
                            stddev: 0.0,
                            count: agg.count,
                            first_ts: agg.first_ts,
                            last_ts: agg.last_ts,
                        },
                    )
                })
                .collect())
        })
        .await
        .context("Task join failed")?
    }

    /// Aggregate hourly telemetry data into time buckets.
    pub async fn aggregate_hourly(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
        interval_secs: i64,
    ) -> Result<Vec<(i64, crate::analytics::AggregatedMetric)>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT
                    (bucket_start / ?5) * ?5 as bucket,
                    MIN(min_value),
                    MAX(max_value),
                    CASE WHEN SUM(sample_count) > 0
                         THEN SUM(avg_value * sample_count) * 1.0 / SUM(sample_count)
                         ELSE AVG(avg_value) END,
                    SUM(sample_count),
                    MIN(bucket_start),
                    MAX(bucket_start)
                 FROM telemetry_hourly
                 WHERE device_id = ?1 AND metric = ?2
                 AND bucket_start >= ?3 AND bucket_start <= ?4
                 GROUP BY bucket
                 ORDER BY bucket",
                )
                .context("Prepare aggregate_hourly failed")?;

            let rows = stmt
                .query_map(
                    rusqlite::params![device_id, metric, from, to, interval_secs],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            crate::analytics::AggregatedMetric {
                                metric: metric.clone(),
                                min: row.get(1)?,
                                max: row.get(2)?,
                                avg: row.get(3)?,
                                stddev: 0.0,
                                count: row.get::<_, i64>(4)? as u64,
                                first_ts: row.get(5)?,
                                last_ts: row.get(6)?,
                            },
                        ))
                    },
                )
                .context("Query aggregate_hourly failed")?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.context("Row parse aggregate_hourly failed")?);
            }
            Ok(results)
        })
        .await
        .context("Task join failed")?
    }

    /// Aggregate a single time period into one AggregatedMetric.
    pub async fn aggregate_single_period(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
    ) -> Result<crate::analytics::AggregatedMetric> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let mut agg = super::Agg::new();
            super::historian::scan_series(&conn, &device_id, &metric, from, to, |ts, v| {
                agg.add(ts, v);
                true
            })
            .context("Read aggregate_single_period samples failed")?;
            // No samples: zeros over the requested period.
            if agg.count == 0 {
                return Ok(crate::analytics::AggregatedMetric {
                    metric,
                    min: 0.0,
                    max: 0.0,
                    avg: 0.0,
                    stddev: 0.0,
                    count: 0,
                    first_ts: from,
                    last_ts: to,
                });
            }
            Ok(crate::analytics::AggregatedMetric {
                metric,
                min: agg.min,
                max: agg.max,
                avg: agg.mean(),
                stddev: 0.0,
                count: agg.count,
                first_ts: agg.first_ts,
                last_ts: agg.last_ts,
            })
        })
        .await
        .context("Task join failed")?
    }
}
