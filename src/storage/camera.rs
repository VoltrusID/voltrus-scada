use super::Storage;
use anyhow::{Context, Result};

use crate::models::{Camera, CameraSnapshot};

impl Storage {
    // ── Camera CRUD ──

    pub async fn create_camera(
        &self,
        name: &str,
        url: &str,
        enabled: bool,
        snapshot_on_alarm: bool,
        retention_hours: u32,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO cameras (name, url, enabled, snapshot_on_alarm, retention_hours, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![name, url, enabled as i64, snapshot_on_alarm as i64, retention_hours, now],
            )
            .context("Insert camera failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_cameras(&self) -> Result<Vec<Camera>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, enabled, snapshot_on_alarm, retention_hours, created_at FROM cameras ORDER BY created_at",
                )
                .context("Prepare list_cameras failed")?;

            let rows = stmt.query_map([], |row| {
                Ok(Camera {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    enabled: row.get::<_, i64>(3)? != 0,
                    snapshot_on_alarm: row.get::<_, i64>(4)? != 0,
                    retention_hours: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .context("Query list_cameras failed")?;

            let mut cameras = Vec::new();
            for row in rows {
                cameras.push(row.context("Row list_cameras failed")?);
            }
            Ok(cameras)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_camera(&self, id: i64) -> Result<Option<Camera>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, enabled, snapshot_on_alarm, retention_hours, created_at FROM cameras WHERE id = ?1",
                )
                .context("Prepare get_camera failed")?;

            let mut rows = stmt
                .query_map(rusqlite::params![id], |row| {
                    Ok(Camera {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        enabled: row.get::<_, i64>(3)? != 0,
                        snapshot_on_alarm: row.get::<_, i64>(4)? != 0,
                        retention_hours: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .context("Query get_camera failed")?;

            match rows.next() {
                Some(row) => Ok(Some(row.context("Row get_camera failed")?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_camera(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        enabled: Option<bool>,
        snapshot_on_alarm: Option<bool>,
        retention_hours: Option<u32>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &name {
                conn.execute(
                    "UPDATE cameras SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update camera name failed")?;
            }
            if let Some(url) = &url {
                conn.execute(
                    "UPDATE cameras SET url = ?1 WHERE id = ?2",
                    rusqlite::params![url, id],
                )
                .context("Update camera url failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE cameras SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update camera enabled failed")?;
            }
            if let Some(snapshot_on_alarm) = snapshot_on_alarm {
                conn.execute(
                    "UPDATE cameras SET snapshot_on_alarm = ?1 WHERE id = ?2",
                    rusqlite::params![snapshot_on_alarm as i64, id],
                )
                .context("Update camera snapshot_on_alarm failed")?;
            }
            if let Some(retention_hours) = retention_hours {
                conn.execute(
                    "UPDATE cameras SET retention_hours = ?1 WHERE id = ?2",
                    rusqlite::params![retention_hours, id],
                )
                .context("Update camera retention_hours failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_camera(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Get file paths for cleanup
            let mut stmt = conn
                .prepare("SELECT file_path FROM camera_snapshots WHERE camera_id = ?1")
                .ok();
            if let Some(ref mut s) = stmt {
                let rows = s
                    .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
                    .ok();
                if let Some(rows) = rows {
                    for row in rows.flatten() {
                        let _ = std::fs::remove_file(&row);
                    }
                }
            }
            conn.execute("DELETE FROM cameras WHERE id = ?1", rusqlite::params![id])
                .context("Delete camera failed")
                .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }

    // ── Camera Snapshot CRUD ──

    pub async fn create_camera_snapshot(
        &self,
        camera_id: i64,
        timestamp: i64,
        alarm_source: Option<&str>,
        file_path: &str,
        file_size: u64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let alarm_source = alarm_source.map(|s| s.to_string());
        let file_path = file_path.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO camera_snapshots (camera_id, timestamp, alarm_source, file_path, file_size, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![camera_id, timestamp, alarm_source, file_path, file_size, crate::timeutil::now_secs()],
            )
            .context("Insert camera snapshot failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_camera_snapshots(
        &self,
        camera_id: i64,
        limit: usize,
    ) -> Result<Vec<CameraSnapshot>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, camera_id, timestamp, alarm_source, file_path, file_size
                     FROM camera_snapshots
                     WHERE camera_id = ?1
                     ORDER BY timestamp DESC
                     LIMIT ?2",
                )
                .context("Prepare list_camera_snapshots failed")?;

            let rows = stmt
                .query_map(rusqlite::params![camera_id, limit as i64], |row| {
                    Ok(CameraSnapshot {
                        id: row.get(0)?,
                        camera_id: row.get(1)?,
                        timestamp: row.get(2)?,
                        alarm_source: row.get(3)?,
                        file_path: row.get(4)?,
                        file_size: row.get(5)?,
                    })
                })
                .context("Query list_camera_snapshots failed")?;

            let mut snapshots = Vec::new();
            for row in rows {
                snapshots.push(row.context("Row list_camera_snapshots failed")?);
            }
            Ok(snapshots)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_camera_snapshot(&self, snapshot_id: i64) -> Result<Option<CameraSnapshot>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, camera_id, timestamp, alarm_source, file_path, file_size
                     FROM camera_snapshots WHERE id = ?1",
                )
                .context("Prepare get_camera_snapshot failed")?;

            let mut rows = stmt
                .query_map(rusqlite::params![snapshot_id], |row| {
                    Ok(CameraSnapshot {
                        id: row.get(0)?,
                        camera_id: row.get(1)?,
                        timestamp: row.get(2)?,
                        alarm_source: row.get(3)?,
                        file_path: row.get(4)?,
                        file_size: row.get(5)?,
                    })
                })
                .context("Query get_camera_snapshot failed")?;

            match rows.next() {
                Some(row) => Ok(Some(row.context("Row get_camera_snapshot failed")?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_camera_snapshot(&self, snapshot_id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Get file path for cleanup
            let file_path: Option<String> = conn
                .query_row(
                    "SELECT file_path FROM camera_snapshots WHERE id = ?1",
                    rusqlite::params![snapshot_id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(path) = &file_path {
                let _ = std::fs::remove_file(path);
            }
            conn.execute(
                "DELETE FROM camera_snapshots WHERE id = ?1",
                rusqlite::params![snapshot_id],
            )
            .context("Delete camera snapshot failed")
            .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }

    /// Delete snapshots older than the camera's retention period.
    pub async fn cleanup_expired_snapshots(
        &self,
        camera_id: i64,
        retention_hours: u32,
    ) -> Result<u64> {
        let cutoff = crate::timeutil::now_secs() - (retention_hours as i64 * 3600);
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Get file paths for cleanup
            let mut stmt = conn
                .prepare("SELECT file_path FROM camera_snapshots WHERE camera_id = ?1 AND timestamp < ?2")
                .ok();
            if let Some(ref mut s) = stmt {
                let rows = s.query_map(rusqlite::params![camera_id, cutoff], |row| {
                    row.get::<_, String>(0)
                }).ok();
                if let Some(rows) = rows {
                    for row in rows.flatten() {
                        let _ = std::fs::remove_file(&row);
                    }
                }
            }
            conn.execute(
                "DELETE FROM camera_snapshots WHERE camera_id = ?1 AND timestamp < ?2",
                rusqlite::params![camera_id, cutoff],
            )
            .context("Cleanup expired snapshots failed")
            .map(|n| n as u64)
        })
        .await
        .context("Task join failed")?
    }
}
