use anyhow::{Context, Result};
use rand::Rng;
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

use crate::asset::models::*;

pub fn generate_id() -> String {
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(16)
        .map(char::from)
        .collect::<String>()
        .to_lowercase()
}

pub fn now_iso() -> String {
    let moment = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        moment.year(),
        moment.month() as u8,
        moment.day(),
        moment.hour(),
        moment.minute(),
        moment.second(),
    )
}

#[derive(Clone)]
pub struct AssetStorage {
    pub(crate) conn: Arc<Mutex<Connection>>,
}

impl AssetStorage {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    pub fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS assets (
                id TEXT PRIMARY KEY,
                device_ref TEXT UNIQUE,
                name TEXT NOT NULL,
                asset_type TEXT NOT NULL,
                vendor TEXT,
                model TEXT,
                firmware_version TEXT,
                serial_number TEXT,
                ip_address TEXT,
                mac_address TEXT,
                location TEXT,
                criticality TEXT NOT NULL DEFAULT 'medium',
                tags TEXT DEFAULT '[]',
                risk_score INTEGER DEFAULT 0,
                last_maintenance TEXT,
                notes TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_assets_device_ref ON assets(device_ref);
            CREATE INDEX IF NOT EXISTS idx_assets_ip ON assets(ip_address);
            CREATE INDEX IF NOT EXISTS idx_assets_type ON assets(asset_type);
            CREATE INDEX IF NOT EXISTS idx_assets_criticality ON assets(criticality);

            CREATE TABLE IF NOT EXISTS maintenance_logs (
                id TEXT PRIMARY KEY,
                asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
                description TEXT NOT NULL,
                technician TEXT NOT NULL,
                performed_at TEXT NOT NULL,
                created_by TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_maintenance_asset_id ON maintenance_logs(asset_id);

            CREATE TABLE IF NOT EXISTS firmware_catalog (
                id TEXT PRIMARY KEY,
                vendor TEXT NOT NULL,
                model TEXT NOT NULL,
                latest_version TEXT NOT NULL,
                release_date TEXT,
                notes TEXT,
                updated_at TEXT NOT NULL,
                UNIQUE(vendor, model)
            );

            CREATE TABLE IF NOT EXISTS snmp_scan_results (
                ip TEXT PRIMARY KEY,
                sys_descr TEXT,
                sys_name TEXT,
                sys_uptime TEXT,
                vendor TEXT,
                model TEXT,
                firmware TEXT,
                mac_address TEXT,
                scanned_at TEXT NOT NULL
            );",
        )
        .context("Failed to create asset tables")?;

        Ok(())
    }

    // ── CRUD ──

    pub async fn create_asset(&self, a: &CreateAssetRequest) -> Result<Asset> {
        let conn = self.conn.clone();
        let name = a.name.clone();
        let asset_type = a.asset_type.clone();
        let vendor = a.vendor.clone();
        let model = a.model.clone();
        let firmware_version = a.firmware_version.clone();
        let serial_number = a.serial_number.clone();
        let ip_address = a.ip_address.clone();
        let mac_address = a.mac_address.clone();
        let location = a.location.clone();
        let criticality = a.criticality.clone();
        let tags = serde_json::to_string(&a.tags).unwrap_or_else(|_| "[]".to_string());
        let notes = a.notes.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let id = generate_id();
            let now = now_iso();

            conn.execute(
                "INSERT INTO assets (id, name, asset_type, vendor, model, firmware_version,
                 serial_number, ip_address, mac_address, location, criticality, tags,
                 risk_score, last_maintenance, notes, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, NULL, ?13, ?14, ?15)",
                rusqlite::params![
                    id, name, asset_type, vendor, model, firmware_version,
                    serial_number, ip_address, mac_address, location, criticality, tags,
                    notes, now, now
                ],
            )
            .context("Failed to insert asset")?;

