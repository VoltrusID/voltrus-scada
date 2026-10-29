use super::{Storage, historian};
use anyhow::{Context, Result};

impl Storage {
    // ── Downsampling & Retention ──

    /// Aggregate every full-resolution sample (packed and raw) older than
    /// `cutoff` into hourly buckets. Returns the number of hourly rows written.
    ///
    /// `cutoff` should sit on an hour boundary: an hour that is cut in half is
    /// written from the half that is older than `cutoff`.
    pub async fn downsample_raw_to_hourly(&self, cutoff: i64) -> Result<u64> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let series = {
                let conn = storage.lock_conn();
                historian::series_before(&conn, cutoff)?
            };
            let mut written = 0u64;
            for (device_id, metric) in &series {
                let conn = storage.lock_conn();
                let hours = historian::aggregate_hours_before(&conn, device_id, metric, cutoff)?;
                let tx = conn
                    .unchecked_transaction()
                    .context("Begin downsample tx failed")?;
                for (hour, agg) in &hours {
                    write_hourly(&tx, device_id, metric, *hour, agg)?;
                    written += 1;
                }
                tx.commit().context("Commit downsample tx failed")?;
            }
            Ok(written)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn downsample_hourly_to_daily(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // The daily mean weighs each hour by its sample count.
            conn.execute(
                "INSERT OR REPLACE INTO telemetry_daily (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
                 SELECT device_id, metric,
                        (bucket_start / 86400) * 86400 as day_start,
                        CASE WHEN SUM(sample_count) > 0
                             THEN SUM(avg_value * sample_count) / SUM(sample_count)
                             ELSE AVG(avg_value) END,
                        MIN(min_value), MAX(max_value), SUM(sample_count)
                 FROM telemetry_hourly
                 WHERE bucket_start < ?1
                 GROUP BY device_id, metric, day_start",
                rusqlite::params![cutoff],
            ).context("Downsample hourly to daily failed").map(|n| n as u64)
        }).await.context("Task join failed")?
    }

    /// Delete every full-resolution sample (raw rows and packed blocks) older
    /// than `cutoff`. Returns the number of samples removed.
    pub async fn delete_raw_older_than(&self, cutoff: i64) -> Result<u64> {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let tx = conn
                .unchecked_transaction()
                .context("Begin delete tx failed")?;
            let raw = tx
                .execute(
                    "DELETE FROM telemetry WHERE timestamp < ?1",
                    rusqlite::params![cutoff],
                )
                .context("Delete raw telemetry failed")? as u64;
            let packed = historian::delete_packed_range(
                &tx,
                None,
                None,
                i64::MIN,
                cutoff.saturating_sub(1),
            )?;
            tx.commit().context("Commit failed")?;
            Ok(raw + packed)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_hourly_older_than(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM telemetry_hourly WHERE bucket_start < ?1",
                rusqlite::params![cutoff],
            )
            .context("Delete hourly telemetry failed")
            .map(|n| n as u64)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_daily_older_than(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM telemetry_daily WHERE bucket_start < ?1",
                rusqlite::params![cutoff],
            )
            .context("Delete daily telemetry failed")
            .map(|n| n as u64)
        })
        .await
        .context("Task join failed")?
    }

    /// Delete the readings of `device_id` (only `metric`'s when given) with
    /// `from <= timestamp <= to`, in seconds. The hourly and daily tiers lose
    /// the buckets that lie wholly inside the range. Returns the samples
    /// deleted from the full-resolution (raw and packed), hourly and daily tiers.
    pub async fn delete_telemetry_range(
        &self,
        device_id: &str,
        metric: Option<&str>,
        from: i64,
        to: i64,
    ) -> Result<(u64, u64, u64)> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.map(str::to_string);
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            // `?2 IS NULL` leaves the metric unrestricted.
            let raw = tx
                .execute(
                    "DELETE FROM telemetry
                     WHERE device_id = ?1 AND (?2 IS NULL OR metric = ?2)
                       AND timestamp >= ?3 AND timestamp <= ?4",
                    rusqlite::params![device_id, metric, from, to],
                )
                .context("Delete raw telemetry failed")? as u64;
            let packed =
                historian::delete_packed_range(&tx, Some(&device_id), metric.as_deref(), from, to)?;
            let hourly = tx
                .execute(
                    "DELETE FROM telemetry_hourly
                     WHERE device_id = ?1 AND (?2 IS NULL OR metric = ?2)
                       AND bucket_start >= ?3 AND bucket_start + 3600 <= ?4 + 1",
                    rusqlite::params![device_id, metric, from, to],
                )
                .context("Delete hourly telemetry failed")?;
            let daily = tx
                .execute(
                    "DELETE FROM telemetry_daily
                     WHERE device_id = ?1 AND (?2 IS NULL OR metric = ?2)
                       AND bucket_start >= ?3 AND bucket_start + 86400 <= ?4 + 1",
                    rusqlite::params![device_id, metric, from, to],
                )
                .context("Delete daily telemetry failed")?;
            tx.commit().context("Commit failed")?;
            Ok((raw + packed, hourly as u64, daily as u64))
        })
        .await
        .context("Task join failed")?
    }

    /// A device's history over `[from, to]` as one series of buckets per tag,
    /// from every retention tier (daily, hourly, packed and raw full
    /// resolution), `bucket_secs` wide. The whole window is always covered:
    /// the answer is as long as the window divided by the bucket width, never
    /// cut short. `metrics` restricts the tags (`None` is all of them).
    ///
    /// The connection is released between tags, so a long window does not hold
    /// up the pollers' inserts for its whole duration.
    pub async fn query_device_series(
        &self,
        device_id: &str,
        from: i64,
        to: i64,
        bucket_secs: i64,
        metrics: Option<std::collections::HashSet<String>>,
    ) -> Result<std::collections::BTreeMap<String, Vec<historian::SeriesBucket>>> {
        let storage = self.clone();
        let device_id = device_id.to_string();

        tokio::task::spawn_blocking(move || {
            let wanted: Vec<String> = {
                let conn = storage.lock_conn();
                historian::series_metrics(&conn, &device_id, from, to)?
                    .into_iter()
                    .filter(|m| metrics.as_ref().is_none_or(|set| set.contains(m)))
                    .collect()
            };
            let mut out = std::collections::BTreeMap::new();
            for metric in wanted {
                let buckets = {
                    let conn = storage.lock_conn();
                    historian::bucket_series(&conn, &device_id, &metric, from, to, bucket_secs)?
                };
                if !buckets.is_empty() {
                    out.insert(metric, buckets);
                }
            }
            Ok(out)
        })
        .await
        .context("Task join failed")?
    }

    /// Every full-resolution sample of a device in `[from, to]`, for exports:
    /// `(timestamp, metric, value)` ascending. Fails when the range holds more
    /// than `max_samples` rather than returning a short answer.
    pub async fn query_device_samples(
        &self,
        device_id: &str,
        from: i64,
        to: i64,
        max_samples: usize,
    ) -> Result<Vec<(i64, String, f64)>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            historian::read_device_all(&conn, &device_id, from, to, max_samples)
        })
        .await
        .context("Task join failed")?
    }

    /// Count total rows matching the export query filters (for pagination meta).
    /// Counts full-resolution samples, raw and packed.
    pub async fn count_telemetry(
        &self,
        device_id: Option<&str>,
        metric: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<i64> {
        let storage = self.clone();
        let device_id = device_id.map(|s| s.to_string());
        let metric = metric.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            historian::count_samples(&conn, device_id.as_deref(), metric.as_deref(), from, to)
        })
        .await
        .context("Task join failed")?
    }

    /// Query telemetry rows for the export API with optional aggregation.
    ///
    /// When `aggregation` is None, returns full-resolution samples (raw rows and
    /// packed blocks merged), newest first. When `aggregation` is
    /// Some("avg"|"min"|"max"|"count"|"sum"), uses the
    /// appropriate aggregate table (`telemetry_hourly` or `telemetry_daily`)
    /// depending on the requested `interval`.
    ///
    /// Returns `(rows, total_count)` where rows are `(timestamp, device_id, metric, value)`.
    pub async fn query_telemetry_export(
        &self,
        device_id: Option<&str>,
        metric: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
        aggregation: Option<&str>,
        interval: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<(i64, String, String, f64)>, i64)> {
        // Full resolution: raw rows and packed blocks merged.
        if aggregation.is_none() || interval.is_none() {
            let storage = self.clone();
            let device_id = device_id.map(|s| s.to_string());
            let metric = metric.map(|s| s.to_string());
            return tokio::task::spawn_blocking(move || {
                let conn = storage.lock_conn();
                let total = historian::count_samples(
                    &conn,
                    device_id.as_deref(),
                    metric.as_deref(),
                    from,
                    to,
                )?;
                let rows = historian::export_page(
                    &conn,
                    device_id.as_deref(),
                    metric.as_deref(),
                    from,
                    to,
                    limit,
                    offset,
                )?;
                Ok((rows, total))
            })
            .await
            .context("Task join failed")?;
        }

        // Determine the target table, time column, and value column up front
        // so the WHERE clause and count query use the correct names.
        let (table, ts_col, val_col) = match (aggregation, interval) {
            (Some(agg), Some(int)) => {
                let val = match (int, agg) {
                    ("1d", "min") => "min_value",
                    ("1d", "max") => "max_value",
                    ("1d", "count") => "sample_count",
                    ("1d", "sum") => "ROUND(avg_value * sample_count, 4)",
                    ("1d", _) => "avg_value",
                    (_, "min") => "min_value",
                    (_, "max") => "max_value",
                    (_, "count") => "sample_count",
                    (_, "sum") => "ROUND(avg_value * sample_count, 4)",
                    (_, _) => "avg_value",
                };
                let tbl = if int == "1d" {
                    "telemetry_daily"
                } else {
                    "telemetry_hourly"
                };
                (tbl, "bucket_start", val)
            }
            _ => unreachable!("full-resolution export returned above"),
        };

        // Determine total count for meta — query the correct table
        let total = self
            .count_telemetry_table(table, ts_col, device_id, metric, from, to)
            .await?;

        let conn = self.conn.clone();
        let device_id = device_id.map(|s| s.to_string());
        let metric = metric.map(|s| s.to_string());
        let table = table.to_string();
        let ts_col = ts_col.to_string();
        let val_col = val_col.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Build WHERE clause using the correct time column for the target table
            let mut where_clauses: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1usize;

            if let Some(ref d) = device_id {
                where_clauses.push(format!("device_id = ?{param_idx}"));
                params.push(Box::new(d.clone()));
                param_idx += 1;
            }
            if let Some(ref m) = metric {
                where_clauses.push(format!("metric = ?{param_idx}"));
                params.push(Box::new(m.clone()));
                param_idx += 1;
            }
            if let Some(f) = from {
                where_clauses.push(format!("{ts_col} >= ?{param_idx}"));
                params.push(Box::new(f));
                param_idx += 1;
            }
            if let Some(t) = to {
                where_clauses.push(format!("{ts_col} <= ?{param_idx}"));
                params.push(Box::new(t));
                param_idx += 1;
            }

            let where_sql = if where_clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", where_clauses.join(" AND "))
            };

            let sql = format!(
                "SELECT {ts_col}, device_id, metric, {val_col} FROM {table} {where_sql} \
                 ORDER BY {ts_col} DESC LIMIT ?{param_idx} OFFSET ?{param_idx2}",
                ts_col = ts_col,
                val_col = val_col,
                table = table,
                where_sql = where_sql,
                param_idx = param_idx,
                param_idx2 = param_idx + 1,
            );
            params.push(Box::new(limit as i64));
            params.push(Box::new(offset as i64));

            let mut stmt = conn.prepare(&sql).context("Prepare export query failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                    ))
                })
                .context("Query export failed")?;
            let rows = rows
                .collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")?;

            Ok((rows, total))
        })
        .await
        .context("Task join failed")?
    }

    /// Count rows in an aggregated telemetry table for pagination meta.
    ///
    /// `table` is `telemetry_hourly` or `telemetry_daily`; `ts_col` is their time
    /// column (`bucket_start`). Full-resolution counts go through
    /// [`Storage::count_telemetry`], which includes packed blocks.
    pub async fn count_telemetry_table(
        &self,
        table: &str,
        ts_col: &str,
        device_id: Option<&str>,
        metric: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let device_id = device_id.map(|s| s.to_string());
        let metric = metric.map(|s| s.to_string());
        let table = table.to_string();
        let ts_col = ts_col.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = format!("SELECT COUNT(*) FROM {table} WHERE 1=1", table = table);
            let mut param_idx = 1usize;

            if let Some(ref _d) = device_id {
                sql.push_str(&format!(" AND device_id = ?{param_idx}"));
                param_idx += 1;
            }
            if let Some(ref _m) = metric {
                sql.push_str(&format!(" AND metric = ?{param_idx}"));
                param_idx += 1;
            }
            if from.is_some() {
                sql.push_str(&format!(" AND {ts_col} >= ?{param_idx}", ts_col = ts_col));
                param_idx += 1;
            }
            if to.is_some() {
                sql.push_str(&format!(" AND {ts_col} <= ?{param_idx}", ts_col = ts_col));
            }

            let mut stmt = conn.prepare(&sql).context("Prepare count failed")?;

            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(ref d) = device_id {
                params.push(Box::new(d.clone()));
            }
            if let Some(ref m) = metric {
                params.push(Box::new(m.clone()));
            }
            if let Some(f) = from {
                params.push(Box::new(f));
            }
            if let Some(t) = to {
                params.push(Box::new(t));
            }

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let count: i64 = stmt
                .query_row(param_refs.as_slice(), |row| row.get(0))
                .context("Count query failed")?;
            Ok(count)
        })
        .await
        .context("Task join failed")?
    }
}

/// Insert or replace one hourly aggregate.
pub(super) fn write_hourly(
    conn: &rusqlite::Connection,
    device_id: &str,
    metric: &str,
    hour: i64,
    agg: &historian::Agg,
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            device_id,
            metric,
            hour,
            agg.mean(),
            agg.min,
            agg.max,
            agg.count as i64
        ],
    )
    .context("Write hourly aggregate failed")?;
    Ok(())
}
