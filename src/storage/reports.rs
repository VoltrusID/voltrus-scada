use super::Storage;
use anyhow::{Context, Result};

use crate::models::Report;

impl Storage {
    // ── Report management ──

    pub async fn list_reports(&self) -> Result<Vec<Report>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, schedule, enabled, device_ids, email_recipients, last_generated, created_at FROM reports ORDER BY id"
            ).context("Prepare list_reports failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(Report {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    schedule: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    device_ids: row.get(4)?,
                    email_recipients: row.get(5)?,
                    last_generated: row.get(6)?,
                    created_at: row.get(7)?,
                })
            }).context("Query list_reports failed")?;
            let mut reports = Vec::new();
            for row in rows {
                reports.push(row.context("Row parse list_reports failed")?);
            }
            Ok(reports)
        }).await.context("Task join failed")?
    }

    pub async fn get_report(&self, id: i64) -> Result<Option<Report>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, schedule, enabled, device_ids, email_recipients, last_generated, created_at FROM reports WHERE id = ?1"
            ).context("Prepare get_report failed")?;
            let result = stmt.query_row(rusqlite::params![id], |row| {
                Ok(Report {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    schedule: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    device_ids: row.get(4)?,
                    email_recipients: row.get(5)?,
                    last_generated: row.get(6)?,
                    created_at: row.get(7)?,
                })
            }).ok();
            Ok(result)
        }).await.context("Task join failed")?
    }

    pub async fn create_report(
        &self,
        name: &str,
        schedule: &str,
        enabled: bool,
        device_ids: &str,
        email_recipients: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let schedule = schedule.to_string();
        let device_ids = device_ids.to_string();
        let email_recipients = email_recipients.to_string();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO reports (name, schedule, enabled, device_ids, email_recipients, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![name, schedule, enabled as i64, device_ids, email_recipients, now],
            ).context("Insert report failed")?;
            Ok(conn.last_insert_rowid())
        }).await.context("Task join failed")?
    }

    pub async fn update_report(
        &self,
        id: i64,
        name: Option<&str>,
        schedule: Option<&str>,
        device_ids: Option<&str>,
        email_recipients: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let schedule = schedule.map(|s| s.to_string());
        let device_ids = device_ids.map(|s| s.to_string());
        let email_recipients = email_recipients.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = name {
                conn.execute(
                    "UPDATE reports SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update report name failed")?;
            }
            if let Some(schedule) = schedule {
                conn.execute(
                    "UPDATE reports SET schedule = ?1 WHERE id = ?2",
                    rusqlite::params![schedule, id],
                )
                .context("Update report schedule failed")?;
            }
            if let Some(device_ids) = device_ids {
                conn.execute(
                    "UPDATE reports SET device_ids = ?1 WHERE id = ?2",
                    rusqlite::params![device_ids, id],
                )
                .context("Update report device_ids failed")?;
            }
            if let Some(email_recipients) = email_recipients {
                conn.execute(
                    "UPDATE reports SET email_recipients = ?1 WHERE id = ?2",
                    rusqlite::params![email_recipients, id],
                )
                .context("Update report email_recipients failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE reports SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update report enabled failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_report(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM reports WHERE id = ?1", rusqlite::params![id])
                .context("Delete report failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_report_last_generated(&self, id: i64, ts: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE reports SET last_generated = ?1 WHERE id = ?2",
                rusqlite::params![ts, id],
            )
            .context("Update report last_generated failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