            Ok(Asset {
                id,
                device_ref: None,
                name,
                asset_type,
                vendor,
                model,
                firmware_version,
                serial_number,
                ip_address,
                mac_address,
                location,
                criticality,
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                risk_score: None,
                last_maintenance: None,
                notes,
                created_at: now.clone(),
                updated_at: now,
            })
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_assets(
        &self,
        asset_type: Option<&str>,
        criticality: Option<&str>,
        vendor: Option<&str>,
        search: Option<&str>,
    ) -> Result<Vec<Asset>> {
        let conn = self.conn.clone();
        let asset_type = asset_type.map(|s| s.to_string());
        let criticality = criticality.map(|s| s.to_string());
        let vendor = vendor.map(|s| s.to_string());
        let search = search.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut where_clauses: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref t) = asset_type {
                where_clauses.push("asset_type = ?".to_string());
                params.push(Box::new(t.clone()));
            }
            if let Some(ref c) = criticality {
                where_clauses.push("criticality = ?".to_string());
                params.push(Box::new(c.clone()));
            }
            if let Some(ref v) = vendor {
                where_clauses.push("vendor = ?".to_string());
                params.push(Box::new(v.clone()));
            }
            if let Some(ref s) = search {
                where_clauses
                    .push("(name LIKE ? OR ip_address LIKE ? OR vendor LIKE ?)".to_string());
                let pattern = format!("%{s}%");
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern.clone()));
                params.push(Box::new(pattern));
            }

            let where_sql = if where_clauses.is_empty() {
                String::new()
            } else {
                format!("WHERE {}", where_clauses.join(" AND "))
            };

            let sql = format!(
                "SELECT id, device_ref, name, asset_type, vendor, model, firmware_version,
                 serial_number, ip_address, mac_address, location, criticality, tags,
                 risk_score, last_maintenance, notes, created_at, updated_at
                 FROM assets {where_sql} ORDER BY updated_at DESC"
            );

            let mut stmt = conn.prepare(&sql).context("Failed to prepare list query")?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                    Ok(Asset {
                        id: row.get(0)?,
                        device_ref: row.get(1)?,
                        name: row.get(2)?,
                        asset_type: row.get(3)?,
                        vendor: row.get(4)?,
                        model: row.get(5)?,
                        firmware_version: row.get(6)?,
                        serial_number: row.get(7)?,
                        ip_address: row.get(8)?,
                        mac_address: row.get(9)?,
                        location: row.get(10)?,
                        criticality: row.get(11)?,
                        tags: serde_json::from_str(row.get::<_, String>(12)?.as_str())
                            .unwrap_or_default(),
                        risk_score: row.get(13)?,
                        last_maintenance: row.get(14)?,
                        notes: row.get(15)?,
                        created_at: row.get(16)?,
                        updated_at: row.get(17)?,
                    })
                })
                .context("Failed to query assets")?;

            let mut assets = Vec::new();
            for row in rows {
                assets.push(row.context("Failed to map asset row")?);
            }
            Ok(assets)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_asset(&self, id: &str) -> Result<Option<Asset>> {
        let conn = self.conn.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, device_ref, name, asset_type, vendor, model, firmware_version,
                 serial_number, ip_address, mac_address, location, criticality, tags,
                 risk_score, last_maintenance, notes, created_at, updated_at
                 FROM assets WHERE id = ?1",
                rusqlite::params![id],
                |row| {
                    Ok(Asset {
                        id: row.get(0)?,
                        device_ref: row.get(1)?,
                        name: row.get(2)?,
                        asset_type: row.get(3)?,
                        vendor: row.get(4)?,
                        model: row.get(5)?,
                        firmware_version: row.get(6)?,
                        serial_number: row.get(7)?,
                        ip_address: row.get(8)?,
                        mac_address: row.get(9)?,
                        location: row.get(10)?,
                        criticality: row.get(11)?,
                        tags: serde_json::from_str(row.get::<_, String>(12)?.as_str())
                            .unwrap_or_default(),
                        risk_score: row.get(13)?,
                        last_maintenance: row.get(14)?,
                        notes: row.get(15)?,
                        created_at: row.get(16)?,
                        updated_at: row.get(17)?,
                    })
                },
            );

            match result {
                Ok(asset) => Ok(Some(asset)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Failed to get asset"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_asset(&self, id: &str, fields: &serde_json::Value) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let fields = fields.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let allowed = [
                "name",
                "asset_type",
                "vendor",
                "model",
                "firmware_version",
                "serial_number",
                "ip_address",
                "mac_address",
                "location",
                "criticality",
                "notes",
                "tags",
            ];

            let mut set_clauses: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(obj) = fields.as_object() {
                for &key in &allowed {
                    if let Some(value) = obj.get(key) {
                        set_clauses.push(format!("{key} = ?"));
                        if key == "tags" {
                            params.push(Box::new(
                                serde_json::to_string(value).unwrap_or_else(|_| "[]".to_string()),
                            ));
                        } else {
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
            }

            if set_clauses.is_empty() {
                return Ok(());
            }

            let now = now_iso();
            set_clauses.push("updated_at = ?".to_string());
            params.push(Box::new(now));
            params.push(Box::new(id));

            let sql = format!("UPDATE assets SET {} WHERE id = ?", set_clauses.join(", "));

            conn.execute(&sql, rusqlite::params_from_iter(params.iter()))
                .context("Failed to update asset")?;

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_asset(&self, id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM assets WHERE id = ?1", rusqlite::params![id])
                .context("Failed to delete asset")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Field Updates ──

    pub async fn update_risk_score(&self, id: &str, score: i64) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = now_iso();
            conn.execute(
                "UPDATE assets SET risk_score = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![score, now, id],
            )
            .context("Failed to update risk score")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_last_maintenance(&self, id: &str, date: &str) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let date = date.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = now_iso();
            conn.execute(
                "UPDATE assets SET last_maintenance = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![date, now, id],
            )
            .context("Failed to update last maintenance")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Device Sync ──

    pub async fn link_device(
        &self,
        device_ref: &str,
        name: &str,
        ip: Option<&str>,
        asset_type: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let device_ref = device_ref.to_string();
        let name = name.to_string();
        let ip = ip.map(|s| s.to_string());
        let asset_type = asset_type.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let id = generate_id();
            let now = now_iso();

            conn.execute(
                "INSERT OR IGNORE INTO assets (id, device_ref, name, asset_type, ip_address, criticality, tags, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'low', '[]', ?6, ?7)",
                rusqlite::params![id, device_ref, name, asset_type, ip, now, now],
            )
            .context("Failed to link device")?;

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn unlink_device(&self, device_ref: &str) -> Result<()> {
        let conn = self.conn.clone();
        let device_ref = device_ref.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = now_iso();
            conn.execute(
                "UPDATE assets SET device_ref = NULL, criticality = 'low', updated_at = ?1 WHERE device_ref = ?2",
                rusqlite::params![now, device_ref],
            )
            .context("Failed to unlink device")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_device_ref(
        &self,
        device_ref: &str,
        name: &str,
        ip: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let device_ref = device_ref.to_string();
        let name = name.to_string();
        let ip = ip.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = now_iso();
            conn.execute(
                "UPDATE assets SET name = ?1, ip_address = ?2, updated_at = ?3 WHERE device_ref = ?4",
                rusqlite::params![name, ip, now, device_ref],
            )
            .context("Failed to update device ref")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── IP Conflict Detection ──

    pub async fn find_ip_conflicts(&self) -> Result<Vec<(String, Vec<Asset>)>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut stmt = conn
                .prepare(
                    "SELECT ip_address FROM assets
                     WHERE ip_address IS NOT NULL AND ip_address != ''
                     GROUP BY ip_address HAVING COUNT(*) > 1",
                )
                .context("Failed to prepare conflict query")?;

            let dup_ips: Vec<String> = stmt
                .query_map([], |row| row.get(0))
                .context("Failed to query duplicate IPs")?
                .filter_map(|r| r.ok())
                .collect();

            let mut conflicts = Vec::new();
            for ip in dup_ips {
                let mut asset_stmt = conn
                    .prepare(
                        "SELECT id, device_ref, name, asset_type, vendor, model, firmware_version,
                         serial_number, ip_address, mac_address, location, criticality, tags,
                         risk_score, last_maintenance, notes, created_at, updated_at
                         FROM assets WHERE ip_address = ?1",
                    )
                    .context("Failed to prepare asset query")?;

                let assets: Vec<Asset> = asset_stmt
                    .query_map(rusqlite::params![ip], |row| {
                        Ok(Asset {
                            id: row.get(0)?,
                            device_ref: row.get(1)?,
                            name: row.get(2)?,
                            asset_type: row.get(3)?,
                            vendor: row.get(4)?,
                            model: row.get(5)?,
                            firmware_version: row.get(6)?,
                            serial_number: row.get(7)?,
                            ip_address: row.get(8)?,
                            mac_address: row.get(9)?,
                            location: row.get(10)?,
                            criticality: row.get(11)?,
                            tags: serde_json::from_str(row.get::<_, String>(12)?.as_str())
                                .unwrap_or_default(),
                            risk_score: row.get(13)?,
                            last_maintenance: row.get(14)?,
                            notes: row.get(15)?,
                            created_at: row.get(16)?,
                            updated_at: row.get(17)?,
                        })
                    })
                    .context("Failed to query assets for IP")?
                    .filter_map(|r| r.ok())
                    .collect();

                conflicts.push((ip, assets));
            }

            Ok(conflicts)
        })
        .await
        .context("Task join failed")?
    }

    // ── Maintenance Logs ──

    pub async fn create_maintenance_log(
        &self,
        asset_id: &str,
        req: &CreateMaintenanceRequest,
        created_by: &str,
    ) -> Result<MaintenanceLog> {
        let conn = self.conn.clone();
        let asset_id = asset_id.to_string();
        let description = req.description.clone();
        let technician = req.technician.clone();
        let performed_at = req.performed_at.clone();
        let created_by = created_by.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let id = generate_id();
            let now = now_iso();

            conn.execute(
                "INSERT INTO maintenance_logs (id, asset_id, description, technician, performed_at, created_by, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, asset_id, description, technician, performed_at, created_by, now],
            )
            .context("Failed to insert maintenance log")?;

            conn.execute(
                "UPDATE assets SET last_maintenance = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![performed_at, now, asset_id],
            )
            .context("Failed to update asset last_maintenance")?;

            Ok(MaintenanceLog {
                id,
                asset_id,
                description,
                technician,
                performed_at,
                created_by,
                created_at: now,
            })
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_maintenance_logs(&self, asset_id: &str) -> Result<Vec<MaintenanceLog>> {
        let conn = self.conn.clone();
        let asset_id = asset_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut stmt = conn
                .prepare(
                    "SELECT id, asset_id, description, technician, performed_at, created_by, created_at
                     FROM maintenance_logs WHERE asset_id = ?1 ORDER BY performed_at DESC",
                )
                .context("Failed to prepare maintenance logs query")?;

            let rows = stmt
                .query_map(rusqlite::params![asset_id], |row| {
                    Ok(MaintenanceLog {
                        id: row.get(0)?,
                        asset_id: row.get(1)?,
                        description: row.get(2)?,
                        technician: row.get(3)?,
                        performed_at: row.get(4)?,
                        created_by: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .context("Failed to query maintenance logs")?;

            let mut logs = Vec::new();
            for row in rows {
                logs.push(row.context("Failed to map maintenance log row")?);
            }
            Ok(logs)
        })
        .await
        .context("Task join failed")?
    }
}
