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
mod device_templates;
#[cfg(feature = "dnp3")]
mod dnp3;
mod downsampling;
mod egress;
#[cfg(feature = "ethernet-ip")]
mod enip_storage;
mod flows;
mod historian;
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

pub use historian::{Agg, ArchiveSink, BUCKET_SECS, ExpireReport, PackReport, PackedStats, Sample};

/// Group `(timestamp, metric, value)` rows by timestamp; a later row of the
/// same metric and timestamp replaces an earlier one.
fn group_by_timestamp(
    rows: Vec<(i64, String, f64)>,
) -> Vec<(i64, std::collections::HashMap<String, f64>)> {
    let mut grouped: std::collections::BTreeMap<i64, std::collections::HashMap<String, f64>> =
        std::collections::BTreeMap::new();
    for (ts, metric, value) in rows {
        grouped.entry(ts).or_default().insert(metric, value);
    }
    grouped.into_iter().collect()
}

/// The database encryption key, kept so side connections can open the same file.
/// Never printed.
#[derive(Clone, Default)]
struct DbKey(Option<Arc<str>>);

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() { "DbKey(set)" } else { "DbKey(none)" })
    }
}

#[derive(Clone, Debug)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
    db_path: String,
    key: DbKey,
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

    /// Run `f` on the shared connection on a blocking thread. The closure gets
    /// exclusive access for its whole duration, so it can open one transaction
    /// that spans several tables. Used by the project YAML import/export and by
    /// add-on editions that replicate the database.
    pub async fn with_conn<T, F>(&self, f: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let storage = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = storage.lock_conn();
            f(&mut conn)
        })
        .await
        .context("Task join failed")?
    }

    /// Open a second, read-only connection to the same database file. A long
    /// read (a consistent snapshot of every table) on it does not hold the
    /// shared connection, so the pollers and the API keep writing meanwhile.
    /// WAL mode gives the reader a stable view for the length of its
    /// transaction. Fails for an in-memory database.
    pub fn open_reader(&self) -> Result<Connection> {
        anyhow::ensure!(
            self.db_path != ":memory:",
            "an in-memory database has no file to open a second connection on"
        );
        let conn = Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("Failed to open database for reading: {}", self.db_path))?;
        if let Some(key) = self.key.0.as_deref() {
            conn.execute_batch(&format!("PRAGMA key = '{}';", key.replace('\'', "''")))
                .context("Failed to set database encryption key on reader")?;
        }
        conn.execute_batch("PRAGMA busy_timeout=5000;")?;
        Ok(conn)
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
            key: DbKey(encryption_key.map(Arc::from)),
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
            key: DbKey::default(),
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

    /// Full-resolution samples of one tag in `[from, to]`, oldest first, at
    /// most 10000. Packed and raw samples are merged transparently.
    pub async fn query_history(
        &self,
        device_id: &str,
        metric: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<DataPoint>> {
        let storage = self.clone();
        let device_id = device_id.to_string();
        let metric = metric.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let samples =
                historian::read_series(&conn, &device_id, &metric, from, to, Some(10000))?;
            Ok(samples
                .into_iter()
                .map(|(timestamp, value)| DataPoint { timestamp, value })
                .collect())
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
        let storage = self.clone();
        let device_id = device_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = storage.lock_conn();
            let rows = historian::read_device(&conn, &device_id, from, to, 10000)?;
            Ok(group_by_timestamp(rows))
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
