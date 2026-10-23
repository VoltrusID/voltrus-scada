use anyhow::{Context, Result};

use crate::asset::models::*;
use crate::asset::storage::core::{AssetStorage, now_iso};

impl AssetStorage {
    // ── Firmware Catalog ──

    pub async fn list_firmware_catalog(
        &self,
        vendor: Option<&str>,
    ) -> Result<Vec<FirmwareCatalogEntry>> {
        let conn = self.conn.clone();
        let vendor = vendor.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(ref v) = vendor {
                    (
                        "SELECT id, vendor, model, latest_version, release_date, notes, updated_at
                         FROM firmware_catalog WHERE vendor = ? ORDER BY vendor, model"
                            .to_string(),
                        vec![Box::new(v.clone())],
                    )
                } else {
                    (
                        "SELECT id, vendor, model, latest_version, release_date, notes, updated_at
                         FROM firmware_catalog ORDER BY vendor, model"
                            .to_string(),
                        vec![],
                    )
                };

            let mut stmt = conn
                .prepare(&sql)
                .context("Failed to prepare firmware catalog query")?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    Ok(FirmwareCatalogEntry {
                        id: row.get(0)?,
                        vendor: row.get(1)?,
                        model: row.get(2)?,
                        latest_version: row.get(3)?,
                        release_date: row.get(4)?,
                        notes: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                })
                .context("Failed to query firmware catalog")?;

            let mut entries = Vec::new();
            for row in rows {
                entries.push(row.context("Failed to map firmware catalog row")?);
            }
            Ok(entries)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_firmware_catalog_entry(
        &self,
        req: &CreateFirmwareCatalogRequest,
    ) -> Result<FirmwareCatalogEntry> {
        let conn = self.conn.clone();
        let vendor = req.vendor.clone();
        let model = req.model.clone();
        let latest_version = req.latest_version.clone();
        let release_date = req.release_date.clone();
        let notes = req.notes.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let id = super::core::generate_id();
            let now = now_iso();

            conn.execute(
                "INSERT INTO firmware_catalog (id, vendor, model, latest_version, release_date, notes, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, vendor, model, latest_version, release_date, notes, now],
            )
            .context("Failed to insert firmware catalog entry")?;

            Ok(FirmwareCatalogEntry {
                id,
                vendor,
                model,
                latest_version,
                release_date,
                notes,
                updated_at: now,
            })
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_firmware_catalog_entry(
        &self,
        id: &str,
        fields: &serde_json::Value,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let fields = fields.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let allowed = ["latest_version", "release_date", "notes"];

            let mut set_clauses: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(obj) = fields.as_object() {
                for &key in &allowed {
                    if let Some(value) = obj.get(key) {
                        set_clauses.push(format!("{key} = ?"));
                        match value {
                            serde_json::Value::Null => {
                                params.push(Box::new(Option::<String>::None))
                            }
                            serde_json::Value::String(s) => params.push(Box::new(s.clone())),
                            other => params.push(Box::new(other.to_string())),
                        }
                    }
                }
            }

            if set_clauses.is_empty() {
                return Ok(());
            }

            let now = now_iso();
            set_clauses.push("updated_at = ?".to_string());
            params.push(Box::new(now));
            params.push(Box::new(id));

            let sql = format!(
                "UPDATE firmware_catalog SET {} WHERE id = ?",
                set_clauses.join(", ")
            );

            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .context("Failed to update firmware catalog entry")?;

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_firmware_catalog_entry(&self, id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM firmware_catalog WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Failed to delete firmware catalog entry")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_latest_firmware(&self, vendor: &str, model: &str) -> Result<Option<String>> {
        let conn = self.conn.clone();
        let vendor = vendor.to_string();
        let model = model.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT latest_version FROM firmware_catalog WHERE vendor = ?1 AND model = ?2",
                rusqlite::params![vendor, model],
                |row| row.get::<_, String>(0),
            );

            match result {
                Ok(version) => Ok(Some(version)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Failed to get latest firmware"),
            }
        })
        .await
        .context("Task join failed")?
    }
}
