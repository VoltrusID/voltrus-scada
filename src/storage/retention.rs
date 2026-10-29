use super::Storage;
use anyhow::Context;

/// Storage statistics returned by get_compression_stats.
pub struct CompressionStats {
    /// Raw full-resolution rows not yet packed.
    pub hot_rows: i64,
    /// Hourly aggregate rows.
    pub warm_rows: i64,
    /// Daily aggregate rows.
    pub cold_rows: i64,
    /// Packed blocks and the samples and payload bytes they hold.
    pub packed: super::PackedStats,
    pub db_size_bytes: u64,
}

impl Storage {
    /// Get row counts and sizes across all tiers.
    pub async fn get_compression_stats(&self) -> anyhow::Result<CompressionStats> {
        let storage = self.clone();
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();

            let hot_rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM telemetry", [], |row| row.get(0))
                .unwrap_or(0);

            let warm_rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM telemetry_hourly", [], |row| {
                    row.get(0)
                })
                .unwrap_or(0);

            let cold_rows: i64 = conn
                .query_row("SELECT COUNT(*) FROM telemetry_daily", [], |row| row.get(0))
                .unwrap_or(0);

            let packed = super::historian::packed_stats(&conn)?;

            let db_size_bytes = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

            Ok(CompressionStats {
                hot_rows,
                warm_rows,
                cold_rows,
                packed,
                db_size_bytes,
            })
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_custom_metrics(&self) -> anyhow::Result<Vec<crate::models::CustomMetric>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, description, formula, unit, tag_refs, validation_error, created_at, updated_at FROM custom_metrics ORDER BY name",
            )?;
            let metrics = stmt
                .query_map([], |row| {
                    Ok(crate::models::CustomMetric {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        description: row.get(2)?,
                        formula: row.get(3)?,
                        unit: row.get(4)?,
                        tag_refs: row.get(5)?,
                        validation_error: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(metrics)
        }).await.context("Task join failed")?
    }

    pub async fn create_custom_metric(
        &self,
        name: &str,
        description: &str,
        formula: &str,
        unit: Option<&str>,
        tag_refs: &str,
        validation_error: Option<&str>,
    ) -> anyhow::Result<crate::models::CustomMetric> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let description = description.to_string();
        let formula = formula.to_string();
        let unit = unit.map(String::from);
        let tag_refs = tag_refs.to_string();
        let validation_error = validation_error.map(String::from);

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO custom_metrics (name, description, formula, unit, tag_refs, validation_error, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![name, description, formula, unit, tag_refs, validation_error, now, now],
            ).context("Insert failed")?;
            let id = conn.last_insert_rowid();
            Ok(crate::models::CustomMetric {
                id, name, description, formula, unit, tag_refs, validation_error,
                created_at: now, updated_at: now,
            })
        }).await.context("Task join failed")?
    }

    pub async fn update_custom_metric(
        &self,
        id: i64,
        name: Option<&str>,
        description: Option<&str>,
        formula: Option<&str>,
        unit: Option<Option<&str>>,
        tag_refs: Option<&str>,
        validation_error: Option<Option<&str>>,
    ) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        let name = name.map(String::from);
        let description = description.map(String::from);
        let formula = formula.map(String::from);
        let unit = unit.map(|o| o.map(String::from));
        let tag_refs = tag_refs.map(String::from);
        let validation_error = validation_error.map(|o| o.map(String::from));

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let mut set_clauses = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref v) = name {
                set_clauses.push("name = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = description {
                set_clauses.push("description = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = formula {
                set_clauses.push("formula = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = unit {
                match v {
                    Some(val) => {
                        set_clauses.push("unit = ?".to_string());
                        params.push(Box::new(val.clone()));
                    }
                    None => {
                        set_clauses.push("unit = NULL".to_string());
                    }
                }
            }
            if let Some(ref v) = tag_refs {
                set_clauses.push("tag_refs = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref v) = validation_error {
                match v {
                    Some(val) => {
                        set_clauses.push("validation_error = ?".to_string());
                        params.push(Box::new(val.clone()));
                    }
                    None => {
                        set_clauses.push("validation_error = NULL".to_string());
                    }
                }
            }

            if set_clauses.is_empty() {
                return Ok(());
            }

            set_clauses.push("updated_at = ?".to_string());
            params.push(Box::new(now));

            let sql = format!(
                "UPDATE custom_metrics SET {} WHERE id = ?",
                set_clauses.join(", ")
            );
            params.push(Box::new(id));

            conn.execute(
                &sql,
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
            )
            .context("Update failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_custom_metric(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM custom_metrics WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Full-resolution samples of one tag in `[from, to]`, oldest first; raw
    /// rows and packed blocks merged. For the statistics endpoints: a window
    /// with more than 10000 samples comes back as an evenly spaced sample of
    /// the whole window (every k-th, plus the last), not its first 10000.
    pub async fn get_metric_values_for_range(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
    ) -> anyhow::Result<Vec<(i64, f64)>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            super::historian::read_series_thinned(
                &conn,
                &device_id,
                &metric,
                from,
                to,
                super::historian::THIN_CAP,
                super::historian::Thin::Stride,
            )
        })
        .await
        .context("Task join failed")?
    }
}
