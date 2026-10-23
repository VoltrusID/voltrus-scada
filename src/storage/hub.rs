use super::Storage;
use crate::models::HubSite;
use anyhow::{Context, Result};

impl Storage {
    pub async fn create_hub_site(
        &self,
        name: &str,
        url: &str,
        api_key: &str,
        enabled: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        let api_key = api_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO hub_sites (name, url, api_key, enabled, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![name, url, api_key, enabled as i64, now],
            )
            .context("Insert hub_site failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_hub_sites(&self) -> Result<Vec<HubSite>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, api_key, enabled, last_seen, created_at FROM hub_sites ORDER BY created_at"
                )
                .context("Prepare list_hub_sites failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(HubSite {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    api_key: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    last_seen: row.get(5)?,
                    created_at: row.get(6)?,
                })
            }).context("Query list_hub_sites failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_hub_site(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        api_key: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());
        let api_key = api_key.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &name {
                conn.execute(
                    "UPDATE hub_sites SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update hub_site name failed")?;
            }
            if let Some(url) = &url {
                conn.execute(
                    "UPDATE hub_sites SET url = ?1 WHERE id = ?2",
                    rusqlite::params![url, id],
                )
                .context("Update hub_site url failed")?;
            }
            if let Some(api_key) = &api_key {
                conn.execute(
                    "UPDATE hub_sites SET api_key = ?1 WHERE id = ?2",
                    rusqlite::params![api_key, id],
                )
                .context("Update hub_site api_key failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE hub_sites SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update hub_site enabled failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_hub_site(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM hub_sites WHERE id = ?1", rusqlite::params![id])
                .context("Delete hub_site failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_hub_site_last_seen(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "UPDATE hub_sites SET last_seen = ?1 WHERE id = ?2",
                rusqlite::params![now, id],
            )
            .context("Update hub_site last_seen failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_enabled_hub_sites(&self) -> Result<Vec<HubSite>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, api_key, enabled, last_seen, created_at FROM hub_sites WHERE enabled = 1 ORDER BY created_at"
                )
                .context("Prepare get_enabled_hub_sites failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(HubSite {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    api_key: row.get(3)?,
                    enabled: row.get::<_, i64>(4)? != 0,
                    last_seen: row.get(5)?,
                    created_at: row.get(6)?,
                })
            }).context("Query get_enabled_hub_sites failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
