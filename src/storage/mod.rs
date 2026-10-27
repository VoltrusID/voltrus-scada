mod ai_copilot;
mod alarm_crud;
mod alarm_management;
mod api_keys;
#[cfg(feature = "bacnet")]
mod bacnet;
mod camera;
mod config_snapshots_storage;
mod cq_commands;
mod cq_policies;
mod device_crud;
mod device_layouts;
#[cfg(feature = "dnp3")]
mod dnp3;
mod downsampling;
mod egress;
#[cfg(feature = "ethernet-ip")]
mod enip_storage;
mod flows;
mod hub;
mod license_storage;
#[cfg(feature = "modbus")]
mod modbus_storage;
mod notifications;
mod oee;
#[cfg(feature = "opcua")]
mod opcua_storage;
mod recipes_storage;
mod reports;
mod retention;
#[cfg(feature = "s7")]
mod s7_storage;
mod schema;
mod scripts_storage;
mod security_events;
mod security_extras;
mod settings_audit;
mod settings_compliance;
mod settings_jit;
mod settings_kv;
mod settings_roles;
mod settings_scada;
mod settings_seed;
mod settings_shortcuts;
#[cfg(test)]
mod tests;
mod users;
pub use users::DELETED_USER;
mod voltrus_plc_storage;

use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::models::DataPoint;

#[derive(Clone, Debug)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
    db_path: String,
}

impl Storage {
    pub fn new(path: &str) -> Result<Self> {
        Self::new_with_key(path, None)
    }

    /// Lock the mutex, recovering from poison if a previous holder panicked.
    /// In SCADA, crashing and restarting is better than silently losing data,
    /// so we recover the lock and log a warning rather than propagating the panic.
    fn lock_conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(
                "Storage mutex poisoned — another thread panicked: {e}. Recovering lock."
            );
            e.into_inner()
        })
    }

    /// Simple health check — verify the connection is usable.
    pub fn conn_health(&self) -> bool {
        self.lock_conn().execute_batch("SELECT 1").is_ok()
    }

    pub fn new_with_key(path: &str, encryption_key: Option<&str>) -> Result<Self> {
        let conn =
            Connection::open(path).with_context(|| format!("Failed to open database: {path}"))?;

        if let Some(key) = encryption_key {
            if let Err(e) =
                conn.execute_batch(&format!("PRAGMA key = '{}';", key.replace("'", "''")))
            {
                anyhow::bail!(
                    "Failed to set database encryption key. Database will NOT be created without encryption. Error: {}",
                    e
                );
            } else {
                tracing::info!("Database encryption key applied successfully");
            }
        }

        schema::init_schema(&conn)?;

        // Initialize asset tables
        if let Err(e) = crate::asset::storage::AssetStorage::init_schema(&conn) {
            tracing::warn!("Failed to initialize asset schema: {}", e);
        }

        // Enable WAL mode for better concurrent read/write performance
        let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
        let _ = conn.execute_batch("PRAGMA synchronous=NORMAL;");
        let _ = conn.execute_batch("PRAGMA wal_autocheckpoint=1000;");
        let _ = conn.execute_batch("PRAGMA busy_timeout=5000;");

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: path.to_string(),
        })
    }

    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("Failed to open in-memory DB")?;
        schema::init_schema(&conn)?;
        if let Err(e) = crate::asset::storage::AssetStorage::init_schema(&conn) {
            tracing::warn!("Failed to initialize asset schema: {}", e);
        }
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path: ":memory:".to_string(),
        })
    }

    pub fn db_path(&self) -> &str {
        &self.db_path
    }

    /// Returns an `AssetStorage` handle backed by the same underlying connection.
    pub fn asset(&self) -> crate::asset::storage::AssetStorage {
        crate::asset::storage::AssetStorage::new(self.conn.clone())
    }

    pub async fn insert_batch(&self, records: &[(String, i64, String, f64)]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }

        let conn = self.conn.clone();
        let records = records.to_vec();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| { tracing::error!("Storage mutex poisoned: {e}"); e.into_inner() });
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            for (device_id, timestamp, metric, value) in &records {
                tx.execute(
                    "INSERT INTO telemetry (device_id, timestamp, metric, value) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![device_id, timestamp, metric, value],
                )
                .context("Insert failed")?;
            }
            tx.commit().context("Commit failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn query_history(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<DataPoint>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| {
                tracing::error!("Storage mutex poisoned: {e}");
                e.into_inner()
            });
            let mut stmt = conn
                .prepare(
                    "SELECT timestamp, value FROM telemetry
                     WHERE device_id = ?1 AND metric = ?2
                     AND timestamp >= ?3 AND timestamp <= ?4
                     ORDER BY timestamp
                     LIMIT 10000",
                )
                .context("Prepare failed")?;

            let rows = stmt
                .query_map(rusqlite::params![device_id, metric, from, to], |row| {
                    Ok(DataPoint {
                        timestamp: row.get(0)?,
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

    pub async fn query_device_metrics(
        &self,
        device_id: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<(i64, std::collections::HashMap<String, f64>)>> {
        let conn = self.conn.clone();
        let device_id = device_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| {
                tracing::error!("Storage mutex poisoned: {e}");
                e.into_inner()
            });
            let mut stmt = conn
                .prepare(
                    "SELECT timestamp, metric, value FROM telemetry
                     WHERE device_id = ?1
                     AND timestamp >= ?2 AND timestamp <= ?3
                     ORDER BY timestamp
                     LIMIT 10000",
                )
                .context("Prepare failed")?;

            let rows = stmt
                .query_map(rusqlite::params![device_id, from, to], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, f64>(2)?,
                    ))
                })
                .context("Query failed")?;

            let mut grouped: std::collections::BTreeMap<
                i64,
                std::collections::HashMap<String, f64>,
            > = std::collections::BTreeMap::new();
            for row in rows {
                let (ts, metric, value) = row.context("Row mapping failed")?;
                grouped.entry(ts).or_default().insert(metric, value);
            }

            Ok(grouped.into_iter().collect())
        })
        .await
        .context("Task join failed")?
    }

    /// Query a SQLite PRAGMA setting and return its string value.
    /// Used by the EDR compatibility module to verify I/O optimizations.
    pub async fn query_pragma(&self, pragma: &str) -> Result<String> {
        let conn = self.conn.clone();
        let sql = format!("PRAGMA {}", pragma);

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap_or_else(|e| {
                tracing::error!("Storage mutex poisoned: {e}");
                e.into_inner()
            });
            let result: String = conn
                .query_row(&sql, [], |row| row.get(0))
                .context(format!("Failed to query PRAGMA: {}", sql))?;
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }
}
