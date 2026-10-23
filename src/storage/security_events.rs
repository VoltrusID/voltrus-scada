use super::Storage;
use anyhow::{Context, Result};

impl Storage {
    /// Insert a security event into the append-only security_log table.
    pub async fn insert_security_event(
        &self,
        event: &crate::models::CreateSecurityEvent,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let event_type = event.event_type.clone();
        let severity = event.severity.clone();
        let source_ip = event.source_ip.clone();
        let user_agent = event.user_agent.clone();
        let username = event.username.clone();
        let details = event.details.clone();
        let now = crate::timeutil::now_secs();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO security_log (event_type, severity, source_ip, user_agent, username, details, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![event_type, severity, source_ip, user_agent, username, details, now],
            ).context("Insert security event failed")?;
            Ok(())
        }).await.context("Task join failed")?
    }

    /// List security events with optional filters and pagination.
    pub async fn list_security_events(
        &self,
        event_type: Option<&str>,
        severity: Option<&str>,
        from: Option<i64>,
        to: Option<i64>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<crate::models::SecurityEvent>> {
        let conn = self.conn.clone();
        let event_type = event_type.map(|s| s.to_string());
        let severity = severity.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut sql = String::from(
                "SELECT id, event_type, severity, source_ip, user_agent, username, details, created_at \
                 FROM security_log WHERE 1=1"
            );
            let mut param_idx = 1;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(et) = event_type {
                sql.push_str(&format!(" AND event_type = ?{}", param_idx));
                param_values.push(Box::new(et.to_string()));
                param_idx += 1;
            }
            if let Some(sev) = severity {
                sql.push_str(&format!(" AND severity = ?{}", param_idx));
                param_values.push(Box::new(sev.to_string()));
                param_idx += 1;
            }
            if let Some(f) = from {
                sql.push_str(&format!(" AND created_at >= ?{}", param_idx));
                param_values.push(Box::new(f));
                param_idx += 1;
            }
            if let Some(t) = to {
                sql.push_str(&format!(" AND created_at <= ?{}", param_idx));
                param_values.push(Box::new(t));
                param_idx += 1;
            }

            sql.push_str(&format!(" ORDER BY created_at DESC LIMIT ?{} OFFSET ?{}", param_idx, param_idx + 1));
            param_values.push(Box::new(limit));
            param_values.push(Box::new(offset));

            let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&sql).context("Prepare security events query failed")?;
            let events = stmt.query_map(params.as_slice(), |row| {
                Ok(crate::models::SecurityEvent {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    severity: row.get(2)?,
                    source_ip: row.get(3)?,
                    user_agent: row.get(4)?,
                    username: row.get(5)?,
                    details: row.get(6)?,
                    created_at: row.get(7)?,
                })
            }).context("Query security events failed")?
            .collect::<Result<Vec<_>, _>>()
            .context("Map security events failed")?;

            Ok(events)
        }).await.context("Task join failed")?
    }

    /// Export security events as CSV string for external audit tools.
    pub async fn export_security_events_csv(
        &self,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<String> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut sql = String::from(
                "SELECT id, event_type, severity, source_ip, user_agent, username, details, created_at \
                 FROM security_log WHERE 1=1"
            );
            let mut param_idx = 1;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(f) = from {
                sql.push_str(&format!(" AND created_at >= ?{}", param_idx));
                param_values.push(Box::new(f));
                param_idx += 1;
            }
            if let Some(t) = to {
                sql.push_str(&format!(" AND created_at <= ?{}", param_idx));
                param_values.push(Box::new(t));
                param_idx += 1;
            }
            let _ = param_idx;

            sql.push_str(" ORDER BY created_at ASC");
            let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();

            let mut stmt = conn.prepare(&sql).context("Prepare security CSV query failed")?;
            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok(crate::models::SecurityEvent {
                    id: row.get(0)?,
                    event_type: row.get(1)?,
                    severity: row.get(2)?,
                    source_ip: row.get(3)?,
                    user_agent: row.get(4)?,
                    username: row.get(5)?,
                    details: row.get(6)?,
                    created_at: row.get(7)?,
                })
            }).context("Query security CSV failed")?;

            let mut csv = String::from("id,event_type,severity,source_ip,user_agent,username,details,created_at\n");
            for row in rows {
                let event = row.context("Map security CSV row failed")?;
                csv.push_str(&format!(
                    "{},\"{}\",\"{}\",{},{},{},{},{}\n",
                    event.id,
                    event.event_type.replace('"', "\"\""),
                    event.severity.replace('"', "\"\""),
                    event.source_ip.as_deref().unwrap_or(""),
                    event.user_agent.as_deref().unwrap_or("").replace('"', "\"\""),
                    event.username.as_deref().unwrap_or("").replace('"', "\"\""),
                    event.details.as_deref().unwrap_or("").replace('"', "\"\"").replace('\n', "\\n"),
                    event.created_at,
                ));
            }

            Ok(csv)
        }).await.context("Task join failed")?
    }

    /// Get the current security thresholds configuration.
    pub async fn get_security_thresholds(&self) -> Result<crate::models::SecurityThresholds> {
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT failed_login_threshold, failed_login_window_secs, \
                        suspicious_401_threshold, suspicious_401_window_secs, \
                        rate_limit_threshold, rate_limit_window_secs \
                 FROM security_thresholds WHERE id = 1",
                )
                .context("Prepare thresholds query failed")?;

            let thresholds = stmt
                .query_row([], |row| {
                    Ok(crate::models::SecurityThresholds {
                        failed_login_threshold: row.get(0)?,
                        failed_login_window_secs: row.get(1)?,
                        suspicious_401_threshold: row.get(2)?,
                        suspicious_401_window_secs: row.get(3)?,
                        rate_limit_threshold: row.get(4)?,
                        rate_limit_window_secs: row.get(5)?,
                    })
                })
                .context("Query thresholds failed")?;

            Ok(thresholds)
        })
        .await
        .context("Task join failed")?
    }

    /// Update security thresholds configuration.
    pub async fn update_security_thresholds(
        &self,
        thresholds: &crate::models::SecurityThresholds,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let t = thresholds.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE security_thresholds SET \
                 failed_login_threshold = ?1, failed_login_window_secs = ?2, \
                 suspicious_401_threshold = ?3, suspicious_401_window_secs = ?4, \
                 rate_limit_threshold = ?5, rate_limit_window_secs = ?6 \
                 WHERE id = 1",
                rusqlite::params![
                    t.failed_login_threshold,
                    t.failed_login_window_secs,
                    t.suspicious_401_threshold,
                    t.suspicious_401_window_secs,
                    t.rate_limit_threshold,
                    t.rate_limit_window_secs,
                ],
            )
            .context("Update thresholds failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
