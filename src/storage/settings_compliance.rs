use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    // ── Electronic signatures ──

    pub async fn create_electronic_signature(
        &self,
        user_id: &str,
        action: &str,
        resource_type: &str,
        resource_id: &str,
        reason: &str,
        meaning: &str,
        signature_hash: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let user_id = user_id.to_string();
        let action = action.to_string();
        let resource_type = resource_type.to_string();
        let resource_id = resource_id.to_string();
        let reason = reason.to_string();
        let meaning = meaning.to_string();
        let signature_hash = signature_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO electronic_signatures (user_id, action, resource_type, resource_id, reason, meaning, signature_hash, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![user_id, action, resource_type, resource_id, reason, meaning, signature_hash, now],
            )
            .context("Insert electronic signature failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_electronic_signatures(
        &self,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
    ) -> Result<Vec<crate::compliance::ElectronicSignature>> {
        let conn = self.conn.clone();
        let rt = resource_type.map(|s| s.to_string());
        let ri = resource_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let base_sql = "SELECT id, user_id, action, resource_type, resource_id, reason, meaning, signature_hash, created_at FROM electronic_signatures";
            let mut sql = String::from(base_sql);
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if rt.is_some() {
                sql.push_str(" WHERE resource_type = ?1");
                param_values.push(Box::new(rt.as_deref().unwrap()));
                if ri.is_some() {
                    sql.push_str(" AND resource_id = ?2");
                    param_values.push(Box::new(ri.as_deref().unwrap()));
                }
            }
            sql.push_str(" ORDER BY created_at DESC");

            let param_refs: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|b| b.as_ref()).collect();

            let mut stmt = conn.prepare(&sql).context("Prepare failed")?;
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(crate::compliance::ElectronicSignature {
                    id: row.get(0)?,
                    user_id: row.get(1)?,
                    action: row.get(2)?,
                    resource_type: row.get(3)?,
                    resource_id: row.get(4)?,
                    reason: row.get(5)?,
                    meaning: row.get(6)?,
                    signature_hash: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Compliance config ──

    pub async fn get_compliance_config(
        &self,
    ) -> Result<Option<crate::compliance::ComplianceConfig>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let config_json: Option<String> = conn
                .query_row(
                    "SELECT config FROM compliance_config WHERE id = 1",
                    [],
                    |row| row.get(0),
                )
                .ok();
            match config_json {
                Some(json_str) => {
                    let config: crate::compliance::ComplianceConfig =
                        serde_json::from_str(&json_str)
                            .context("Failed to parse compliance config")?;
                    Ok(Some(config))
                }
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn upsert_compliance_config(
        &self,
        config: &crate::compliance::ComplianceConfig,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let config_json =
            serde_json::to_string(config).context("Failed to serialize compliance config")?;
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO compliance_config (id, config) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET config = ?1",
                rusqlite::params![config_json],
            )
            .context("Upsert compliance config failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
