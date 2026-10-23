use super::Storage;
use anyhow::{Context, Result};

use crate::models::{EnipDevice, EnipDeviceConfig, EnipTag};

impl Storage {
    pub async fn create_enip_device(
        &self,
        name: &str,
        url: &str,
        slot: u8,
        interval_ms: u64,
        enabled: bool,
        assembly_instance: u32,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO enip_devices (name, url, slot, interval_ms, enabled, assembly_instance, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![name, url, slot, interval_ms, enabled as i64, assembly_instance, now],
            )
            .context("Insert enip device failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_enip_device(&self, id: i64) -> Result<Option<EnipDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, name, url, slot, interval_ms, enabled, assembly_instance, created_at FROM enip_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(EnipDevice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        slot: row.get(3)?,
                        interval_ms: row.get(4)?,
                        enabled: row.get::<_, i64>(5)? != 0,
                        assembly_instance: row.get(6)?,
                        created_at: row.get(7)?,
                    })
                },
            );
            match result {
                Ok(device) => Ok(Some(device)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get enip device failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_enip_devices(&self) -> Result<Vec<EnipDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, slot, interval_ms, enabled, assembly_instance, created_at FROM enip_devices ORDER BY created_at"
                )
                .context("Prepare list_enip_devices failed")?;

            let rows = stmt.query_map([], |row| {
                Ok(EnipDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    slot: row.get(3)?,
                    interval_ms: row.get(4)?,
                    enabled: row.get::<_, i64>(5)? != 0,
                    assembly_instance: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })
            .context("Query list_enip_devices failed")?;

            let mut devices = Vec::new();
            for row in rows {
                devices.push(row.context("Row list_enip_devices failed")?);
            }
            Ok(devices)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_enip_device(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        slot: Option<u8>,
        interval_ms: Option<u64>,
        enabled: Option<bool>,
        assembly_instance: Option<u32>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &name {
                conn.execute(
                    "UPDATE enip_devices SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update enip device name failed")?;
            }
            if let Some(url) = &url {
                conn.execute(
                    "UPDATE enip_devices SET url = ?1 WHERE id = ?2",
                    rusqlite::params![url, id],
                )
                .context("Update enip device url failed")?;
            }
            if let Some(slot) = slot {
                conn.execute(
                    "UPDATE enip_devices SET slot = ?1 WHERE id = ?2",
                    rusqlite::params![slot, id],
                )
                .context("Update enip device slot failed")?;
            }
            if let Some(interval_ms) = interval_ms {
                conn.execute(
                    "UPDATE enip_devices SET interval_ms = ?1 WHERE id = ?2",
                    rusqlite::params![interval_ms, id],
                )
                .context("Update enip device interval_ms failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE enip_devices SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update enip device enabled failed")?;
            }
            if let Some(assembly_instance) = assembly_instance {
                conn.execute(
                    "UPDATE enip_devices SET assembly_instance = ?1 WHERE id = ?2",
                    rusqlite::params![assembly_instance, id],
                )
                .context("Update enip device assembly_instance failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_enip_device(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Look up device name and load all tags to clean up data sources
            let device_name: Option<String> = conn
                .query_row(
                    "SELECT name FROM enip_devices WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .ok();

            if let Some(device_name) = &device_name {
                let mut stmt = conn
                    .prepare("SELECT name FROM enip_tags WHERE device_id = ?1")
                    .context("Prepare select enip tags failed")?;

                let tag_rows = stmt
                    .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
                    .context("Query enip tags failed")?;

                for tag_name in tag_rows.flatten() {
                    let source_key = format!("{}:{}", device_name, tag_name);
                    let _ = conn.execute(
                        "DELETE FROM data_sources WHERE key = ?1",
                        rusqlite::params![source_key],
                    );
                }
            }

            conn.execute(
                "DELETE FROM enip_devices WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete enip device failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    pub async fn create_enip_tag(
        &self,
        device_id: i64,
        device_name: &str,
        tag_name: &str,
        name: &str,
        display_name: &str,
        data_type: &str,
        assembly_offset: u32,
        assembly_length: u32,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let tag_name = tag_name.to_string();
        let name = name.to_string();
        let display_name = display_name.to_string();
        let data_type = data_type.to_string();
        let device_name = device_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO enip_tags (device_id, tag_name, name, display_name, data_type, assembly_offset, assembly_length, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![device_id, tag_name, name, display_name, data_type, assembly_offset, assembly_length, now],
            )
            .context("Insert enip tag failed")?;

            // Auto-create data source for this tag
            let source_key = format!("{}:{}", device_name, name);
            let ds_name = if display_name.is_empty() {
                name.clone()
            } else {
                display_name.clone()
            };
            let _ = conn.execute(
                "INSERT OR IGNORE INTO data_sources (key, name, unit, color, min_val, max_val, warn_val, crit_val,
                 base_val, sort_order, decimals, source_type, source_config, device_id, chart_type)
                 VALUES (?1, ?2, '', '#f97316', 0, 0, 0, 0, 0, 0, 1, 'enip', '{}', ?3, 'sparkline')",
                rusqlite::params![source_key, ds_name, device_name],
            );

            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_enip_tags(&self, device_id: i64) -> Result<Vec<EnipTag>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, device_id, tag_name, name, display_name, data_type, assembly_offset, assembly_length, created_at FROM enip_tags WHERE device_id = ?1 ORDER BY created_at"
                )
                .context("Prepare list_enip_tags failed")?;

            let rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(EnipTag {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    tag_name: row.get(2)?,
                    name: row.get(3)?,
                    display_name: row.get(4)?,
                    data_type: row.get(5)?,
                    assembly_offset: row.get(6)?,
                    assembly_length: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .context("Query list_enip_tags failed")?;

            let mut tags = Vec::new();
            for row in rows {
                tags.push(row.context("Row list_enip_tags failed")?);
            }
            Ok(tags)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_enip_tag(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Look up tag name and device name to delete corresponding data source
            if let Ok((tag_name, device_id)) = conn.query_row(
                "SELECT name, device_id FROM enip_tags WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            ) && let Ok(device_name) = conn.query_row(
                "SELECT name FROM enip_devices WHERE id = ?1",
                rusqlite::params![device_id],
                |row| row.get::<_, String>(0),
            ) {
                let source_key = format!("{}:{}", device_name, tag_name);
                let _ = conn.execute(
                    "DELETE FROM data_sources WHERE key = ?1",
                    rusqlite::params![source_key],
                );
            }

            conn.execute("DELETE FROM enip_tags WHERE id = ?1", rusqlite::params![id])
                .context("Delete enip tag failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    /// Load all enabled EtherNet/IP devices with their tags and convert to runtime configs.
    pub async fn get_all_enip_device_configs(&self) -> Result<Vec<EnipDeviceConfig>> {
        let devices = self.list_enip_devices().await?;
        let mut configs = Vec::new();
        for device in &devices {
            if !device.enabled {
                continue;
            }
            let tags = self.list_enip_tags(device.id).await?;
            let device_key = format!("enip_{}", device.id);
            configs.push(EnipDeviceConfig {
                id: device_key,
                url: device.url.clone(),
                slot: device.slot,
                interval_ms: device.interval_ms,
                assembly_instance: device.assembly_instance,
                tags: tags
                    .iter()
                    .map(|t| crate::ethernet_ip::EnipTagConfig {
                        tag_name: t.tag_name.clone(),
                        name: t.name.clone(),
                        data_type: t.data_type.clone(),
                        assembly_offset: t.assembly_offset,
                        assembly_length: t.assembly_length,
                    })
                    .collect(),
            });
        }
        Ok(configs)
    }
}
