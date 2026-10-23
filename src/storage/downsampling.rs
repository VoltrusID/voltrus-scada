use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    // ── Downsampling & Retention ──

    pub async fn downsample_raw_to_hourly(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO telemetry_hourly (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
                 SELECT device_id, metric,
                        (timestamp / 3600) * 3600 as bucket_start,
                        AVG(value), MIN(value), MAX(value), COUNT(*)
                 FROM telemetry
                 WHERE timestamp < ?1
                 GROUP BY device_id, metric, bucket_start",
                rusqlite::params![cutoff],
            ).context("Downsample raw to hourly failed").map(|n| n as u64)
        }).await.context("Task join failed")?
    }

    pub async fn downsample_hourly_to_daily(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR REPLACE INTO telemetry_daily (device_id, metric, bucket_start, avg_value, min_value, max_value, sample_count)
                 SELECT device_id, metric,
                        (bucket_start / 86400) * 86400 as day_start,
                        AVG(avg_value), MIN(min_value), MAX(max_value), SUM(sample_count)
                 FROM telemetry_hourly
                 WHERE bucket_start < ?1
                 GROUP BY device_id, metric, day_start",
                rusqlite::params![cutoff],
            ).context("Downsample hourly to daily failed").map(|n| n as u64)
        }).await.context("Task join failed")?
    }

    pub async fn delete_raw_older_than(&self, cutoff: i64) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM telemetry WHERE timestamp < ?1",
                rusqlite::params![cutoff],
            )
            .context("Delete raw telemetry failed")
            .map(|n| n as u64)
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

    pub async fn query_raw_for_archive(
        &self,
        cutoff: i64,
    ) -> Result<Vec<(String, String, i64, f64)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT device_id, metric, timestamp, value FROM telemetry
                 WHERE timestamp < ?1 ORDER BY timestamp",
                )
                .context("Prepare archive query failed")?;
            let rows = stmt
                .query_map(rusqlite::params![cutoff], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .context("Query archive rows failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Query telemetry across all retention tiers, transparently merging raw/hourly/daily data.
    ///
    /// Tier time windows:
    /// - Raw:   [raw_cutoff, ∞)  — recent high-resolution data
    /// - Hourly: [hourly_cutoff, raw_cutoff) — mid-range aggregates
    /// - Daily:  (-∞, hourly_cutoff) — long-term aggregates
    ///
    /// `min_interval_secs` controls resolution awareness: if the caller needs data
    /// at coarser intervals than a tier provides, that tier is skipped.
    /// (raw=5s, hourly=3600s, daily=86400s)
    pub async fn query_device_metrics_merged(
        &self,
        device_id: &str,
        from: i64,
        to: i64,
        raw_cutoff: i64,
        hourly_cutoff: i64,
        min_interval_secs: i64,
    ) -> Result<Vec<(i64, std::collections::HashMap<String, f64>)>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut grouped: std::collections::BTreeMap<
                i64,
                std::collections::HashMap<String, f64>,
            > = std::collections::BTreeMap::new();

            // Daily data: (-∞, hourly_cutoff)
            // Always useful at any resolution
            let daily_from = from;
            let daily_to = to.min(hourly_cutoff);
            if daily_from < daily_to {
                let mut stmt = conn
                    .prepare(
                        "SELECT bucket_start, metric, avg_value FROM telemetry_daily
                     WHERE device_id = ?1 AND bucket_start >= ?2 AND bucket_start <= ?3
                     ORDER BY bucket_start LIMIT 10000",
                    )
                    .context("Prepare daily failed")?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id, daily_from, daily_to], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, f64>(2)?,
                        ))
                    })
                    .context("Query daily failed")?;
                let mut count = 0usize;
                for row in rows {
                    let (ts, metric, value) = row.context("Row mapping failed")?;
                    grouped.entry(ts).or_default().insert(metric, value);
                    count += 1;
                }
                if count >= 10000 {
                    tracing::warn!(
                        "query_device_metrics_merged: daily tier hit 10000-row cap for device {device_id}; older daily buckets omitted"
                    );
                }
            }

            // Hourly data: [hourly_cutoff, raw_cutoff)
            // Skip if caller needs daily+ resolution only
            let hourly_from = from.max(hourly_cutoff);
            let hourly_to = to.min(raw_cutoff);
            if min_interval_secs <= 3600 && hourly_from < hourly_to {
                let mut stmt = conn
                    .prepare(
                        "SELECT bucket_start, metric, avg_value FROM telemetry_hourly
                     WHERE device_id = ?1 AND bucket_start >= ?2 AND bucket_start <= ?3
                     ORDER BY bucket_start LIMIT 10000",
                    )
                    .context("Prepare hourly failed")?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![device_id, hourly_from, hourly_to],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, f64>(2)?,
                            ))
                        },
                    )
                    .context("Query hourly failed")?;
                let mut count = 0usize;
                for row in rows {
                    let (ts, metric, value) = row.context("Row mapping failed")?;
                    grouped.entry(ts).or_default().insert(metric, value);
                    count += 1;
                }
                if count >= 10000 {
                    tracing::warn!(
                        "query_device_metrics_merged: hourly tier hit 10000-row cap for device {device_id}; some hourly buckets omitted"
                    );
                }
            }

            // Raw data: [raw_cutoff, ∞)
            // Skip if caller needs hourly+ resolution only (raw is ~5s intervals)
            let raw_from = from.max(raw_cutoff);
            let raw_to = to;
            if min_interval_secs <= 300 && raw_from < raw_to {
                let mut stmt = conn
                    .prepare(
                        "SELECT timestamp, metric, value FROM telemetry
                     WHERE device_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
                     ORDER BY timestamp LIMIT 10000",
                    )
                    .context("Prepare raw failed")?;
                let rows = stmt
                    .query_map(rusqlite::params![device_id, raw_from, raw_to], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, f64>(2)?,
                        ))
                    })
                    .context("Query raw failed")?;
                let mut count = 0usize;
                for row in rows {
                    let (ts, metric, value) = row.context("Row mapping failed")?;
                    grouped.entry(ts).or_default().insert(metric, value);
                    count += 1;
                }
                if count >= 10000 {
                    tracing::warn!(
                        "query_device_metrics_merged: raw tier hit 10000-row cap for device {device_id}; most recent raw samples omitted — narrow the time window"
                    );
                }
            }

            Ok(grouped.into_iter().collect())
        })
        .await
        .context("Task join failed")?
    }

    /// Count total rows matching the export query filters (for pagination meta).
    pub async fn count_telemetry(
        &self,
        device_id: Option<&str>,
        metric: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let device_id = device_id.map(|s| s.to_string());
        let metric = metric.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from("SELECT COUNT(*) FROM telemetry WHERE 1=1");
            if device_id.is_some() {
                sql.push_str(" AND device_id = ?1");
            }
            if metric.is_some() {
                sql.push_str(&format!(
                    " AND metric = ?{}",
                    if device_id.is_some() { 2 } else { 1 }
                ));
            }
            if from.is_some() {
                let idx = device_id.is_some() as usize + metric.is_some() as usize + 1;
                sql.push_str(&format!(" AND timestamp >= ?{idx}"));
            }
            if to.is_some() {
                let idx = device_id.is_some() as usize
                    + metric.is_some() as usize
                    + from.is_some() as usize
                    + 1;
                sql.push_str(&format!(" AND timestamp <= ?{idx}"));
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

    /// Query telemetry rows for the export API with optional aggregation.
    ///
    /// When `aggregation` is None, returns raw rows from the `telemetry` table.
    /// When `aggregation` is Some("avg"|"min"|"max"|"count"|"sum"), uses the
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
            _ => ("telemetry", "timestamp", "value"),
        };

        // Determine total count for meta — query the correct table
        let total = if aggregation.is_some() {
            self.count_telemetry_table(table, ts_col, device_id, metric, from, to)
                .await?
        } else {
            self.count_telemetry(device_id, metric, from, to).await?
        };

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

    /// Count rows in an arbitrary telemetry table (raw or aggregated) for pagination meta.
    ///
    /// `table` is the table name (e.g. "telemetry", "telemetry_hourly", "telemetry_daily").
    /// `ts_col` is the time column name ("timestamp" for raw, "bucket_start" for aggregated).
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
