use super::Storage;
use anyhow::{Context, Result};

use crate::models::{
    OpcUaDevice, OpcUaDeviceConfig, OpcUaNode, OpcUaNodeConfig, OpcUaSecurityMode,
    OpcUaSecurityPolicy,
};

impl Storage {
    pub async fn create_opcua_device(
        &self,
        name: &str,
        url: &str,
        security_policy: &OpcUaSecurityPolicy,
        security_mode: &OpcUaSecurityMode,
        username: Option<&str>,
        password: Option<&str>,
        interval_ms: u64,
        enabled: bool,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let name = name.to_string();
        let url = url.to_string();
        let security_policy =
            serde_json::to_string(security_policy).unwrap_or_else(|_| "none".to_string());
        let security_mode =
            serde_json::to_string(security_mode).unwrap_or_else(|_| "none".to_string());
        let username = username.map(|s| s.to_string());
        let password = password.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO opcua_devices (name, url, security_policy, security_mode, username, password, interval_ms, enabled, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![name, url, security_policy, security_mode, username, password, interval_ms, enabled as i64, now],
            )
            .context("Insert opcua device failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_opcua_device(&self, id: i64) -> Result<Option<OpcUaDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, name, url, security_policy, security_mode, username, password, interval_ms, enabled, created_at FROM opcua_devices WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    let policy_str: String = row.get(3)?;
                    let mode_str: String = row.get(4)?;
                    Ok(OpcUaDevice {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        security_policy: serde_json::from_str(&policy_str).unwrap_or(OpcUaSecurityPolicy::None),
                        security_mode: serde_json::from_str(&mode_str).unwrap_or(OpcUaSecurityMode::None),
                        username: row.get(5)?,
                        password: row.get(6)?,
                        interval_ms: row.get(7)?,
                        enabled: row.get::<_, i64>(8)? != 0,
                        created_at: row.get(9)?,
                    })
                },
            );
            match result {
                Ok(device) => Ok(Some(device)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get opcua device failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_opcua_devices(&self) -> Result<Vec<OpcUaDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, security_policy, security_mode, username, password, interval_ms, enabled, created_at FROM opcua_devices ORDER BY created_at"
                )
                .context("Prepare list_opcua_devices failed")?;

            let rows = stmt.query_map([], |row| {
                let policy_str: String = row.get(3)?;
                let mode_str: String = row.get(4)?;
                Ok(OpcUaDevice {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    url: row.get(2)?,
                    security_policy: serde_json::from_str(&policy_str).unwrap_or(OpcUaSecurityPolicy::None),
                    security_mode: serde_json::from_str(&mode_str).unwrap_or(OpcUaSecurityMode::None),
                    username: row.get(5)?,
                    password: row.get(6)?,
                    interval_ms: row.get(7)?,
                    enabled: row.get::<_, i64>(8)? != 0,
                    created_at: row.get(9)?,
                })
            })
            .context("Query list_opcua_devices failed")?;

            let mut devices = Vec::new();
            for row in rows {
                devices.push(row.context("Row list_opcua_devices failed")?);
            }
            Ok(devices)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_opcua_device(
        &self,
        id: i64,
        name: Option<&str>,
        url: Option<&str>,
        security_policy: Option<&OpcUaSecurityPolicy>,
        security_mode: Option<&OpcUaSecurityMode>,
        username: Option<Option<&str>>,
        password: Option<Option<&str>>,
        interval_ms: Option<u64>,
        enabled: Option<bool>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let name = name.map(|s| s.to_string());
        let url = url.map(|s| s.to_string());
        let security_policy = security_policy
            .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "none".to_string()));
        let security_mode =
            security_mode.map(|m| serde_json::to_string(m).unwrap_or_else(|_| "none".to_string()));
        let username = username.map(|o| o.map(|s| s.to_string()));
        let password = password.map(|o| o.map(|s| s.to_string()));

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &name {
                conn.execute(
                    "UPDATE opcua_devices SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update opcua device name failed")?;
            }
            if let Some(url) = &url {
                conn.execute(
                    "UPDATE opcua_devices SET url = ?1 WHERE id = ?2",
                    rusqlite::params![url, id],
                )
                .context("Update opcua device url failed")?;
            }
            if let Some(policy) = &security_policy {
                conn.execute(
                    "UPDATE opcua_devices SET security_policy = ?1 WHERE id = ?2",
                    rusqlite::params![policy, id],
                )
                .context("Update opcua device security_policy failed")?;
            }
            if let Some(mode) = &security_mode {
                conn.execute(
                    "UPDATE opcua_devices SET security_mode = ?1 WHERE id = ?2",
                    rusqlite::params![mode, id],
                )
                .context("Update opcua device security_mode failed")?;
            }
            if let Some(ref username) = username {
                conn.execute(
                    "UPDATE opcua_devices SET username = ?1 WHERE id = ?2",
                    rusqlite::params![username, id],
                )
                .context("Update opcua device username failed")?;
            }
            if let Some(ref password) = password {
                conn.execute(
                    "UPDATE opcua_devices SET password = ?1 WHERE id = ?2",
                    rusqlite::params![password, id],
                )
                .context("Update opcua device password failed")?;
            }
            if let Some(interval_ms) = interval_ms {
                conn.execute(
                    "UPDATE opcua_devices SET interval_ms = ?1 WHERE id = ?2",
                    rusqlite::params![interval_ms, id],
                )
                .context("Update opcua device interval_ms failed")?;
            }
            if let Some(enabled) = enabled {
                conn.execute(
                    "UPDATE opcua_devices SET enabled = ?1 WHERE id = ?2",
                    rusqlite::params![enabled as i64, id],
                )
                .context("Update opcua device enabled failed")?;
            }
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_opcua_device(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Look up device name and load all nodes to clean up data sources
            let device_name: Option<String> = conn
                .query_row(
                    "SELECT name FROM opcua_devices WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .ok();

            if let Some(device_name) = &device_name {
                let mut stmt = conn
                    .prepare("SELECT name FROM opcua_nodes WHERE device_id = ?1")
                    .context("Prepare select opcua nodes failed")?;

                let node_rows = stmt
                    .query_map(rusqlite::params![id], |row| row.get::<_, String>(0))
                    .context("Query opcua nodes failed")?;

                for node_name in node_rows.flatten() {
                    let source_key = format!("{}:{}", device_name, node_name);
                    let _ = conn.execute(
                        "DELETE FROM data_sources WHERE key = ?1",
                        rusqlite::params![source_key],
                    );
                }
            }

            conn.execute(
                "DELETE FROM opcua_devices WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete opcua device failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    pub async fn create_opcua_node(
        &self,
        device_id: i64,
        device_name: &str,
        node_id: &str,
        name: &str,
        display_name: &str,
        data_type: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let node_id = node_id.to_string();
        let name = name.to_string();
        let display_name = display_name.to_string();
        let data_type = data_type.to_string();
        let device_name = device_name.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO opcua_nodes (device_id, node_id, name, display_name, data_type, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![device_id, node_id, name, display_name, data_type, now],
            )
            .context("Insert opcua node failed")?;

            // Auto-create data source for this node
            let source_key = format!("{}:{}", device_name, name);
            let ds_name = if display_name.is_empty() {
                name.clone()
            } else {
                display_name.clone()
            };
            let _ = conn.execute(
                "INSERT OR IGNORE INTO data_sources (key, name, unit, color, min_val, max_val, warn_val, crit_val,
                 base_val, sort_order, decimals, source_type, source_config, device_id, chart_type)
                 VALUES (?1, ?2, '', '#f97316', 0, 0, 0, 0, 0, 0, 1, 'opcua', '{}', ?3, 'sparkline')",
                rusqlite::params![source_key, ds_name, device_name],
            );

            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_opcua_nodes(&self, device_id: i64) -> Result<Vec<OpcUaNode>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, device_id, node_id, name, display_name, data_type, created_at FROM opcua_nodes WHERE device_id = ?1 ORDER BY created_at"
                )
                .context("Prepare list_opcua_nodes failed")?;

            let rows = stmt.query_map(rusqlite::params![device_id], |row| {
                Ok(OpcUaNode {
                    id: row.get(0)?,
                    device_id: row.get(1)?,
                    node_id: row.get(2)?,
                    name: row.get(3)?,
                    display_name: row.get(4)?,
                    data_type: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .context("Query list_opcua_nodes failed")?;

            let mut nodes = Vec::new();
            for row in rows {
                nodes.push(row.context("Row list_opcua_nodes failed")?);
            }
            Ok(nodes)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_opcua_node(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Look up node name and device name to delete corresponding data source
            if let Ok((node_name, device_id)) = conn.query_row(
                "SELECT name, device_id FROM opcua_nodes WHERE id = ?1",
                rusqlite::params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            ) && let Ok(device_name) = conn.query_row(
                "SELECT name FROM opcua_devices WHERE id = ?1",
                rusqlite::params![device_id],
                |row| row.get::<_, String>(0),
            ) {
                let source_key = format!("{}:{}", device_name, node_name);
                let _ = conn.execute(
                    "DELETE FROM data_sources WHERE key = ?1",
                    rusqlite::params![source_key],
                );
            }

            conn.execute(
                "DELETE FROM opcua_nodes WHERE id = ?1",
                rusqlite::params![id],
            )
            .context("Delete opcua node failed")
        })
        .await
        .context("Task join failed")?
        .map(|_| ())
    }

    /// Load all enabled OPC-UA devices with their nodes and convert to runtime configs.
    pub async fn get_all_opcua_device_configs(&self) -> Result<Vec<OpcUaDeviceConfig>> {
        let devices = self.list_opcua_devices().await?;
        let mut configs = Vec::new();
        for device in &devices {
            if !device.enabled {
                continue;
            }
            let nodes = self.list_opcua_nodes(device.id).await?;
            let device_key = format!("opcua_{}", device.id);
            configs.push(OpcUaDeviceConfig {
                id: device_key,
                url: device.url.clone(),
                security_policy: device.security_policy.clone(),
                security_mode: device.security_mode.clone(),
                username: device.username.clone(),
                password: device.password.clone(),
                interval_ms: device.interval_ms,
                nodes: nodes
                    .iter()
                    .map(|n| OpcUaNodeConfig {
                        node_id: n.node_id.clone(),
                        name: n.name.clone(),
                        display_name: n.display_name.clone(),
                        data_type: n.data_type.clone(),
                    })
                    .collect(),
            });
        }
        Ok(configs)
    }
}
