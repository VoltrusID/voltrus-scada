use super::Storage;
use anyhow::{Context, Result};

use crate::models::{AlarmAnalyticsEntry, AlarmAuditEntry, AlarmJournalEntry, AlarmSummary};

impl Storage {
    // ── Alarm lifecycle: resolve, acknowledge, shelf ──

    /// Resolve all unresolved alarms for a source_key, applying deadband hysteresis.
    /// Only resolves alarms where `value < threshold - deadband`.
    /// Pass `None` for current_value to resolve unconditionally (no deadband check).
    /// Resolves regardless of acknowledgment status — once the value is back to
    /// normal, the alarm should clear so a new one can fire later.
    pub async fn resolve_active_alarms(
        &self,
        source_key: &str,
        current_value: Option<f64>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let source_key = source_key.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();

            // Build the query: resolve unresolved alarms where value < threshold - deadband.
            // When current_value is None, resolve all unresolved alarms (no deadband check).
            let (alarm_ids, details_json): (Vec<i64>, Vec<String>) = {
                let sql = match current_value {
                    Some(_) => {
                        "SELECT id, '{\"value\":' || ?1 || ', \"threshold\":' || threshold || ', \"deadband\":' || COALESCE(deadband, 0) || '}'
                         FROM alarms WHERE source_key = ?2 AND resolved_at IS NULL
                           AND (
                            (COALESCE(alarm_type, 'high') = 'high'
                             AND ?1 < threshold - COALESCE(deadband, 0))
                         OR
                            (COALESCE(alarm_type, 'low') = 'low'
                             AND ?1 > threshold + COALESCE(deadband, 0))
                       )"
                    }
                    None => {
                        "SELECT id, '{}' FROM alarms WHERE source_key = ?1 AND resolved_at IS NULL"
                    }
                };
                let mut stmt = conn.prepare(sql)?;
                let rows = match current_value {
                    Some(cv) => {
                        let rows = stmt.query_map(rusqlite::params![cv, source_key], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                        })?;
                        rows.collect::<Result<Vec<_>, _>>().unwrap_or_default()
                    }
                    None => {
                        let rows = stmt.query_map(rusqlite::params![source_key], |row| {
                            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                        })?;
                        rows.collect::<Result<Vec<_>, _>>().unwrap_or_default()
                    }
                };
                rows.into_iter().unzip()
            };

            if alarm_ids.is_empty() {
                return Ok(());
            }

            // Resolve the matched alarms
            let placeholders: Vec<String> = alarm_ids.iter().map(|_| "?".to_string()).collect();
            let sql = format!(
                "UPDATE alarms SET resolved_at = ?, state = 'resolved' WHERE id IN ({})",
                placeholders.join(",")
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];
            for id in &alarm_ids {
                params.push(Box::new(*id));
            }
            conn.execute(&sql, rusqlite::params_from_iter(params))
                .context("Resolve alarms failed")?;

            // Create audit entries for each resolved alarm
            for (i, alarm_id) in alarm_ids.iter().enumerate() {
                let details = details_json.get(i).map(|s| s.as_str()).unwrap_or("{}");
                let _ = conn.execute(
                    "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'auto_resolved', 'system', ?2, ?3)",
                    rusqlite::params![alarm_id, details, now],
                );
            }

