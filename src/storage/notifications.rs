use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    // ── Notification Settings ──

    pub async fn get_notification_settings_by_source(
        &self,
        source_key: &str,
    ) -> Result<Vec<crate::models::NotificationSetting>> {
        let conn = self.conn.clone();
        let source_key = source_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, source_key, channel_type, config, enabled, created_at
                 FROM notification_settings
                 WHERE source_key = ?1",
            )?;
            let rows = stmt.query_map(rusqlite::params![source_key], |row| {
                Ok(crate::models::NotificationSetting {
                    id: row.get(0)?,
                    source_key: row.get(1)?,
                    channel_type: row.get(2)?,
                    config: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                })
            })?;
            let mut settings = Vec::new();
            for row in rows {
                settings.push(row?);
            }
            Ok(settings)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_all_notification_settings(
        &self,
    ) -> Result<Vec<crate::models::NotificationSetting>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, source_key, channel_type, config, enabled, created_at
                 FROM notification_settings
                 ORDER BY id",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(crate::models::NotificationSetting {
                    id: row.get(0)?,
                    source_key: row.get(1)?,
                    channel_type: row.get(2)?,
                    config: row.get(3)?,
                    enabled: row.get::<_, i32>(4)? != 0,
                    created_at: row.get(5)?,
                })
            })?;
            let mut settings = Vec::new();
            for row in rows {
                settings.push(row?);
            }
            Ok(settings)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_notification_setting(
        &self,
        source_key: &str,
        channel_type: &str,
        config: &str,
        enabled: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let source_key = source_key.to_string();
        let channel_type = channel_type.to_string();
        let config = config.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO notification_settings (source_key, channel_type, config, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![source_key, channel_type, config, enabled as i32, now],
            )
            .context("Insert notification setting failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_notification_setting(
        &self,
        id: i64,
        source_key: Option<&str>,
        channel_type: Option<&str>,
        config: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let source_key = source_key.map(|s| s.to_string());
        let channel_type = channel_type.map(|s| s.to_string());
        let config = config.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(ref sk) = source_key {
                conn.execute(
                    "UPDATE notification_settings SET source_key = ?1 WHERE id = ?2",
                    rusqlite::params![sk, id],
                )?;
            }
            if let Some(ref ct) = channel_type {
                conn.execute(
                    "UPDATE notification_settings SET channel_type = ?1 WHERE id = ?2",
                    rusqlite::params![ct, id],
                )?;
            }
            if let Some(ref c) = config {
                conn.execute(
                    "UPDATE notification_settings SET config = ?1 WHERE id = ?2",
                    rusqlite::params![c, id],
                )?;
            }
            if let Some(e) = enabled {
                conn.execute(
                    "UPDATE notification_settings SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![e as i32, id],
                )?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_notification_setting(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM notification_settings WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete notification setting failed")
            .map(|_| ())
        })
        .await
        .context("Task join failed")?
    }
}
