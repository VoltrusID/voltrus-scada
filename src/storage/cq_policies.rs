use super::Storage;
use anyhow::{Context, Result};

use crate::models::{DataSource, WritePolicy, WritePolicyUpdate};

impl Storage {
    pub async fn get_write_policy(&self, data_source_id: i64) -> Result<Option<WritePolicy>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT writable, write_min, write_max, write_max_rate, write_requires_reason, write_deadband
                 FROM data_sources WHERE id = ?1",
                rusqlite::params![data_source_id],
                |row| {
                    let writable: i64 = row.get(0)?;
                    let write_requires_reason: i64 = row.get(4)?;
                    Ok(WritePolicy {
                        writable: writable != 0,
                        write_min: row.get(1)?,
                        write_max: row.get(2)?,
                        write_max_rate: row.get(3)?,
                        write_requires_reason: write_requires_reason != 0,
                        write_deadband: row.get(5)?,
                    })
                },
            );
            match result {
                Ok(policy) => Ok(Some(policy)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get write policy failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_write_policy(
        &self,
        data_source_id: i64,
        update: &WritePolicyUpdate,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let writable = update.writable;
        let write_min = update.write_min;
        let write_max = update.write_max;
        let write_max_rate = update.write_max_rate;
        let write_requires_reason = update.write_requires_reason;
        let write_deadband = update.write_deadband;
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut sets: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(v) = writable {
                sets.push(format!("writable = ?{}", sets.len() + 1));
                params.push(Box::new(v as i64));
            }
            if let Some(ref v) = write_min {
                sets.push(format!("write_min = ?{}", sets.len() + 1));
                params.push(Box::new(*v));
            }
            if let Some(ref v) = write_max {
                sets.push(format!("write_max = ?{}", sets.len() + 1));
                params.push(Box::new(*v));
            }
            if let Some(ref v) = write_max_rate {
                sets.push(format!("write_max_rate = ?{}", sets.len() + 1));
                params.push(Box::new(*v));
            }
            if let Some(v) = write_requires_reason {
                sets.push(format!("write_requires_reason = ?{}", sets.len() + 1));
                params.push(Box::new(v as i64));
            }
            if let Some(ref v) = write_deadband {
                sets.push(format!("write_deadband = ?{}", sets.len() + 1));
                params.push(Box::new(*v));
            }

            if sets.is_empty() {
                return Ok(());
            }

            let sql = format!(
                "UPDATE data_sources SET {} WHERE id = ?{}",
                sets.join(", "),
                sets.len() + 1,
            );
            params.push(Box::new(data_source_id));

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            conn.execute(&sql, param_refs.as_slice())
                .context("Update write policy failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn count_recent_writes(&self, data_source_id: i64, window_secs: i64) -> Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let cutoff = now - window_secs;
            // NOTE: We count by created_at (when the command was queued) rather than
            // sent_at (when it was actually sent). This means commands still sitting in
            // "pending"/"queued" also count against the rate limit. The trade-off is that
            // a burst of queued commands won't slip past the limit — the user gets rate-
            // limited at submission time. If we used sent_at only, a flood of pending
            // commands could all be dispatched once the processor catches up.
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM commands WHERE data_source_id = ? AND created_at >= ?",
                    rusqlite::params![data_source_id, cutoff],
                    |row| row.get(0),
                )
                .context("Count recent writes failed")?;
            Ok(count)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_data_source_device_id(
        &self,
        data_source_id: i64,
    ) -> Result<Option<(i64, String)>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // data_sources has device_id TEXT and source_type TEXT.
            // For modbus, device_id is a string id. For opcua/s7/enip, it's a stringified integer.
            // We parse device_id as i64 and return source_type as protocol.
            let result = conn.query_row(
                "SELECT device_id, source_type FROM data_sources WHERE id = ?1",
                rusqlite::params![data_source_id],
                |row| {
                    let device_id_str: String = row.get(0)?;
                    let protocol: String = row.get(1)?;
                    Ok((device_id_str, protocol))
                },
            );
            match result {
                Ok((device_id_str, protocol)) => {
                    match device_id_str.parse::<i64>() {
                        Ok(id) => Ok(Some((id, protocol))),
                        Err(_) => {
                            // device_id is non-numeric (e.g., modbus text id).
                            // Try to look up integer id from modbus_devices table.
                            let modbus_id: i64 = conn
                                .query_row(
                                    "SELECT COUNT(*) FROM modbus_devices WHERE id = ?1",
                                    rusqlite::params![device_id_str],
                                    |row| row.get::<_, i64>(0),
                                )
                                .unwrap_or(0);
                            if modbus_id > 0 {
                                // Modbus device exists but has text id; return 0 as device_id marker
                                // The caller will need to handle modbus text IDs separately
                                Ok(Some((0i64, protocol)))
                            } else {
                                Ok(None)
                            }
                        }
                    }
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get data source device id failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    /// Find a data source by device_id and key. Returns the data source if found.
    pub async fn find_data_source_by_device_and_key(
        &self,
        device_id: &str,
        key: &str,
    ) -> Result<Option<DataSource>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, key, name, unit, color, min_val, max_val, warn_val, crit_val, base_val, sort_order, decimals, source_type, source_config, device_id, equipment_id, chart_type, path, on_delay_secs, off_delay_secs FROM data_sources WHERE device_id = ?1 AND key = ?2",
                rusqlite::params![device_id, key],
                |row| {
                    Ok(DataSource {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        name: row.get(2)?,
                        unit: row.get(3)?,
                        color: row.get(4)?,
                        min_val: row.get(5)?,
                        max_val: row.get(6)?,
                        warn_val: row.get(7)?,
                        crit_val: row.get(8)?,
                        base_val: row.get(9)?,
                        sort_order: row.get(10)?,
                        decimals: row.get(11)?,
                        source_type: row.get(12)?,
                        source_config: row.get(13)?,
                        device_id: row.get(14)?,
                        equipment_id: row.get(15)?,
                        chart_type: row.get(16)?,
                        path: row.get(17)?,
                        on_delay_secs: row.get(18)?,
                        off_delay_secs: row.get(19)?,
                    })
                },
            );
            match result {
                Ok(ds) => Ok(Some(ds)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Find data source by device and key failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    /// Get the key of a data source by its ID. Returns None if not found.
    pub async fn get_data_source_key(&self, data_source_id: i64) -> Result<Option<String>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT key FROM data_sources WHERE id = ?1",
                rusqlite::params![data_source_id],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(key) => Ok(Some(key)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get data source key failed"),
            }
        })
        .await
        .context("Task join failed")?
    }
}
