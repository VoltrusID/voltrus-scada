#![allow(clippy::needless_range_loop)]
use super::Storage;
use anyhow::{Context, Result};

use crate::models::ConfigSnapshot;

/// Configuration tables a snapshot captures and a rollback rewrites.
/// `alarms` is runtime history (with audit/journal rows cascading from it),
/// not configuration, so it is never captured or rewritten; older snapshots
/// that still carry an `alarms` key have it ignored on rollback.
const VERSIONED_TABLES: [&str; 5] = ["devices", "data_sources", "flows", "screens", "recipes"];

impl Storage {
    // ── Config Snapshot CRUD ──

    pub async fn list_config_snapshots(&self, limit: i64) -> Result<Vec<ConfigSnapshot>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, label, description, snapshot_data, trigger, created_by, created_at
                     FROM config_snapshots ORDER BY id DESC LIMIT ?1",
                )
                .context("Prepare list_config_snapshots failed")?;
            let rows = stmt
                .query_map(rusqlite::params![limit], |row| {
                    Ok(ConfigSnapshot {
                        id: row.get(0)?,
                        label: row.get(1)?,
                        description: row.get(2)?,
                        snapshot_data: row.get(3)?,
                        trigger: row.get(4)?,
                        created_by: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .context("Query list_config_snapshots failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row parse list_config_snapshots failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_config_snapshot(&self, id: i64) -> Result<Option<ConfigSnapshot>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, label, description, snapshot_data, trigger, created_by, created_at
                     FROM config_snapshots WHERE id = ?1",
                )
                .context("Prepare get_config_snapshot failed")?;
            let mut rows = stmt
                .query_map(rusqlite::params![id], |row| {
                    Ok(ConfigSnapshot {
                        id: row.get(0)?,
                        label: row.get(1)?,
                        description: row.get(2)?,
                        snapshot_data: row.get(3)?,
                        trigger: row.get(4)?,
                        created_by: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .context("Query get_config_snapshot failed")?;
            match rows.next() {
                Some(r) => Ok(Some(r.context("Row parse get_config_snapshot failed")?)),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_config_snapshot(
        &self,
        label: Option<&str>,
        description: Option<&str>,
        trigger: &str,
        created_by: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let label_owned = label.map(|s| s.to_string());
        let description_owned = description.map(|s| s.to_string());
        let trigger_owned = trigger.to_string();
        let created_by_owned = created_by.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut snapshot_map = serde_json::Map::new();

            for table in &VERSIONED_TABLES {
                let mut stmt = match conn.prepare(&format!("SELECT * FROM {}", table)) {
                    Ok(s) => s,
                    Err(_) => continue, // table may not exist in older DBs
                };
                let col_count = stmt.column_count();
                let col_names: Vec<String> = (0..col_count)
                    .filter_map(|i| stmt.column_name(i).ok().map(|n| n.to_string()))
                    .collect();
                let rows: Vec<serde_json::Value> = stmt
                    .query_map([], |row| {
                        let mut map = serde_json::Map::new();
                        for i in 0..col_count {
                            let val: rusqlite::types::Value = row.get(i)?;
                            map.insert(
                                col_names[i].clone(),
                                rusqlite_value_to_json(val),
                            );
                        }
                        Ok(serde_json::Value::Object(map))
                    })
                    .ok()
                    .map(|iter| iter.filter_map(|r| r.ok()).collect())
                    .unwrap_or_default();

                snapshot_map.insert(table.to_string(), serde_json::json!(rows));
            }

            let snapshot_data = serde_json::Value::Object(snapshot_map).to_string();
            let now = crate::timeutil::now_secs();

            conn.execute(
                "INSERT INTO config_snapshots (label, description, snapshot_data, trigger, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![label_owned, description_owned, snapshot_data, trigger_owned, created_by_owned, now],
            )
            .context("Insert config_snapshot failed")?;

            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn rollback_to_snapshot(&self, snapshot_id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Load snapshot data
            let snapshot_data: String = match conn.query_row(
                "SELECT snapshot_data FROM config_snapshots WHERE id = ?1",
                rusqlite::params![snapshot_id],
                |row| row.get(0),
            ) {
                Ok(d) => d,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(e) => return Err(e).context("Query snapshot_data failed"),
            };

            let tx = conn
                .unchecked_transaction()
                .context("Begin rollback tx failed")?;
            apply_snapshot(&tx, &snapshot_data)?;
            tx.commit().context("Commit rollback tx failed")?;
            Ok(true)
        })
        .await
        .context("Task join failed")?
    }

    /// Undo a go-live in one transaction: restore the pre-go-live snapshot, put
    /// `device_id` back into simulation mode and drop its go-live record
    /// (`go_live:<device_id>` setting). Either everything happens or nothing does.
    /// Returns false when the snapshot no longer exists.
    pub async fn rollback_go_live(&self, device_id: &str, snapshot_id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let snapshot_data: String = match conn.query_row(
                "SELECT snapshot_data FROM config_snapshots WHERE id = ?1",
                rusqlite::params![snapshot_id],
                |row| row.get(0),
            ) {
                Ok(d) => d,
                Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(false),
                Err(e) => return Err(e).context("Query snapshot_data failed"),
            };

            let tx = conn
                .unchecked_transaction()
                .context("Begin go-live rollback tx failed")?;
            apply_snapshot(&tx, &snapshot_data)?;
            let affected = tx
                .execute(
                    "UPDATE devices SET simulation_mode = 1 WHERE id = ?1",
                    rusqlite::params![device_id],
                )
                .context("Re-enable simulation failed")?;
            if affected == 0 {
                anyhow::bail!("Device {device_id} is not in the pre-go-live snapshot");
            }
            tx.execute(
                "DELETE FROM settings WHERE key = ?1",
                rusqlite::params![format!("go_live:{device_id}")],
            )
            .context("Clear go-live record failed")?;
            tx.commit().context("Commit go-live rollback tx failed")?;
            Ok(true)
        })
        .await
        .context("Task join failed")?
    }

    /// Delete one snapshot. Returns false when no snapshot has that id.
    pub async fn delete_config_snapshot(&self, id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let n = conn
                .execute(
                    "DELETE FROM config_snapshots WHERE id = ?1",
                    rusqlite::params![id],
                )
                .context("Delete config_snapshot failed")?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn cleanup_old_snapshots(&self, keep_count: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM config_snapshots
                 WHERE trigger = 'auto'
                   AND id NOT IN (
                       SELECT id FROM config_snapshots
                       WHERE trigger = 'auto'
                       ORDER BY id DESC LIMIT ?1
                   )",
                rusqlite::params![keep_count],
            )
            .context("Delete old auto snapshots failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}

// ── Config Snapshot helper functions ──

/// Replace the contents of every versioned table inside `tx` with the rows
/// stored in `snapshot_data`. The caller owns the transaction and decides when
/// to commit, so a rollback can be combined with other writes atomically.
fn apply_snapshot(tx: &rusqlite::Transaction<'_>, snapshot_data: &str) -> Result<()> {
    let snapshot: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(snapshot_data).context("Parse snapshot JSON failed")?;

    for table in &VERSIONED_TABLES {
        if let Some(table_data) = snapshot.get(*table) {
            let arr = match table_data.as_array() {
                Some(a) => a,
                None => continue,
            };

            // Delete all existing rows
            let _ = tx.execute(&format!("DELETE FROM {}", table), []).ok(); // table may not exist

            if arr.is_empty() {
                continue;
            }

            // Determine column names from the first row object
            if let Some(serde_json::Value::Object(first_row)) = arr.first() {
                let col_names: Vec<&str> = first_row.keys().map(|s| s.as_str()).collect();
                let placeholders: Vec<String> =
                    (1..=col_names.len()).map(|i| format!("?{}", i)).collect();
                let insert_sql = format!(
                    "INSERT INTO {} ({}) VALUES ({})",
                    table,
                    col_names.join(", "),
                    placeholders.join(", "),
                );

                for row_obj in arr {
                    if let serde_json::Value::Object(obj) = row_obj {
                        let params: Vec<Box<dyn rusqlite::types::ToSql>> = col_names
                            .iter()
                            .map(|col| {
                                let val = obj.get(*col).unwrap_or(&serde_json::Value::Null);
                                json_to_rusqlite_value(val)
                            })
                            .collect();

                        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                            params.iter().map(|p| p.as_ref()).collect();

                        tx.execute(&insert_sql, param_refs.as_slice())
                            .context(format!("Insert rollback row into {} failed", table))?;
                    }
                }
            }
        }
    }

    Ok(())
}

fn rusqlite_value_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val {
        rusqlite::types::Value::Null => serde_json::Value::Null,
        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
        rusqlite::types::Value::Real(f) => serde_json::json!(f),
        rusqlite::types::Value::Text(s) => serde_json::json!(s),
        rusqlite::types::Value::Blob(b) => serde_json::json!(b),
    }
}

fn json_to_rusqlite_value(val: &serde_json::Value) -> Box<dyn rusqlite::types::ToSql> {
    match val {
        serde_json::Value::Null => Box::new(Option::<String>::None),
        serde_json::Value::Bool(b) => Box::new(*b as i32),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Box::new(i)
            } else if let Some(f) = n.as_f64() {
                Box::new(f)
            } else {
                Box::new(n.to_string())
            }
        }
        serde_json::Value::String(s) => Box::new(s.clone()),
        _ => Box::new(val.to_string()),
    }
}
