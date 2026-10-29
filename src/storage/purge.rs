//! Row retention for the tables that grow with plant events, and giving the
//! freed pages back to the filesystem.
//!
//! Every delete runs in bounded batches, each its own short transaction on the
//! shared connection, with a pause in between: a first purge of years of rows
//! must not hold the connection (and with it the pollers and the API) for
//! longer than one batch takes.

use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use super::Storage;

/// Pause between batches, so writers queued on the connection get a turn.
const BATCH_PAUSE: Duration = Duration::from_millis(25);

/// `PRAGMA auto_vacuum` value of a database that can shrink incrementally.
const AUTO_VACUUM_INCREMENTAL: i64 = 2;

/// A group of tables the retention worker trims. The cutoff passed to
/// [`Storage::purge`] is a unix timestamp; rows older than it go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurgeKind {
    /// Expired and revoked refresh tokens (the cutoff is "now").
    RefreshTokens,
    /// Cleared alarms and their journal. An alarm that is active, acknowledged
    /// but not cleared, or shelved is never removed. The alarm's audit trail is
    /// a separate table and stays (see [`PurgeKind::AlarmAuditLog`]).
    ClearedAlarms,
    /// Downtime events that have ended.
    DowntimeEvents,
    /// The security event log.
    SecurityLog,
    /// Flow run history.
    FlowRuns,
    /// Script execution history.
    ScriptExecutions,
    /// The general audit trail.
    AuditLog,
    /// The alarm audit trail.
    AlarmAuditLog,
    /// Commands that reached a final state, with their audit rows.
    FinishedCommands,
}

impl PurgeKind {
    /// Delete one batch of at most `limit` rows; returns how many primary rows
    /// went. Dependent rows are removed explicitly, in the same transaction,
    /// so the result does not depend on whether foreign keys are enforced.
    fn delete_batch(self, conn: &Connection, cutoff: i64, limit: i64) -> Result<usize> {
        let simple = |sql: &str| -> Result<usize> {
            conn.execute(sql, params![cutoff, limit])
                .with_context(|| format!("Purge failed: {sql}"))
        };
        match self {
            Self::RefreshTokens => simple(
                "DELETE FROM refresh_tokens WHERE id IN (
                     SELECT id FROM refresh_tokens
                     WHERE expires_at < ?1 OR revoked_at IS NOT NULL
                     LIMIT ?2)",
            ),
            Self::DowntimeEvents => simple(
                "DELETE FROM downtime_events WHERE id IN (
                     SELECT id FROM downtime_events
                     WHERE started_at < ?1 AND ended_at IS NOT NULL AND ended_at < ?1
                     LIMIT ?2)",
            ),
            Self::SecurityLog => simple(
                "DELETE FROM security_log WHERE id IN (
                     SELECT id FROM security_log WHERE created_at < ?1 LIMIT ?2)",
            ),
            Self::FlowRuns => simple(
                "DELETE FROM flow_runs WHERE id IN (
                     SELECT id FROM flow_runs WHERE started_at < ?1 LIMIT ?2)",
            ),
            Self::ScriptExecutions => simple(
                "DELETE FROM script_executions WHERE id IN (
                     SELECT id FROM script_executions WHERE created_at < ?1 LIMIT ?2)",
            ),
            Self::AuditLog => simple(
                "DELETE FROM audit_log WHERE id IN (
                     SELECT id FROM audit_log WHERE created_at < ?1 LIMIT ?2)",
            ),
            Self::AlarmAuditLog => simple(
                "DELETE FROM alarm_audit_log WHERE id IN (
                     SELECT id FROM alarm_audit_log WHERE created_at < ?1 LIMIT ?2)",
            ),
            Self::ClearedAlarms => {
                let tx = conn.unchecked_transaction().context("Begin purge failed")?;
                const EXPIRED: &str = "SELECT id FROM alarms
                     WHERE triggered_at < ?1
                       AND resolved_at IS NOT NULL AND resolved_at < ?1
                       AND COALESCE(state, 'active') <> 'shelved'
                     ORDER BY id LIMIT ?2";
                tx.execute(
                    &format!("DELETE FROM alarm_journal WHERE alarm_id IN ({EXPIRED})"),
                    params![cutoff, limit],
                )
                .context("Purge alarm journal failed")?;
                let n = tx
                    .execute(
                        &format!("DELETE FROM alarms WHERE id IN ({EXPIRED})"),
                        params![cutoff, limit],
                    )
                    .context("Purge alarms failed")?;
                tx.commit().context("Commit purge failed")?;
                Ok(n)
            }
            Self::FinishedCommands => {
                let tx = conn.unchecked_transaction().context("Begin purge failed")?;
                const FINISHED: &str = "SELECT id FROM commands
                     WHERE created_at < ?1 AND resolved_at IS NOT NULL
                     ORDER BY id LIMIT ?2";
                tx.execute(
                    &format!("DELETE FROM command_audit_log WHERE command_id IN ({FINISHED})"),
                    params![cutoff, limit],
                )
                .context("Purge command audit failed")?;
                let n = tx
                    .execute(
                        &format!("DELETE FROM commands WHERE id IN ({FINISHED})"),
                        params![cutoff, limit],
                    )
                    .context("Purge commands failed")?;
                tx.commit().context("Commit purge failed")?;
                Ok(n)
            }
        }
    }
}

