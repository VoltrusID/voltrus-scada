use super::Storage;
use anyhow::{Context, Result};

use crate::models::{Command, CommandAuditEntry, CommandQuery};

impl Storage {
    pub async fn insert_command(&self, cmd: &Command) -> Result<()> {
        let conn = self.conn.clone();
        let cmd = cmd.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO commands (id, data_source_id, device_id, protocol, value, status, policy,
                     scheduled_at, retry_count, max_retries, error_message, created_at, sent_at, acked_at,
                     resolved_at, created_by, device_ref)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                rusqlite::params![
                    cmd.id, cmd.data_source_id, cmd.device_id, cmd.protocol, cmd.value,
                    cmd.status, cmd.policy, cmd.scheduled_at, cmd.retry_count, cmd.max_retries,
                    cmd.error_message, cmd.created_at, cmd.sent_at, cmd.acked_at, cmd.resolved_at,
                    cmd.created_by, cmd.device_ref,
                ],
            )
            .context("Insert command failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_command_status(
        &self,
        command_id: &str,
        new_status: &str,
        error_message: Option<&str>,
        changed_by: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let command_id = command_id.to_string();
        let new_status = new_status.to_string();
        let error_message = error_message.map(|s| s.to_string());
        let changed_by = changed_by.to_string();
        let reason = reason.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let tx = conn.unchecked_transaction().context("Begin tx failed")?;
            let now = crate::timeutil::now_secs();

            // Fetch current status for audit log
            let old_status: String = tx
                .query_row(
                    "SELECT status FROM commands WHERE id = ?",
                    rusqlite::params![command_id],
                    |row| row.get(0),
                )
                .context("Command not found")?;

            // Build SQL and params based on new status (using unnumbered ? placeholders
            // bound positionally by rusqlite to avoid off-by-one errors).
            match new_status.as_str() {
                "sent" => {
                    tx.execute(
                        "UPDATE commands SET status = ?, sent_at = ?, error_message = ? WHERE id = ?",
                        rusqlite::params![new_status, now, error_message, command_id],
                    )
                    .context("Update command status failed")?;
                }
                "acked" => {
                    tx.execute(
                        "UPDATE commands SET status = ?, acked_at = ?, resolved_at = ?, error_message = ? WHERE id = ?",
                        rusqlite::params![new_status, now, now, error_message, command_id],
                    )
                    .context("Update command status failed")?;
                }
                "failed" | "expired" | "cancelled" => {
                    tx.execute(
                        "UPDATE commands SET status = ?, resolved_at = ?, error_message = ? WHERE id = ?",
                        rusqlite::params![new_status, now, error_message, command_id],
                    )
                    .context("Update command status failed")?;
                }
                "queued" => {
                    tx.execute(
                        "UPDATE commands SET status = ?, retry_count = retry_count + 1, error_message = ? WHERE id = ?",
                        rusqlite::params![new_status, error_message, command_id],
                    )
                    .context("Update command status failed")?;
                }
                _ => {
                    tx.execute(
                        "UPDATE commands SET status = ?, error_message = ? WHERE id = ?",
                        rusqlite::params![new_status, error_message, command_id],
                    )
                    .context("Update command status failed")?;
                }
            }

            // Insert audit log entry
            tx.execute(
                "INSERT INTO command_audit_log (command_id, old_status, new_status, changed_by, reason, created_at)
                 VALUES (?, ?, ?, ?, ?, ?)",
                rusqlite::params![command_id, old_status, new_status, changed_by, reason, now],
            )
            .context("Insert command audit log failed")?;

            tx.commit().context("Commit failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_command(&self, command_id: &str) -> Result<Option<Command>> {
        let conn = self.conn.clone();
        let command_id = command_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT id, data_source_id, device_id, protocol, value, status, policy,
                        scheduled_at, retry_count, max_retries, error_message, created_at,
                        sent_at, acked_at, resolved_at, created_by, device_ref
                 FROM commands WHERE id = ?1",
                rusqlite::params![command_id],
                |row| {
                    Ok(Command {
                        id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        device_id: row.get(2)?,
                        protocol: row.get(3)?,
                        value: row.get(4)?,
                        status: row.get(5)?,
                        policy: row.get(6)?,
                        scheduled_at: row.get(7)?,
                        retry_count: row.get(8)?,
                        max_retries: row.get(9)?,
                        error_message: row.get(10)?,
                        created_at: row.get(11)?,
                        sent_at: row.get(12)?,
                        acked_at: row.get(13)?,
                        resolved_at: row.get(14)?,
                        created_by: row.get(15)?,
                        device_ref: row.get(16)?,
                        readback_status: None,
                        readback_value: None,
                        readback_attempts: 0,
                        readback_ts: None,
                    })
                },
            );
            match result {
                Ok(cmd) => Ok(Some(cmd)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e).context("Get command failed"),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_commands(&self, query: &CommandQuery) -> Result<Vec<Command>> {
        let conn = self.conn.clone();
        let tag = query.tag.clone();
        let status = query.status.clone();
        let device = query.device.clone();
        let from = query.from;
        let to = query.to;
        let limit = query.limit.unwrap_or(50);
        let offset = query.offset.unwrap_or(0);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut sql = String::from(
                "SELECT id, data_source_id, device_id, protocol, value, status, policy,
                        scheduled_at, retry_count, max_retries, error_message, created_at,
                        sent_at, acked_at, resolved_at, created_by, device_ref
                 FROM commands WHERE 1=1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref s) = status {
                sql.push_str(" AND status = ?");
                params.push(Box::new(s.clone()));
            }
            if let Some(d) = &device {
                // `device` is the device identifier as text: the numeric
                // id of an S7/OPC UA/... device, or a Modbus device's text id.
                if let Ok(id) = d.parse::<i64>() {
                    sql.push_str(" AND (device_id = ? OR device_ref = ?)");
                    params.push(Box::new(id));
                } else {
                    sql.push_str(" AND device_ref = ?");
                }
                params.push(Box::new(d.clone()));
            }
            if let Some(f) = from {
                sql.push_str(" AND created_at >= ?");
                params.push(Box::new(f));
            }
            if let Some(t) = to {
                sql.push_str(" AND created_at <= ?");
                params.push(Box::new(t));
            }
            if let Some(ref t) = tag {
                // tag matches data_source_id via key lookup
                sql.push_str(" AND data_source_id IN (SELECT id FROM data_sources WHERE key = ?)");
                params.push(Box::new(t.clone()));
            }

            sql.push_str(" ORDER BY created_at DESC LIMIT ? OFFSET ?");
            params.push(Box::new(limit));
            params.push(Box::new(offset));

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|p| p.as_ref()).collect();
            let mut stmt = conn.prepare(&sql).context("Prepare list_commands failed")?;
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(Command {
                        id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        device_id: row.get(2)?,
                        protocol: row.get(3)?,
                        value: row.get(4)?,
                        status: row.get(5)?,
                        policy: row.get(6)?,
                        scheduled_at: row.get(7)?,
                        retry_count: row.get(8)?,
                        max_retries: row.get(9)?,
                        error_message: row.get(10)?,
                        created_at: row.get(11)?,
                        sent_at: row.get(12)?,
                        acked_at: row.get(13)?,
                        resolved_at: row.get(14)?,
                        created_by: row.get(15)?,
                        device_ref: row.get(16)?,
                        readback_status: None,
                        readback_value: None,
                        readback_attempts: 0,
                        readback_ts: None,
                    })
                })
                .context("Query list_commands failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_queued_commands(&self) -> Result<Vec<Command>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, data_source_id, device_id, protocol, value, status, policy,
                        scheduled_at, retry_count, max_retries, error_message, created_at,
                        sent_at, acked_at, resolved_at, created_by, device_ref
                 FROM commands
                 WHERE status IN ('queued', 'pending')
                 ORDER BY created_at ASC",
                )
                .context("Prepare get_queued_commands failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Command {
                        id: row.get(0)?,
                        data_source_id: row.get(1)?,
                        device_id: row.get(2)?,
                        protocol: row.get(3)?,
                        value: row.get(4)?,
                        status: row.get(5)?,
                        policy: row.get(6)?,
                        scheduled_at: row.get(7)?,
                        retry_count: row.get(8)?,
                        max_retries: row.get(9)?,
                        error_message: row.get(10)?,
                        created_at: row.get(11)?,
                        sent_at: row.get(12)?,
                        acked_at: row.get(13)?,
                        resolved_at: row.get(14)?,
                        created_by: row.get(15)?,
                        device_ref: row.get(16)?,
                        readback_status: None,
                        readback_value: None,
                        readback_attempts: 0,
                        readback_ts: None,
                    })
                })
                .context("Query get_queued_commands failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_command_audit(&self, command_id: &str) -> Result<Vec<CommandAuditEntry>> {
        let conn = self.conn.clone();
        let command_id = command_id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, command_id, old_status, new_status, changed_by, reason, created_at
                 FROM command_audit_log
                 WHERE command_id = ?1
                 ORDER BY created_at ASC",
                )
                .context("Prepare get_command_audit failed")?;
            let rows = stmt
                .query_map(rusqlite::params![command_id], |row| {
                    Ok(CommandAuditEntry {
                        id: row.get(0)?,
                        command_id: row.get(1)?,
                        old_status: row.get(2)?,
                        new_status: row.get(3)?,
                        changed_by: row.get(4)?,
                        reason: row.get(5)?,
                        created_at: row.get(6)?,
                    })
                })
                .context("Query get_command_audit failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
