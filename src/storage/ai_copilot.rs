use super::Storage;
use anyhow::{Context, Result};

use crate::models::{AiProposal, EquipmentTemplate};

impl Storage {
    // ── AI Copilot Settings (uses the settings table with ai_ prefix convention) ──

    pub async fn get_ai_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.clone();
        let key = format!("ai_{}", key);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT value FROM settings WHERE key = ?1")
                .context("Prepare failed")?;
            let mut rows = stmt
                .query_map(rusqlite::params![key], |row| row.get::<_, String>(0))
                .context("Query failed")?;
            match rows.next() {
                Some(Ok(v)) => Ok(Some(v)),
                Some(Err(e)) => Err(e).context("Row mapping failed"),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn set_ai_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.conn.clone();
        let key = format!("ai_{}", key);
        let value = value.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
            .context("Upsert AI setting failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── AI Proposals ──

    pub async fn create_ai_proposal(
        &self,
        proposal_type: &str,
        title: &str,
        context: &str,
        proposed_config: &str,
        target_entity_id: Option<&str>,
        model: &str,
        tokens_used: i64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let proposal_type = proposal_type.to_string();
        let title = title.to_string();
        let context = context.to_string();
        let proposed_config = proposed_config.to_string();
        let target_entity_id = target_entity_id.map(|s| s.to_string());
        let model = model.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            conn.execute(
                "INSERT INTO ai_proposals (proposal_type, title, context, proposed_config, target_entity_id, model, tokens_used, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?8)",
                rusqlite::params![proposal_type, title, context, proposed_config, target_entity_id, model, tokens_used, now],
            )
            .context("Insert AI proposal failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_ai_proposal(&self, id: i64) -> Result<Option<AiProposal>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, proposal_type, title, context, proposed_config, target_entity_id, model, tokens_used, status, reviewed_by, applied_config, created_at, updated_at
                     FROM ai_proposals WHERE id = ?1",
                )
                .context("Prepare failed")?;
            let mut rows = stmt.query_map(rusqlite::params![id], |row| {
                Ok(AiProposal {
                    id: row.get(0)?,
                    proposal_type: row.get(1)?,
                    title: row.get(2)?,
                    context: row.get(3)?,
                    proposed_config: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(serde_json::Value::Null),
                    target_entity_id: row.get(5)?,
                    model: row.get(6)?,
                    tokens_used: row.get(7)?,
                    status: row.get(8)?,
                    reviewed_by: row.get(9)?,
                    applied_config: row.get::<_, Option<String>>(10)?.and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            }).context("Query failed")?;
            match rows.next() {
                Some(Ok(v)) => Ok(Some(v)),
                Some(Err(e)) => Err(e).context("Row mapping failed"),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_ai_proposals(
        &self,
        proposal_type: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<AiProposal>> {
        let conn = self.conn.clone();
        let proposal_type = proposal_type.map(|s| s.to_string());
        let status = status.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT id, proposal_type, title, context, proposed_config, target_entity_id, model, tokens_used, status, reviewed_by, applied_config, created_at, updated_at
                 FROM ai_proposals WHERE 1=1",
            );
            if proposal_type.is_some() {
                sql.push_str(" AND proposal_type = ? ");
            }
            if status.is_some() {
                sql.push_str(" AND status = ? ");
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");

            let mut stmt = conn.prepare(&sql).context("Prepare failed")?;

            let _param_idx;
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = {
                let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(ref pt) = proposal_type {
                    p.push(Box::new(pt.clone()));
                }
                if let Some(ref s) = status {
                    p.push(Box::new(s.clone()));
                }
                p.push(Box::new(limit));
                _param_idx = p.len();
                p
            };

            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(AiProposal {
                    id: row.get(0)?,
                    proposal_type: row.get(1)?,
                    title: row.get(2)?,
                    context: row.get(3)?,
                    proposed_config: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(serde_json::Value::Null),
                    target_entity_id: row.get(5)?,
                    model: row.get(6)?,
                    tokens_used: row.get(7)?,
                    status: row.get(8)?,
                    reviewed_by: row.get(9)?,
                    applied_config: row.get::<_, Option<String>>(10)?.and_then(|s| serde_json::from_str(&s).ok()),
                    created_at: row.get(11)?,
                    updated_at: row.get(12)?,
                })
            }).context("Query failed")?;

            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_ai_proposal_status(
        &self,
        id: i64,
        status: &str,
        reviewed_by: Option<&str>,
        applied_config: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let status = status.to_string();
        let reviewed_by = reviewed_by.map(|s| s.to_string());
        let applied_config = applied_config.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            conn.execute(
                "UPDATE ai_proposals SET status = ?1, reviewed_by = ?2, applied_config = ?3, updated_at = ?4 WHERE id = ?5",
                rusqlite::params![status, reviewed_by, applied_config, now, id],
            )
            .context("Update AI proposal status failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn mark_ai_proposal_applied(
        &self,
        id: i64,
        applied_config: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let applied_config = applied_config.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            conn.execute(
                "UPDATE ai_proposals SET status = 'applied', applied_config = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![applied_config, now, id],
            )
            .context("Mark AI proposal applied failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Equipment Templates ──

    pub async fn list_equipment_templates(
        &self,
        industry: Option<&str>,
    ) -> Result<Vec<EquipmentTemplate>> {
        let conn = self.conn.clone();
        let industry = industry.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT id, name, industry, equipment_type, tag_template, alarm_template, dashboard_template, flow_template
                 FROM equipment_templates WHERE 1=1",
            );
            if industry.is_some() {
                sql.push_str(" AND industry = ?");
            }
            let params: Vec<Box<dyn rusqlite::types::ToSql>> = {
                let mut p: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(ref ind) = industry {
                    p.push(Box::new(ind.clone()));
                }
                p
            };
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = conn.prepare(&sql).context("Prepare failed")?;
            let rows = stmt.query_map(param_refs.as_slice(), |row| {
                Ok(EquipmentTemplate {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    industry: row.get(2)?,
                    equipment_type: row.get(3)?,
                    tag_template: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or(serde_json::Value::Array(vec![])),
                    alarm_template: serde_json::from_str(&row.get::<_, String>(5)?).unwrap_or(serde_json::Value::Array(vec![])),
                    dashboard_template: serde_json::from_str(&row.get::<_, String>(6)?).unwrap_or(serde_json::Value::Object(Default::default())),
                    flow_template: serde_json::from_str(&row.get::<_, String>(7)?).unwrap_or(serde_json::Value::Array(vec![])),
                })
            }).context("Query failed")?;

            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
