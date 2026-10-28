use super::Storage;
use anyhow::{Context, Result};

use crate::models::Setting;

impl Storage {
    // ── Settings ──

    pub async fn get_settings(&self, keys: &[&str]) -> Result<Vec<Setting>> {
        let conn = self.conn.clone();
        let keys: Vec<String> = keys.iter().map(|s| s.to_string()).collect();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let placeholders: Vec<String> = keys
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect();
            let sql = format!(
                "SELECT key, value FROM settings WHERE key IN ({})",
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql).context("Prepare failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = keys
                .iter()
                .map(|k| k as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(Setting {
                        key: row.get(0)?,
                        value: row.get(1)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn upsert_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.clone();
        let key = key.to_string();
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
            .context("Upsert setting failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_setting(&self, key: &str) -> Result<()> {
        let conn = self.conn.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM settings WHERE key = ?1", rusqlite::params![key])
                .context("Delete setting failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── User Preferences ──

    /// Get a user-scoped preference value. Returns None if not set.
    pub async fn get_user_preference(&self, username: &str, key: &str) -> Result<Option<String>> {
        let settings = self
            .get_settings(&[&format!("user:{}:{}", username, key)])
            .await?;
        Ok(settings.into_iter().next().map(|s| s.value))
    }

    /// Set a user-scoped preference value.
    pub async fn set_user_preference(&self, username: &str, key: &str, value: &str) -> Result<()> {
        self.upsert_setting(&format!("user:{}:{}", username, key), value)
            .await
    }
}