/// What an incremental vacuum did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VacuumReport {
    /// False for a database created before `auto_vacuum=INCREMENTAL`: its freed
    /// pages are reused by new rows but the file never shrinks until
    /// `voltrus db vacuum` rebuilds it.
    pub supported: bool,
    /// Pages returned to the filesystem.
    pub freed_pages: u64,
    /// Free pages still in the file afterwards.
    pub free_pages_left: u64,
}

/// File sizes around a full `VACUUM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullVacuumReport {
    pub before_bytes: u64,
    pub after_bytes: u64,
}

/// Size of the database file plus its WAL.
fn db_bytes(path: &str) -> u64 {
    ["", "-wal"]
        .iter()
        .map(|suffix| {
            std::fs::metadata(format!("{path}{suffix}"))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum()
}

fn free_pages(conn: &Connection) -> Result<u64> {
    let pages: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .context("Reading freelist_count failed")?;
    Ok(pages.max(0) as u64)
}

impl Storage {
    /// Delete the rows of `kind` older than `cutoff` in batches of
    /// `batch_rows`, returning how many were removed.
    pub async fn purge(&self, kind: PurgeKind, cutoff: i64, batch_rows: u32) -> Result<u64> {
        let limit = i64::from(batch_rows.max(1));
        let mut total = 0u64;
        loop {
            let removed = self
                .with_conn(move |conn| kind.delete_batch(conn, cutoff, limit))
                .await?;
            total += removed as u64;
            if (removed as i64) < limit {
                return Ok(total);
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }

    /// Return free pages to the filesystem, `step_pages` at a time, until none
    /// are left. Each step is its own short transaction.
    pub async fn incremental_vacuum(&self, step_pages: u32) -> Result<VacuumReport> {
        let step = step_pages.max(1);
        let mode: i64 = self
            .with_conn(|conn| Ok(conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?))
            .await?;
        if mode != AUTO_VACUUM_INCREMENTAL {
            let free_pages_left = self.with_conn(|conn| free_pages(conn)).await?;
            return Ok(VacuumReport {
                supported: false,
                freed_pages: 0,
                free_pages_left,
            });
        }

        let mut freed = 0u64;
        loop {
            let (before, after) = self
                .with_conn(move |conn| {
                    let before = free_pages(conn)?;
                    if before > 0 {
                        conn.execute_batch(&format!("PRAGMA incremental_vacuum({step});"))
                            .context("incremental_vacuum failed")?;
                    }
                    Ok((before, free_pages(conn)?))
                })
                .await?;
            freed += before.saturating_sub(after);
            if after == 0 || after >= before {
                return Ok(VacuumReport {
                    supported: true,
                    freed_pages: freed,
                    free_pages_left: after,
                });
            }
            tokio::time::sleep(BATCH_PAUSE).await;
        }
    }

    /// Rebuild the whole database file (`VACUUM`) and switch it to
    /// `auto_vacuum=INCREMENTAL`. It needs free disk space equal to the
    /// database and holds the connection for the whole rebuild: run it with the
    /// server stopped (`voltrus db vacuum`), never from the retention worker.
    pub async fn vacuum_full(&self) -> Result<FullVacuumReport> {
        let before_bytes = db_bytes(&self.db_path);
        self.with_conn(|conn| {
            // Takes effect at the VACUUM that follows.
            conn.execute_batch("PRAGMA auto_vacuum=INCREMENTAL;")?;
            conn.execute_batch("VACUUM;").context("VACUUM failed")?;
            // The rebuilt pages went through the WAL; fold them in and shrink it.
            conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .context("WAL checkpoint failed")?;
            Ok(())
        })
        .await?;
        Ok(FullVacuumReport {
            before_bytes,
            after_bytes: db_bytes(&self.db_path),
        })
    }
}
