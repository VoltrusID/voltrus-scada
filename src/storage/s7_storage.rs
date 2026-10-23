use super::Storage;
use anyhow::{Context, Result};

use crate::models::{S7Device, S7TagRow};

impl Storage {
    pub async fn create_s7_device(
        &self,
        name: &str,
        url: &str,
        rack: u8,
        slot: u8,
        interval_ms: u64,
        enabled: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO s7_devices (name, url, rack, slot, interval_ms, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![name, url, rack, slot, interval_ms, enabled as i64, now],
            )
            .context("Insert s7 device failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_s7_devices(&self) -> Result<Vec<S7Device>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, rack, slot, interval_ms, enabled, created_at FROM s7_devices ORDER BY created_at"
                )
                .context("Prepare list_s7_devices failed")?;

            let rows = stmt.query_map([], |row| {
                Ok(S7Device {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    rack: row.get(3)?,
                    slot: row.get(4)?,
                    interval_ms: row.get(5)?,
                    enabled: row.get::<_, i64>(6)? != 0,
                    created_at: row.get(7)?,
                })
            })
            .context("Query list_s7_devices failed")?;

            let mut devices = Vec::new();
            for row in rows {
                devices.push(row.context("Row list_s7_devices failed")?);
            }
            Ok(devices)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_s7_device(&self, id: i64) -> Result<Option<S7Device>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, name, url, rack, slot, interval_ms, enabled, created_at FROM s7_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(S7Device {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        rack: row.get(3)?,
                        slot: row.get(4)?,
                        interval_ms: row.get(5)?,
                        enabled: row.get::<_, i64>(6)? != 0,
                        created_at: row.get(7)?,
                    })
                },
            );
            match result {
                Ok(device) => Ok(Some(device)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get s7 device failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_s7_device(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        rack: Option<u8>,
        slot: Option<u8>,
        interval_ms: Option<u64>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &name {
                conn.execute(
                    "UPDATE s7_devices SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update s7 device name failed")?;
            }
            if let Some(url) = &url {
                conn.execute(
                    "UPDATE s7_devices SET url = ?1 WHERE id = ?2",
                    rusqlite::params![url, id],
                )
                .context("Update s7 device url failed")?;
            }
            if let Some(rack) = rack {
                conn.execute(
                    "UPDATE s7_devices SET rack = ?1 WHERE id = ?2",
                    rusqlite::params![rack, id],
                )
                .context("Update s7 device rack failed")?;
            }
            if let Some(slot) = slot {
                conn.execute(
                    "UPDATE s7_devices SET slot = ?1 WHERE id = ?2",
                    rusqlite::params![slot, id],
                )
                .context("Update s7 device slot failed")?;
            }
            if let Some(interval_ms) = interval_ms {
                conn.execute(
                    "UPDATE s7_devices SET interval_ms = ?1 WHERE id = ?2",
                    rusqlite::params![interval_ms, id],
                )
                .context("Update s7 device interval_ms failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE s7_devices SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update s7 device enabled failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_s7_device(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let device_name: Option<String> = conn
                .query_row(
                    "SELECT name FROM s7_devices WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .ok();

            if let Some(device_name) = &device_name {
                let mut stmt = conn
                    .prepare("SELECT name FROM s7_tags WHERE device_id = ?1")
                    .context("Prepare select s7 tags failed")?;

                let tag_rows = stmt
                    .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
                    .context("Query s7 tags failed")?;

                for tag_name in tag_rows.flatten() {
                    let source_key = format!("s7_{}:{}", device_name, tag_name);
                    let _ = conn.execute(
                        "DELETE FROM data_sources WHERE key = ?1",
                        rusqlite::params![source_key],
                    );
                }
            }

            conn.execute(
                "DELETE FROM s7_devices WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete s7 device failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    pub async fn create_s7_tag(
        &self,
        device_id: i64,
        device_name: &str,
        tag_address: &str,
        name: &str,
        display_name: &str,
        data_type: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let tag_address = tag_address.to_string();
        let name = name.to_string();
        let display_name = display_name.to_string();
        let data_type = data_type.to_string();
        let device_name = device_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO s7_tags (device_id, tag_address, name, display_name, data_type, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![device_id, tag_address, name, display_name, data_type, now],
            )
            .context("Insert s7 tag failed")?;

            let source_key = format!("s7_{}:{}", device_name, name);
            let ds_name = if display_name.is_empty() {
                name.clone()
            } else {
                display_name.clone()
            };
            let _ = conn.execute(
                "INSERT OR IGNORE INTO data_sources (key, name, unit, color, min_val, max_val, warn_val, crit_val,
                 base_val, sort_order, decimals, source_type, source_config, device_id, chart_type)
                 VALUES (?1, ?2, '', '#3b82f6', 0, 0, 0, 0, 0, 0, 1, 's7', '{}', ?3, 'sparkline')",
                rusqlite::params![source_key, ds_name, device_name],
            );

            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_s7_tags(&self, device_id: i64) -> Result<Vec<S7TagRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, device_id, tag_address, name, display_name, data_type, created_at FROM s7_tags WHERE device_id = ?1 ORDER BY created_at"
                )
                .context("Prepare list_s7_tags failed")?;

            let rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(S7TagRow {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    tag_address: row.get(2)?,
                    name: row.get(3)?,
                    display_name: row.get(4)?,
                    data_type: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .context("Query list_s7_tags failed")?;

            let mut tags = Vec::new();
            for row in rows {
                tags.push(row.context("Row list_s7_tags failed")?);
            }
            Ok(tags)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_s7_tag(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            if let Ok((tag_name, device_id)) = conn.query_row(
                "SELECT name, device_id FROM s7_tags WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            ) && let Ok(device_name) = conn.query_row(
                "SELECT name FROM s7_devices WHERE id = ?1",
                rusqlite::params![device_id],
                |row| row.get::<_, String>(0),
            ) {
                let source_key = format!("s7_{}:{}", device_name, tag_name);
                let _ = conn.execute(
                    "DELETE FROM data_sources WHERE key = ?1",
                    rusqlite::params![source_key],
                );
            }

            conn.execute("DELETE FROM s7_tags WHERE id = ?1", rusqlite::params![id])
                .context("Delete s7 tag failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    pub async fn get_all_s7_device_configs(&self) -> Result<Vec<crate::s7::S7DeviceConfig>> {
        let devices = self.list_s7_devices().await?;
        let mut configs = Vec::new();
        for device in &devices {
            if !device.enabled {
                continue;
            }
            let tags = self.list_s7_tags(device.id).await?;
            let device_key = format!("s7_{}", device.id);
            let mut s7_tags = Vec::new();
            for tag_row in &tags {
                match crate::s7::S7Client::parse_tag_address(&tag_row.tag_address) {
                    Ok(s7_tag) => s7_tags.push(s7_tag),
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse S7 tag '{}' for device '{}': {}",
                            tag_row.tag_address,
                            device.name,
                            e
                        );
                    }
                }
            }
            configs.push(crate::s7::S7DeviceConfig {
                id: device_key,
                url: device.url.clone(),
                rack: device.rack,
                slot: device.slot,
                interval_ms: device.interval_ms,
                tags: s7_tags,
            });
        }
        Ok(configs)
    }
}
