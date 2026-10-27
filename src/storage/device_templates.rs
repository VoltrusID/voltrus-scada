use super::Storage;
use anyhow::Context;

use crate::device_templates::DeviceTemplate;

impl Storage {
    /// Store a template. Returns false when `(template_id, source, version)` is already stored.
    pub async fn insert_device_template(&self, t: &DeviceTemplate) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let body = serde_json::to_string(t).context("Serialize template failed")?;
        let t = t.clone();
        let now = crate::timeutil::now_secs();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let rows = conn
                .execute(
                    "INSERT OR IGNORE INTO device_templates
                     (template_id, source, version, manufacturer, model, protocol, verified, body, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        t.template_id,
                        t.source,
                        t.version,
                        t.manufacturer,
                        t.model,
                        t.protocol.as_str(),
                        t.verified as i64,
                        body,
                        now
                    ],
                )
                .context("Insert device template failed")?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// Every stored template, newest first.
    pub async fn list_device_templates(&self) -> anyhow::Result<Vec<DeviceTemplate>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT body FROM device_templates ORDER BY id DESC")
                .context("Prepare failed")?;
            let bodies = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .context("Query failed")?
                .collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")?;
            bodies
                .iter()
                .map(|b| {
                    serde_json::from_str::<DeviceTemplate>(b)
                        .context("Stored device template is not valid JSON")
                })
                .collect()
        })
        .await
        .context("Task join failed")?
    }

    /// Delete one stored template; false when there is none.
    pub async fn delete_device_template(
        &self,
        template_id: &str,
        source: &str,
        version: &str,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let (template_id, source, version) = (
            template_id.to_string(),
            source.to_string(),
            version.to_string(),
        );
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let rows = conn
                .execute(
                    "DELETE FROM device_templates WHERE template_id = ?1 AND source = ?2 AND version = ?3",
                    rusqlite::params![template_id, source, version],
                )
                .context("Delete device template failed")?;
            Ok(rows > 0)
        })
        .await
        .context("Task join failed")?
    }
}
