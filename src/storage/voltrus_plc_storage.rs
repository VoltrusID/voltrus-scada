use super::Storage;
use anyhow::{Context, Result};

use crate::models::{VoltrusPlcDevice, VoltrusPlcTag};
use crate::voltrus_plc::VoltrusPlcDeviceConfig;

impl Storage {
    pub async fn create_voltrus_plc_device(
        &self,
        name: &str,
        url: &str,
        transport: &str,
        interval_ms: u64,
        enabled: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        let transport = transport.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO voltrus_plc_devices (name, url, transport, interval_ms, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![name, url, transport, interval_ms, enabled as i64, now],
            )
            .context("Insert voltrus_plc device failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_voltrus_plc_devices(&self) -> Result<Vec<VoltrusPlcDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, transport, interval_ms, enabled, created_at
                     FROM voltrus_plc_devices ORDER BY created_at",
                )
                .context("Prepare list_voltrus_plc_devices failed")?;

            let rows = stmt.query_map([], |row| {
                Ok(VoltrusPlcDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    transport: row.get(3)?,
                    interval_ms: row.get(4)?,
                    enabled: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;

            rows.collect::<Result<Vec<_>, _>>()
                .context("Failed to collect voltrus_plc devices")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_voltrus_plc_device(&self, id: i64) -> Result<Option<VoltrusPlcDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, transport, interval_ms, enabled, created_at
                     FROM voltrus_plc_devices WHERE id = ?1",
                )
                .context("Prepare get_voltrus_plc_device failed")?;

            let mut rows = stmt.query_map(rusqlite::params![id], |row| {
                Ok(VoltrusPlcDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    transport: row.get(3)?,
                    interval_ms: row.get(4)?,
                    enabled: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?;

            match rows.next() {
                Some(row) => Ok(Some(row?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_voltrus_plc_device(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        transport: Option<&str>,
        interval_ms: Option<u64>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());
        let transport = transport.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut updates = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(v) = name {
                updates.push("name = ?");
                params.push(Box::new(v));
            }
            if let Some(v) = url {
                updates.push("url = ?");
                params.push(Box::new(v));
            }
            if let Some(v) = transport {
                updates.push("transport = ?");
                params.push(Box::new(v));
            }
            if let Some(v) = interval_ms {
                updates.push("interval_ms = ?");
                params.push(Box::new(v));
            }
            if let Some(v) = enabled {
                updates.push("enabled = ?");
                params.push(Box::new(v as i64));
            }

            if updates.is_empty() {
                return Ok(());
            }

            params.push(Box::new(id));
            let sql = format!(
                "UPDATE voltrus_plc_devices SET {} WHERE id = ?",
                updates.join(", ")
            );

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())
                .context("Update voltrus_plc device failed")?;

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_voltrus_plc_device(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM voltrus_plc_tags WHERE device_id = ?1",
                rusqlite::params![id],
            )
            .context("Delete voltrus_plc tags failed")?;
            conn.execute(
                "DELETE FROM voltrus_plc_devices WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete voltrus_plc device failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_voltrus_plc_tag(
        &self,
        device_id: i64,
        device_name: &str,
        variable_name: &str,
        display_name: &str,
        data_type: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let variable_name = variable_name.to_string();
        let display_name = display_name.to_string();
        let data_type = data_type.to_string();
        let device_name = device_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO voltrus_plc_tags (device_id, variable_name, display_name, data_type, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![device_id, variable_name, display_name, data_type, now],
            )
            .context("Insert voltrus_plc tag failed")?;

            // Insert into data_sources so tag appears in SCADA dashboards/alarms
            let source_key = format!("voltrus_plc_{}:{}", device_name, variable_name);
            let ds_name = if display_name.is_empty() {
                variable_name.clone()
            } else {
                display_name.clone()
            };
            let _ = conn.execute(
                "INSERT OR IGNORE INTO data_sources (key, name, unit, color, min_val, max_val, warn_val, crit_val,
                 base_val, sort_order, decimals, source_type, source_config, device_id, chart_type)
                 VALUES (?1, ?2, '', '#3b82f6', 0, 0, 0, 0, 0, 0, 1, 'voltrus-plc', '{}', ?3, 'sparkline')",
                rusqlite::params![source_key, ds_name, device_name],
            );

            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_voltrus_plc_tags(&self, device_id: i64) -> Result<Vec<VoltrusPlcTag>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, device_id, variable_name, display_name, data_type, created_at
                     FROM voltrus_plc_tags WHERE device_id = ?1 ORDER BY created_at",
                )
                .context("Prepare list_voltrus_plc_tags failed")?;

            let rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(VoltrusPlcTag {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    variable_name: row.get(2)?,
                    display_name: row.get(3)?,
                    data_type: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?;

            rows.collect::<Result<Vec<_>, _>>()
                .context("Failed to collect voltrus_plc tags")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_voltrus_plc_tag(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM voltrus_plc_tags WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete voltrus_plc tag failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Load all device configs with their tags for poller startup.
    pub async fn get_all_voltrus_plc_device_configs(&self) -> Result<Vec<VoltrusPlcDeviceConfig>> {
        let devices = self.list_voltrus_plc_devices().await?;
        let mut configs = Vec::new();

        for device in devices {
            if !device.enabled {
                continue;
            }
            let tags = self.list_voltrus_plc_tags(device.id).await?;
            configs.push(VoltrusPlcDeviceConfig {
                id: device.id.to_string(),
                url: device.url,
                transport: device.transport,
                interval_ms: device.interval_ms,
                tags,
            });
        }

        Ok(configs)
    }
}