            // Journal: append-only lifecycle history for each resolved alarm.
            for alarm_id in &alarm_ids {
                let _ = conn.execute(
                    "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'resolved', ?2, NULL)",
                    rusqlite::params![alarm_id, now],
                );
            }

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn acknowledge_alarm(
        &self,
        id: i64,
        username: &str,
        cooldown_secs: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let cooldown_until = if cooldown_secs > 0 { Some(now + cooldown_secs) } else { None };
            conn.execute(
                "UPDATE alarms SET acknowledged_at = ?1, acknowledged_by = ?2, cooldown_until = ?3, state = 'acknowledged' WHERE id = ?4",
                rusqlite::params![now, username, cooldown_until, id],
            )
            .context("Acknowledge alarm failed")?;
            // Create audit entry
            let _ = conn.execute(
                "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'acknowledged', ?2, ?3, ?4)",
                rusqlite::params![id, username, format!("{{\"cooldown_secs\":{}}}", cooldown_secs), now],
            );
            // Journal: append-only lifecycle history.
            let _ = conn.execute(
                "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'acknowledged', ?2, ?3)",
                rusqlite::params![id, now, username],
            );
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn acknowledge_all(&self, username: &str, cooldown_secs: i64) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let cooldown_until = if cooldown_secs > 0 { Some(now + cooldown_secs) } else { None };

            // Find alarms that will be acknowledged, for audit entries
            let alarm_ids: Vec<i64> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM alarms WHERE acknowledged_at IS NULL",
                )?;
                let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
                rows.collect::<Result<Vec<_>, _>>().unwrap_or_default()
            };

            conn.execute(
                "UPDATE alarms SET acknowledged_at = ?1, acknowledged_by = ?2, cooldown_until = ?3, state = 'acknowledged'
                 WHERE acknowledged_at IS NULL",
                rusqlite::params![now, username, cooldown_until],
            )
            .context("Acknowledge all alarms failed")?;

            // Create audit entries for each acknowledged alarm
            for alarm_id in alarm_ids {
                let _ = conn.execute(
                    "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'acknowledged', ?2, ?3, ?4)",
                    rusqlite::params![alarm_id, username, format!("{{\"cooldown_secs\":{}}}", cooldown_secs), now],
                );
                // Journal: append-only lifecycle history.
                let _ = conn.execute(
                    "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'acknowledged', ?2, ?3)",
                    rusqlite::params![alarm_id, now, username],
                );
            }

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Alarm shelving ──

    pub async fn shelf_alarm(
        &self,
        id: i64,
        shelved_until: i64,
        shelved_by: &str,
        reason: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let shelved_by = shelved_by.to_string();
        let reason = reason.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "UPDATE alarms SET shelved_until = ?1, shelved_by = ?2, shelved_reason = ?3, state = 'shelved' WHERE id = ?4",
                rusqlite::params![shelved_until, shelved_by, reason, id],
            )
            .context("Shelf alarm failed")?;
            // Create audit entry
            let _ = conn.execute(
                "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'shelved', ?2, ?3, ?4)",
                rusqlite::params![id, shelved_by, format!("{{\"reason\":\"{}\",\"until\":{}}}", reason.replace('"', "\\\""), shelved_until), now],
            );
            // Journal: append-only lifecycle history.
            let _ = conn.execute(
                "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'shelved', ?2, ?3)",
                rusqlite::params![id, now, shelved_by],
            );
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn unshelf_alarm(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            // Determine new state based on acknowledgement status
            let new_state: String = conn
                .query_row(
                    "SELECT CASE WHEN acknowledged_at IS NOT NULL THEN 'acknowledged' ELSE 'active' END FROM alarms WHERE id = ?1",
                    rusqlite::params![id],
                    |row| row.get(0),
                )
                .unwrap_or_else(|_| "active".to_string());

            conn.execute(
                "UPDATE alarms SET shelved_until = NULL, shelved_by = NULL, shelved_reason = NULL, state = ?1 WHERE id = ?2",
                rusqlite::params![new_state, id],
            )
            .context("Unshelf alarm failed")?;
            // Create audit entry
            let _ = conn.execute(
                "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'unshelved', 'system', '{}', ?2)",
                rusqlite::params![id, now],
            );
            // Journal: append-only lifecycle history.
            let _ = conn.execute(
                "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'unshelved', ?2, NULL)",
                rusqlite::params![id, now],
            );
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Automatically unshelf alarms whose shelved_until timestamp has expired.
    pub async fn auto_unshelf_expired_alarms(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();

            // Find expired shelved alarms
            let expired_ids: Vec<i64> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM alarms WHERE state = 'shelved' AND shelved_until IS NOT NULL AND shelved_until <= ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![now], |row| row.get::<_, i64>(0))?;
                rows.collect::<Result<Vec<_>, _>>().unwrap_or_default()
            };

            if expired_ids.is_empty() {
                return Ok(());
            }

            // Unshelf each expired alarm
            for id in &expired_ids {
                let new_state: String = conn
                    .query_row(
                        "SELECT CASE WHEN acknowledged_at IS NOT NULL THEN 'acknowledged' ELSE 'active' END FROM alarms WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .unwrap_or_else(|_| "active".to_string());

                conn.execute(
                    "UPDATE alarms SET shelved_until = NULL, shelved_by = NULL, shelved_reason = NULL, state = ?1 WHERE id = ?2",
                    rusqlite::params![new_state, id],
                )
                .context("Auto-unshelf alarm failed")?;

                let _ = conn.execute(
                    "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, 'auto_unshelved', 'system', '{}', ?2)",
                    rusqlite::params![id, now],
                );

                // Journal: append-only lifecycle history.
                let _ = conn.execute(
                    "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, 'unshelved', ?2, NULL)",
                    rusqlite::params![id, now],
                );
            }

            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Alarm journal (ISA-18.2, append-only) ──

    /// Append one lifecycle event to the alarm journal. Journal rows are never
    /// updated or deleted; the table only grows.
    pub async fn append_alarm_journal(
        &self,
        alarm_id: i64,
        event: &str,
        actor: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let event = event.to_string();
        let actor = actor.map(|a| a.to_string());
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO alarm_journal (alarm_id, event, at, actor) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![alarm_id, event, now, actor],
            )
            .context("Insert alarm journal entry failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// List journal entries for one alarm, oldest first.
    pub async fn list_alarm_journal(
        &self,
        alarm_id: i64,
        limit: u32,
    ) -> Result<Vec<AlarmJournalEntry>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, alarm_id, event, at, actor FROM alarm_journal
                     WHERE alarm_id = ?1
                     ORDER BY at ASC, id ASC LIMIT ?2",
                )
                .context("Prepare journal query failed")?;
            let rows = stmt
                .query_map(rusqlite::params![alarm_id, limit], |row| {
                    Ok(AlarmJournalEntry {
                        id: row.get(0)?,
                        alarm_id: row.get(1)?,
                        event: row.get(2)?,
                        at: row.get(3)?,
                        actor: row.get(4)?,
                    })
                })
                .context("Query journal failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Alarm audit ──

    pub async fn create_alarm_audit_entry(
        &self,
        alarm_id: i64,
        action: &str,
        performed_by: &str,
        details: &str,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let action = action.to_string();
        let performed_by = performed_by.to_string();
        let details = details.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            conn.execute(
                "INSERT INTO alarm_audit_log (alarm_id, action, performed_by, details, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![alarm_id, action, performed_by, details, now],
            )
            .context("Insert alarm audit entry failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_alarm_audit(
        &self,
        alarm_id: Option<i64>,
        from: Option<i64>,
        to: Option<i64>,
        limit: u32,
    ) -> Result<Vec<AlarmAuditEntry>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT id, alarm_id, action, performed_by, details, created_at FROM alarm_audit_log WHERE 1=1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(aid) = alarm_id {
                sql.push_str(" AND alarm_id = ?");
                params.push(Box::new(aid));
            }
            if let Some(f) = from {
                sql.push_str(" AND created_at >= ?");
                params.push(Box::new(f));
            }
            if let Some(t) = to {
                sql.push_str(" AND created_at <= ?");
                params.push(Box::new(t));
            }
            sql.push_str(" ORDER BY created_at DESC LIMIT ?");
            params.push(Box::new(limit));

            let mut stmt = conn.prepare(&sql).context("Prepare audit query failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(AlarmAuditEntry {
                        id: row.get(0)?,
                        alarm_id: row.get(1)?,
                        action: row.get(2)?,
                        performed_by: row.get(3)?,
                        details: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .context("Query audit failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    // ── Alarm summary & analytics ──

    pub async fn get_alarm_summary(&self) -> Result<AlarmSummary> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let twenty_four_hours_ago = now - 86400;

            let active_critical: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms WHERE state = 'active' AND acknowledged_at IS NULL AND priority = 'critical'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let active_warning: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms WHERE state = 'active' AND acknowledged_at IS NULL AND priority = 'warning'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let active_info: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms WHERE state = 'active' AND acknowledged_at IS NULL AND priority = 'info'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let shelved: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms WHERE state = 'shelved'",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let recent_24h: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM alarms WHERE triggered_at >= ?1",
                    rusqlite::params![twenty_four_hours_ago],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            let avg_ack_time_secs: f64 = conn
                .query_row(
                    "SELECT AVG(acknowledged_at - triggered_at) FROM alarms WHERE acknowledged_at IS NOT NULL AND triggered_at >= ?1",
                    rusqlite::params![twenty_four_hours_ago],
                    |row| row.get::<_, Option<f64>>(0),
                )
                .ok()
                .flatten()
                .unwrap_or(0.0);

            Ok(AlarmSummary {
                active_critical,
                active_warning,
                active_info,
                shelved,
                recent_24h,
                avg_ack_time_secs,
            })
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_alarm_analytics(
        &self,
        from: Option<i64>,
        to: Option<i64>,
    ) -> Result<Vec<AlarmAnalyticsEntry>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut sql = String::from(
                "SELECT source_key, COALESCE(source_name, source_key) as source_name,
                        COUNT(*) as trigger_count,
                        COALESCE(AVG(CASE WHEN acknowledged_at IS NOT NULL THEN acknowledged_at - triggered_at END), 0) as avg_time_to_ack_secs,
                        MAX(triggered_at) as last_triggered_at
                 FROM alarms WHERE 1=1",
            );
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            if let Some(f) = from {
                sql.push_str(" AND triggered_at >= ?");
                params.push(Box::new(f));
            }
            if let Some(t) = to {
                sql.push_str(" AND triggered_at <= ?");
                params.push(Box::new(t));
            }
            sql.push_str(" GROUP BY source_key, source_name ORDER BY trigger_count DESC");

            let mut stmt = conn.prepare(&sql).context("Prepare analytics query failed")?;
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(AlarmAnalyticsEntry {
                        source_key: row.get(0)?,
                        source_name: row.get(1)?,
                        trigger_count: row.get(2)?,
                        avg_time_to_ack_secs: row.get(3)?,
                        last_triggered_at: row.get(4)?,
                    })
                })
                .context("Query analytics failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }
}
