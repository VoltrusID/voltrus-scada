use super::Storage;
use anyhow::{Context, Result};

use crate::models::{NavShortcut, UserShortcut};

impl Storage {
    // ── Shortcuts ──

    pub async fn get_user_shortcuts(&self, user_id: i64) -> Result<Vec<UserShortcut>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT user_id, data_source_id, shortcut FROM user_shortcuts WHERE user_id = ?1")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![user_id], |row| {
                    Ok(UserShortcut {
                        user_id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        shortcut: row.get(2)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_user_shortcuts(
        &self,
        user_id: i64,
        entries: &[(i64, Option<&str>)],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let entries: Vec<(i64, Option<String>)> = entries
            .iter()
            .map(|(dsid, sc)| (*dsid, sc.map(|s| s.to_string())))
            .collect();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // Ensure table exists (guard against DBs created before this feature)
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS user_shortcuts (
                    user_id INTEGER NOT NULL,
                    data_source_id INTEGER NOT NULL,
                    shortcut TEXT NOT NULL,
                    PRIMARY KEY (user_id, data_source_id)
                )",
            )
            .context("Ensure user_shortcuts table failed")?;
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM user_shortcuts WHERE user_id = ?1",
                rusqlite::params![user_id],
            )
            .context("Delete shortcuts failed")?;
            for (data_source_id, shortcut) in &entries {
                let sc = shortcut.as_deref().unwrap_or("");
                tx.execute(
                    "INSERT INTO user_shortcuts (user_id, data_source_id, shortcut) VALUES (?1, ?2, ?3)",
                    rusqlite::params![user_id, data_source_id, sc],
                )
                .with_context(|| format!("Insert shortcut failed: user_id={user_id}, data_source_id={data_source_id}, shortcut={sc:?}"))?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Nav Shortcuts ──

    pub async fn get_user_nav_shortcuts(&self, user_id: i64) -> Result<Vec<NavShortcut>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS user_nav_shortcuts (
                    user_id INTEGER NOT NULL,
                    target TEXT NOT NULL,
                    shortcut TEXT NOT NULL,
                    PRIMARY KEY (user_id, target)
                )",
            )
            .context("Ensure user_nav_shortcuts table failed")?;
            let mut stmt = conn
                .prepare(
                    "SELECT user_id, target, shortcut FROM user_nav_shortcuts WHERE user_id = ?1",
                )
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![user_id], |row| {
                    Ok(NavShortcut {
                        user_id: row.get(0)?,
                        target: row.get(1)?,
                        shortcut: row.get(2)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn save_user_nav_shortcuts(
        &self,
        user_id: i64,
        entries: &[(String, Option<&str>)],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let entries: Vec<(String, Option<String>)> = entries
            .iter()
            .map(|(t, sc)| (t.clone(), sc.map(|s| s.to_string())))
            .collect();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS user_nav_shortcuts (
                    user_id INTEGER NOT NULL,
                    target TEXT NOT NULL,
                    shortcut TEXT NOT NULL,
                    PRIMARY KEY (user_id, target)
                )",
            )
            .context("Ensure user_nav_shortcuts table failed")?;
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            tx.execute(
                "DELETE FROM user_nav_shortcuts WHERE user_id = ?1",
                rusqlite::params![user_id],
            )
            .context("Delete nav shortcuts failed")?;
            for (target, shortcut) in &entries {
                let sc = shortcut.as_deref().unwrap_or("");
                tx.execute(
                    "INSERT INTO user_nav_shortcuts (user_id, target, shortcut) VALUES (?1, ?2, ?3)",
                    rusqlite::params![user_id, target, sc],
                )
                .with_context(|| format!("Insert nav shortcut failed: user_id={user_id}, target={target}, shortcut={sc:?}"))?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }
}
