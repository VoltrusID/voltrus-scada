use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    pub async fn get_license_activation(
        &self,
    ) -> Result<Option<crate::models::LicenseActivationRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT license_key, fingerprint_hash, activation_blob, fingerprint_hostname, fingerprint_cpu, fingerprint_disk_serial, fingerprint_mac, fingerprint_machine_id, activated_at, expires_at, tier, device_limit FROM license_activation ORDER BY id DESC LIMIT 1")
                .context("Prepare failed")?;
            let result = stmt.query_row([], |row| {
                Ok(crate::models::LicenseActivationRow {
                    license_key: row.get(0)?,
                    fingerprint_hash: row.get(1)?,
                    activation_blob: row.get(2)?,
                    fingerprint_hostname: row.get(3)?,
                    fingerprint_cpu: row.get(4)?,
                    fingerprint_disk_serial: row.get(5)?,
                    fingerprint_mac: row.get(6)?,
                    fingerprint_machine_id: row.get(7)?,
                    activated_at: row.get(8)?,
                    expires_at: row.get(9)?,
                    tier: row.get(10)?,
                    device_limit: row.get(11)?,
                })
            }).ok();
            Ok(result)
        }).await.context("Task join failed")?
    }

    pub async fn store_license_activation(
        &self,
        row: &crate::models::LicenseActivationRow,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let row = row.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let _ = conn.execute("DELETE FROM license_activation", []);
            conn.execute(
                "INSERT INTO license_activation (license_key, fingerprint_hash, fingerprint_hostname, fingerprint_cpu, fingerprint_disk_serial, fingerprint_mac, fingerprint_machine_id, activation_blob, activated_at, expires_at, tier, device_limit)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    row.license_key,
                    row.fingerprint_hash,
                    row.fingerprint_hostname,
                    row.fingerprint_cpu,
                    row.fingerprint_disk_serial,
                    row.fingerprint_mac,
                    row.fingerprint_machine_id,
                    row.activation_blob,
                    row.activated_at,
                    row.expires_at,
                    row.tier,
                    row.device_limit,
                ],
            ).context("Store license activation failed")?;
            Ok(())
        }).await.context("Task join failed")?
    }

    pub async fn clear_license_activation(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM license_activation", [])
                .context("Clear license activation failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
