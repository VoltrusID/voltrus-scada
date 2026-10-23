use anyhow::{Context, Result};

use crate::asset::models::*;
use crate::asset::storage::core::AssetStorage;

impl AssetStorage {
    // ── SNMP Scan Results ──

    pub async fn clear_scan_results(&self) -> Result<()> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM snmp_scan_results", [])
                .context("Failed to clear scan results")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn insert_scan_result(&self, r: &SnmpScanResult) -> Result<()> {
        let conn = self.conn.clone();
        let ip = r.ip.clone();
        let sys_descr = r.sys_descr.clone();
        let sys_name = r.sys_name.clone();
        let sys_uptime = r.sys_uptime.clone();
        let vendor = r.vendor.clone();
        let model = r.model.clone();
        let firmware = r.firmware.clone();
        let mac_address = r.mac_address.clone();
        let scanned_at = r.scanned_at.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            conn.execute(
                "INSERT OR REPLACE INTO snmp_scan_results (ip, sys_descr, sys_name, sys_uptime, vendor, model, firmware, mac_address, scanned_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![ip, sys_descr, sys_name, sys_uptime, vendor, model, firmware, mac_address, scanned_at],
            )
            .context("Failed to insert scan result")?;

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_scan_results(&self) -> Result<Vec<SnmpScanResult>> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut stmt = conn
                .prepare(
                    "SELECT ip, sys_descr, sys_name, sys_uptime, vendor, model, firmware, mac_address, scanned_at
                     FROM snmp_scan_results ORDER BY ip",
                )
                .context("Failed to prepare scan results query")?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(SnmpScanResult {
                        ip: row.get(0)?,
                        sys_descr: row.get(1)?,
                        sys_name: row.get(2)?,
                        sys_uptime: row.get(3)?,
                        vendor: row.get(4)?,
                        model: row.get(5)?,
                        firmware: row.get(6)?,
                        mac_address: row.get(7)?,
                        scanned_at: row.get(8)?,
                    })
                })
                .context("Failed to query scan results")?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.context("Failed to map scan result row")?);
            }
            Ok(results)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_scan_result(&self, ip: &str) -> Result<Option<SnmpScanResult>> {
        let conn = self.conn.clone();
        let ip = ip.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT ip, sys_descr, sys_name, sys_uptime, vendor, model, firmware, mac_address, scanned_at
                 FROM snmp_scan_results WHERE ip = ?1",
                rusqlite::params![ip],
                |row| {
                    Ok(SnmpScanResult {
                        ip: row.get(0)?,
                        sys_descr: row.get(1)?,
                        sys_name: row.get(2)?,
                        sys_uptime: row.get(3)?,
                        vendor: row.get(4)?,
                        model: row.get(5)?,
                        firmware: row.get(6)?,
                        mac_address: row.get(7)?,
                        scanned_at: row.get(8)?,
                    })
                },
            );

            match result {
                Ok(scan_result) => Ok(Some(scan_result)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Failed to get scan result"),
            }
        })
        .await
        .context("Task join failed")?
    }
}
