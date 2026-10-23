use super::Storage;
use crate::integration::egress::EgressJob;
use anyhow::{Context, Result};

impl Storage {
    pub async fn get_enabled_egress_jobs(&self) -> Result<Vec<EgressJob>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, schedule, format, destination, device_ids, time_range, compression, last_run, last_status, last_error, created_at, updated_at
                 FROM egress_jobs WHERE enabled = 1 ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| Ok(EgressJob {
                id: row.get(0)?, name: row.get(1)?, enabled: row.get::<_, i32>(2)? != 0,
                schedule: row.get(3)?, format: row.get(4)?, destination: row.get(5)?,
                device_ids: row.get(6)?, time_range: row.get(7)?, compression: row.get::<_, i32>(8)? != 0,
                last_run: row.get(9)?, last_status: row.get(10)?, last_error: row.get(11)?,
                created_at: row.get(12)?, updated_at: row.get(13)?,
            }))?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    pub async fn list_egress_jobs(&self) -> Result<Vec<EgressJob>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, schedule, format, destination, device_ids, time_range, compression, last_run, last_status, last_error, created_at, updated_at
                 FROM egress_jobs ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| Ok(EgressJob {
                id: row.get(0)?, name: row.get(1)?, enabled: row.get::<_, i32>(2)? != 0,
                schedule: row.get(3)?, format: row.get(4)?, destination: row.get(5)?,
                device_ids: row.get(6)?, time_range: row.get(7)?, compression: row.get::<_, i32>(8)? != 0,
                last_run: row.get(9)?, last_status: row.get(10)?, last_error: row.get(11)?,
                created_at: row.get(12)?, updated_at: row.get(13)?,
            }))?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_egress_job(&self, id: i64) -> Result<Option<EgressJob>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, schedule, format, destination, device_ids, time_range, compression, last_run, last_status, last_error, created_at, updated_at
                 FROM egress_jobs WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(rusqlite::params![id], |row| Ok(EgressJob {
                id: row.get(0)?, name: row.get(1)?, enabled: row.get::<_, i32>(2)? != 0,
                schedule: row.get(3)?, format: row.get(4)?, destination: row.get(5)?,
                device_ids: row.get(6)?, time_range: row.get(7)?, compression: row.get::<_, i32>(8)? != 0,
                last_run: row.get(9)?, last_status: row.get(10)?, last_error: row.get(11)?,
                created_at: row.get(12)?, updated_at: row.get(13)?,
            }))?;
            match rows.next() { Some(row) => Ok(Some(row?)), None => Ok(None) }
        }).await.context("Task join failed")?
    }

    pub async fn create_egress_job(
        &self,
        name: &str,
        enabled: bool,
        schedule: &str,
        format: &str,
        destination: &str,
        device_ids: &str,
        time_range: &str,
        compression: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let schedule = schedule.to_string();
        let format = format.to_string();
        let destination = destination.to_string();
        let device_ids = device_ids.to_string();
        let time_range = time_range.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO egress_jobs (name, enabled, schedule, format, destination, device_ids, time_range, compression, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![name, enabled as i32, schedule, format, destination, device_ids, time_range, compression as i32, now, now],
            ).context("Insert egress job failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn update_egress_job(
        &self,
        id: i64,
        name: Option<&str>,
        enabled: Option<bool>,
        schedule: Option<&str>,
        format: Option<&str>,
        destination: Option<&str>,
        device_ids: Option<&str>,
        time_range: Option<&str>,
        compression: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let schedule = schedule.map(|s| s.to_string());
        let format = format.map(|s| s.to_string());
        let destination = destination.map(|s| s.to_string());
        let device_ids = device_ids.map(|s| s.to_string());
        let time_range = time_range.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            if let Some(ref v) = name {
                conn.execute(
                    "UPDATE egress_jobs SET name=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(v) = enabled {
                conn.execute(
                    "UPDATE egress_jobs SET enabled=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v as i32, now, id],
                )?;
            }
            if let Some(ref v) = schedule {
                conn.execute(
                    "UPDATE egress_jobs SET schedule=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(ref v) = format {
                conn.execute(
                    "UPDATE egress_jobs SET format=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(ref v) = destination {
                conn.execute(
                    "UPDATE egress_jobs SET destination=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(ref v) = device_ids {
                conn.execute(
                    "UPDATE egress_jobs SET device_ids=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(ref v) = time_range {
                conn.execute(
                    "UPDATE egress_jobs SET time_range=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v, now, id],
                )?;
            }
            if let Some(v) = compression {
                conn.execute(
                    "UPDATE egress_jobs SET compression=?1, updated_at=?2 WHERE id=?3",
                    rusqlite::params![v as i32, now, id],
                )?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_egress_job_status(
        &self,
        id: i64,
        last_run: Option<i64>,
        last_status: Option<String>,
        last_error: Option<String>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("UPDATE egress_jobs SET last_run=?1, last_status=?2, last_error=?3, updated_at=?4 WHERE id=?5", rusqlite::params![last_run, last_status, last_error, crate::timeutil::now_secs(), id])
                .context("Update egress job status failed").map(|_| ())
        }).await.context("Task join failed")?
    }

    pub async fn delete_egress_job(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM egress_jobs WHERE id=?1", rusqlite::params![id])
                .context("Delete egress job failed")
                .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_alarm_counts_by_severity(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT severity, COUNT(*) FROM alarms WHERE resolved_at IS NULL GROUP BY severity",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row?);
            }
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    // ── Outbound Webhooks (v0.39.0) ──

    pub async fn list_outbound_webhooks(&self) -> Result<Vec<serde_json::Value>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, url, events, device_filter, headers, enabled, retry_count, timeout_ms, last_triggered, last_status, created_at
                 FROM outbound_webhooks ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?, "name": row.get::<_, String>(1)?, "url": row.get::<_, String>(2)?,
                "events": row.get::<_, String>(3)?, "device_filter": row.get::<_, String>(4)?, "headers": row.get::<_, String>(5)?,
                "enabled": row.get::<_, i32>(6)? != 0, "retry_count": row.get::<_, u32>(7)?, "timeout_ms": row.get::<_, u64>(8)?,
                "last_triggered": row.get::<_, Option<i64>>(9)?, "last_status": row.get::<_, Option<String>>(10)?, "created_at": row.get::<_, i64>(11)?,
            })))?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    pub async fn get_outbound_webhook(&self, id: i64) -> Result<Option<serde_json::Value>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, url, events, device_filter, headers, enabled, retry_count, timeout_ms, last_triggered, last_status, created_at
                 FROM outbound_webhooks WHERE id = ?1",
            )?;
            let mut rows = stmt.query_map(rusqlite::params![id], |row| Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?, "name": row.get::<_, String>(1)?, "url": row.get::<_, String>(2)?,
                "events": row.get::<_, String>(3)?, "device_filter": row.get::<_, String>(4)?, "headers": row.get::<_, String>(5)?,
                "enabled": row.get::<_, i32>(6)? != 0, "retry_count": row.get::<_, u32>(7)?, "timeout_ms": row.get::<_, u64>(8)?,
                "last_triggered": row.get::<_, Option<i64>>(9)?, "last_status": row.get::<_, Option<String>>(10)?, "created_at": row.get::<_, i64>(11)?,
            })))?;
            match rows.next() { Some(row) => Ok(Some(row?)), None => Ok(None) }
        }).await.context("Task join failed")?
    }

    pub async fn create_outbound_webhook(
        &self,
        name: &str,
        url: &str,
        events: &str,
        device_filter: &str,
        headers: &str,
        enabled: bool,
        retry_count: u32,
        timeout_ms: u64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        let events = events.to_string();
        let device_filter = device_filter.to_string();
        let headers = headers.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO outbound_webhooks (name, url, events, device_filter, headers, enabled, retry_count, timeout_ms, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![name, url, events, device_filter, headers, enabled as i32, retry_count, timeout_ms, now],
            ).context("Insert outbound webhook failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn delete_outbound_webhook(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM outbound_webhooks WHERE id=?1",
                rusqlite::params![id],
            )
            .context("Delete outbound webhook failed")
            .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_outbound_webhook_status(
        &self,
        id: i64,
        last_triggered: Option<i64>,
        last_status: Option<String>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE outbound_webhooks SET last_triggered=?1, last_status=?2 WHERE id=?3",
                rusqlite::params![last_triggered, last_status, id],
            )
            .context("Update outbound webhook status failed")
            .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_outbound_webhooks_for_event(
        &self,
        event: &str,
    ) -> Result<Vec<serde_json::Value>> {
        let all = self.list_outbound_webhooks().await?;
        Ok(all
            .into_iter()
            .filter(|w| {
                let enabled = w.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                if !enabled {
                    return false;
                }
                let events = w.get("events").and_then(|v| v.as_str()).unwrap_or("[]");
                events.contains(event)
            })
            .collect())
    }
}
