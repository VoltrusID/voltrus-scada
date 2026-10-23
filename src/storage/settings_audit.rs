use super::Storage;
use anyhow::{Context, Result};

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
            conn.execute(
                "INSERT INTO audit_log (action, username, resource, details, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![action, username, resource, details, now],
            )
            .context("Insert audit log failed")?;
            Ok(())
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
