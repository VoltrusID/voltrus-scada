use super::Storage;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

/// Result of [`Storage::verify_audit_chain`].
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct AuditChainReport {
    /// True when every hashed row matches and links to the one before.
    pub ok: bool,
    pub checked: u64,
    /// Rows from before the hash chain existed.
    pub unhashed_legacy: u64,
    /// First row that was altered, inserted out of chain or follows a deleted row.
    pub first_bad_id: Option<i64>,
}

/// `sha256(prev_hash || row)`, with the row as a JSON array so no field
/// boundary can be shifted between two fields.
fn audit_row_hash(
    prev_hash: &str,
    id: i64,
    action: &str,
    username: &str,
    resource: &str,
    details: Option<&str>,
    created_at: i64,
) -> String {
    use sha2::{Digest, Sha256};
    let row = serde_json::json!([id, action, username, resource, details, created_at]);
    let mut hasher = Sha256::new();
    hasher.update(prev_hash.as_bytes());
    hasher.update(row.to_string().as_bytes());
    hex::encode(hasher.finalize())
}

impl Storage {
    // ── Audit Log ──

    pub async fn log_audit_event(
        &self,
        action: &str,
        username: &str,
        resource: &str,
        details: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let action = action.to_string();
        let username = username.to_string();
        let resource = resource.to_string();
        let details = details.map(|s| s.to_string());
        let now = crate::timeutil::now_secs();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // The previous hash is read and the new row hashed inside one
            // transaction under the connection lock, so concurrent writers
            // cannot fork the chain.
            let tx = conn
                .unchecked_transaction()
                .context("Begin audit insert failed")?;
            let prev_hash: String = tx
                .query_row(
                    "SELECT hash FROM audit_log WHERE hash IS NOT NULL ORDER BY id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .context("Read audit chain head failed")?
                .unwrap_or_default();
            tx.execute(
                "INSERT INTO audit_log (action, username, resource, details, created_at, prev_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![action, username, resource, details, now, prev_hash],
            )
            .context("Insert audit log failed")?;
            let id = tx.last_insert_rowid();
            let hash = audit_row_hash(
                &prev_hash,
                id,
                &action,
                &username,
                &resource,
                details.as_deref(),
                now,
            );
            tx.execute(
                "UPDATE audit_log SET hash = ?1 WHERE id = ?2",
                rusqlite::params![hash, id],
            )
            .context("Hash audit log failed")?;
            tx.commit().context("Commit audit log failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Walk the audit log's hash chain. Each row stores
    /// `sha256(prev_hash || row)`; editing a row, deleting one from the middle
    /// or inserting one without the chain shows up as the first bad id. Rows
    /// written before the chain existed (no hash, all before the first hashed
    /// row) are counted, not checked. Retention purges the oldest rows, so the
    /// first remaining row's `prev_hash` is taken as given.
    pub async fn verify_audit_chain(&self) -> Result<AuditChainReport> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, action, username, resource, details, created_at, prev_hash, hash
                     FROM audit_log ORDER BY id",
                )
                .context("Prepare audit verify failed")?;
            let mut rows = stmt.query([]).context("Query audit verify failed")?;
            let mut report = AuditChainReport::default();
            let mut head: Option<String> = None;
            while let Some(row) = rows.next().context("Read audit row failed")? {
                let id: i64 = row.get(0)?;
                let hash: Option<String> = row.get(7)?;
                let Some(hash) = hash else {
                    if head.is_none() {
                        report.unhashed_legacy += 1;
                        continue;
                    }
                    report.first_bad_id = Some(id);
                    break;
                };
                let prev_hash: String = row.get::<_, Option<String>>(6)?.unwrap_or_default();
                if head.as_ref().is_some_and(|h| *h != prev_hash) {
                    report.first_bad_id = Some(id);
                    break;
                }
                let details: Option<String> = row.get(4)?;
                let expected = audit_row_hash(
                    &prev_hash,
                    id,
                    &row.get::<_, String>(1)?,
                    &row.get::<_, String>(2)?,
                    &row.get::<_, String>(3)?,
                    details.as_deref(),
                    row.get(5)?,
                );
                if expected != hash {
                    report.first_bad_id = Some(id);
                    break;
                }
                report.checked += 1;
                head = Some(hash);
            }
            report.ok = report.first_bad_id.is_none();
            Ok(report)
        })
        .await
        .context("Task join failed")?
    }

    /// List audit logs with full-text search, filter, sort, and pagination.
    pub async fn list_audit_logs_filtered(
        &self,
        search: Option<&str>,
        action: Option<&str>,
        username: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
        page: u32,
        limit: u32,
        sort: &str,
        order: &str,
    ) -> Result<(Vec<crate::models::AuditLogEntry>, i64)> {
        let conn = self.conn.clone();
        let search = search.map(|s| s.to_string());
        let action = action.map(|s| s.to_string());
        let username = username.map(|s| s.to_string());
        let sort = sort.to_string();
        let order = order.to_string();

        let sort_col = match sort.as_str() {
            "action" | "username" => sort.clone(),
            _ => "created_at".to_string(),
        };
        let order_dir = match order.as_str() {
            "asc" => "ASC".to_string(),
            _ => "DESC".to_string(),
        };

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut where_sql = String::from("WHERE 1=1");
            let mut param_idx = 1;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref term) = search {
                where_sql.push_str(&format!(
                    " AND (details LIKE ?{idx} OR resource LIKE ?{idx} OR username LIKE ?{idx} OR action LIKE ?{idx})",
                    idx = param_idx
                ));
                let pattern = format!("%{}%", term);
                param_values.push(Box::new(pattern));
                param_idx += 1;
            }
            if let Some(ref a) = action {
                where_sql.push_str(&format!(" AND action = ?{}", param_idx));
                param_values.push(Box::new(a.clone()));
                param_idx += 1;
            }
            if let Some(ref u) = username {
                where_sql.push_str(&format!(" AND username = ?{}", param_idx));
                param_values.push(Box::new(u.clone()));
                param_idx += 1;
            }
            if let Some(f) = from {
                where_sql.push_str(&format!(" AND created_at >= ?{}", param_idx));
                param_values.push(Box::new(f));
                param_idx += 1;
            }
            if let Some(t) = to {
                where_sql.push_str(&format!(" AND created_at <= ?{}", param_idx));
                param_values.push(Box::new(t));
                param_idx += 1;
            }

            let count_sql = format!("SELECT COUNT(*) FROM audit_log {}", where_sql);
            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();
            let total: i64 = conn
                .query_row(&count_sql, params.as_slice(), |row| row.get(0))
                .context("Count audit logs failed")?;

            let offset = (page.saturating_sub(1)) * limit;
            let data_sql = format!(
                "SELECT id, action, username, resource, details, created_at
                 FROM audit_log {}
                 ORDER BY {} {}
                 LIMIT ?{} OFFSET ?{}",
                where_sql, sort_col, order_dir, param_idx, param_idx + 1
            );
            param_values.push(Box::new(limit));
            param_values.push(Box::new(offset));
            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&data_sql).context("Prepare audit query failed")?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok(crate::models::AuditLogEntry {
                        id: row.get(0)?,
                        action: row.get(1)?,
                        username: row.get(2)?,
                        resource: row.get(3)?,
                        details: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .context("Query audit logs failed")?
                .collect::<Result<Vec<_>, _>>()
                .context("Map audit logs failed")?;

            Ok((rows, total))
        })
        .await
        .context("Task join failed")?
    }

    /// Export audit logs as CSV string.
    pub async fn export_audit_logs_csv(
        &self,
        search: Option<&str>,
        action: Option<&str>,
        username: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<String> {
        let conn = self.conn.clone();
        let search = search.map(|s| s.to_string());
        let action = action.map(|s| s.to_string());
        let username = username.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut where_sql = String::from("WHERE 1=1");
            let mut param_idx = 1;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref term) = search {
                where_sql.push_str(&format!(
                    " AND (details LIKE ?{idx} OR resource LIKE ?{idx} OR username LIKE ?{idx} OR action LIKE ?{idx})",
                    idx = param_idx
                ));
                let pattern = format!("%{}%", term);
                param_values.push(Box::new(pattern));
                param_idx += 1;
            }
            if let Some(ref a) = action {
                where_sql.push_str(&format!(" AND action = ?{}", param_idx));
                param_values.push(Box::new(a.clone()));
                param_idx += 1;
            }
            if let Some(ref u) = username {
                where_sql.push_str(&format!(" AND username = ?{}", param_idx));
                param_values.push(Box::new(u.clone()));
                param_idx += 1;
            }
            if let Some(f) = from {
                where_sql.push_str(&format!(" AND created_at >= ?{}", param_idx));
                param_values.push(Box::new(f));
                param_idx += 1;
            }
            if let Some(t) = to {
                where_sql.push_str(&format!(" AND created_at <= ?{}", param_idx));
                param_values.push(Box::new(t));
                param_idx += 1;
            }
            let _ = param_idx;

            let sql = format!(
                "SELECT id, action, username, resource, details, created_at
                 FROM audit_log {}
                 ORDER BY created_at ASC",
                where_sql
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn
                .prepare(&sql)
                .context("Prepare audit CSV query failed")?;
            let rows = stmt
                .query_map(params.as_slice(), |row| {
                    Ok(crate::models::AuditLogEntry {
                        id: row.get(0)?,
                        action: row.get(1)?,
                        username: row.get(2)?,
                        resource: row.get(3)?,
                        details: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .context("Query audit CSV failed")?;

            let mut csv = String::from("id,action,username,resource,details,created_at\n");
            for row in rows {
                let entry = row.context("Map audit CSV row failed")?;
                csv.push_str(&format!(
                    "{},\"{}\",\"{}\",\"{}\",\"{}\",{}\n",
                    entry.id,
                    entry.action.replace('"', "\"\""),
                    entry.username.replace('"', "\"\""),
                    entry.resource.replace('"', "\"\""),
                    entry
                        .details
                        .as_deref()
                        .unwrap_or("")
                        .replace('"', "\"\"")
                        .replace('\n', "\\n"),
                    entry.created_at,
                ));
            }

            Ok(csv)
        })
        .await
        .context("Task join failed")?
    }

    /// Backward-compatible list_audit_logs for existing callers.
    pub async fn list_audit_logs(
        &self,
        limit: i64,
    ) -> Result<Vec<(i64, String, String, String, Option<String>, i64)>> {
        let (entries, _) = self
            .list_audit_logs_filtered(
                None,
                None,
                None,
                None,
                None,
                1,
                limit as u32,
                "created_at",
                "desc",
            )
            .await?;
        Ok(entries
            .into_iter()
            .map(|e| {
                (
                    e.id,
                    e.action,
                    e.username,
                    e.resource,
                    e.details,
                    e.created_at,
                )
            })
            .collect())
    }
}
