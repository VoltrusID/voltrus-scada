use super::*;
use crate::storage::PurgeKind;

const DAY: i64 = 86_400;

fn now() -> i64 {
    crate::timeutil::now_secs()
}

async fn count(storage: &Storage, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    storage
        .with_conn(move |conn| Ok(conn.query_row(&sql, [], |r| r.get(0))?))
        .await
        .unwrap()
}

/// An alarm in the given state, its timestamps moved `age_days` into the past.
/// `resolved`: cleared (resolved_at set); `ack`: acknowledged; `shelved`: shelved.
async fn aged_alarm(
    storage: &Storage,
    source: &str,
    age_days: i64,
    resolved: bool,
    ack: bool,
    shelved: bool,
) -> i64 {
    let id = storage
        .create_alarm_with_type(source, "warning", 80.0, 70.0, "High", 0.0, "high")
        .await
        .unwrap();
    if ack {
        storage.acknowledge_alarm(id, "admin", 0).await.unwrap();
    }
    if shelved {
        storage
            .shelf_alarm(id, now() + DAY, "admin", "maintenance")
            .await
            .unwrap();
    }
    if resolved {
        storage
            .resolve_active_alarms(source, Some(10.0))
            .await
            .unwrap();
    }
    let then = now() - age_days * DAY;
    storage
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE alarms SET triggered_at = ?1,
                     resolved_at = CASE WHEN resolved_at IS NULL THEN NULL ELSE ?1 END
                 WHERE id = ?2",
                rusqlite::params![then, id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    id
}

async fn alarm_ids(storage: &Storage) -> Vec<i64> {
    storage
        .with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT id FROM alarms ORDER BY id")?;
            let ids = stmt
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<i64>, _>>()?;
            Ok(ids)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn expired_and_revoked_refresh_tokens_go_live_ones_stay() {
    let storage = Storage::new_in_memory().unwrap();
    let t = now();
    storage
        .create_refresh_token("u", "expired", t - 10, t - 1000)
        .await
        .unwrap();
    storage
        .create_refresh_token("u", "revoked", t + 1000, t - 500)
        .await
        .unwrap();
    storage.revoke_refresh_token("revoked").await.unwrap();
    storage
        .create_refresh_token("u", "live", t + 1000, t)
        .await
        .unwrap();

    let removed = storage
        .purge(PurgeKind::RefreshTokens, t, 100)
        .await
        .unwrap();

    assert_eq!(removed, 2);
    assert!(
        storage
            .get_refresh_token_by_hash("live")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .get_refresh_token_by_hash("expired")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_refresh_token_by_hash("revoked")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn only_cleared_old_alarms_are_purged_and_their_journal_with_them() {
    let storage = Storage::new_in_memory().unwrap();
    let old_cleared = aged_alarm(&storage, "a", 800, true, false, false).await;
    let old_cleared_acked = aged_alarm(&storage, "b", 800, true, true, false).await;
    let recent_cleared = aged_alarm(&storage, "c", 10, true, false, false).await;
    let old_active = aged_alarm(&storage, "d", 800, false, false, false).await;
    let old_acked_not_cleared = aged_alarm(&storage, "e", 800, false, true, false).await;
    let old_shelved = aged_alarm(&storage, "f", 800, false, false, true).await;

    let removed = storage
        .purge(PurgeKind::ClearedAlarms, now() - 730 * DAY, 100)
        .await
        .unwrap();

    assert_eq!(removed, 2);
    assert_eq!(
        alarm_ids(&storage).await,
        vec![
            recent_cleared,
            old_active,
            old_acked_not_cleared,
            old_shelved
        ],
        "active, acknowledged-but-not-cleared, shelved and recent alarms survive"
    );
    for gone in [old_cleared, old_cleared_acked] {
        assert!(
            storage
                .list_alarm_journal(gone, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
    for kept in [
        recent_cleared,
        old_active,
        old_acked_not_cleared,
        old_shelved,
    ] {
        assert!(
            !storage
                .list_alarm_journal(kept, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn an_unresolved_alarm_is_never_purged_however_old() {
    let storage = Storage::new_in_memory().unwrap();
    aged_alarm(&storage, "a", 5000, false, false, false).await;

    let removed = storage
        .purge(PurgeKind::ClearedAlarms, now() + 10 * DAY, 100)
        .await
        .unwrap();

    assert_eq!(removed, 0);
    assert_eq!(count(&storage, "alarms").await, 1);
}

#[tokio::test]
async fn the_alarm_audit_trail_outlives_a_purged_alarm_with_foreign_keys_on() {
    let storage = Storage::new_in_memory().unwrap();
    let id = aged_alarm(&storage, "a", 800, true, true, false).await;
    let audit_rows = count(&storage, "alarm_audit_log").await;
    assert!(audit_rows >= 3, "created, acknowledged, auto_resolved");

    storage
        .with_conn(|conn| Ok(conn.execute_batch("PRAGMA foreign_keys = ON;")?))
        .await
        .unwrap();
    storage
        .purge(PurgeKind::ClearedAlarms, now() - 730 * DAY, 100)
        .await
        .unwrap();

    assert_eq!(count(&storage, "alarms").await, 0);
    assert_eq!(
        count(&storage, "alarm_audit_log").await,
        audit_rows,
        "audit rows of alarm {id} must not cascade away"
    );
}

#[tokio::test]
async fn a_database_with_the_old_audit_foreign_key_is_rebuilt_keeping_its_rows() {
    let storage = Storage::new_in_memory().unwrap();
    storage
        .with_conn(|conn| {
            conn.execute_batch(
                "DROP TABLE alarm_audit_log;
                 CREATE TABLE alarm_audit_log (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     alarm_id INTEGER NOT NULL REFERENCES alarms(id) ON DELETE CASCADE,
                     action TEXT NOT NULL,
                     performed_by TEXT NOT NULL,
                     details TEXT NOT NULL DEFAULT '',
                     created_at INTEGER NOT NULL
                 );
                 INSERT INTO alarms (id, source_key, severity, value, threshold, message, triggered_at)
                     VALUES (7, 'a', 'warning', 1, 1, 'm', 1);
                 INSERT INTO alarm_audit_log (alarm_id, action, performed_by, created_at)
                     VALUES (7, 'created', 'system', 1);",
            )?;
            Ok(())
        })
        .await
        .unwrap();

    storage
        .with_conn(|conn| crate::storage::schema::init_schema(conn))
        .await
        .unwrap();

    let (fks, rows, indexes): (i64, i64, i64) = storage
        .with_conn(|conn| {
            Ok((
                conn.query_row(
                    "SELECT COUNT(*) FROM pragma_foreign_key_list('alarm_audit_log')",
                    [],
                    |r| r.get(0),
                )?,
                conn.query_row("SELECT COUNT(*) FROM alarm_audit_log", [], |r| r.get(0))?,
                conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index'
                       AND tbl_name = 'alarm_audit_log' AND name LIKE 'idx_alarm_audit%'",
                    [],
                    |r| r.get(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(fks, 0);
    assert_eq!(rows, 1);
    assert_eq!(indexes, 3, "alarm/time, action/time and created_at");
}

#[tokio::test]
async fn ended_old_downtime_goes_open_and_recent_stay() {
    let storage = Storage::new_in_memory().unwrap();
    let t = now();
    storage
        .with_conn(move |conn| {
            for (started, ended) in [
                (t - 900 * DAY, Some(t - 899 * DAY)), // old, ended: goes
                (t - 900 * DAY, None),                // old, still open: stays
                (t - 5 * DAY, Some(t - 4 * DAY)),     // recent: stays
            ] {
                conn.execute(
                    "INSERT INTO downtime_events (device_id, state, started_at, ended_at, created_at)
                     VALUES ('d', 'stopped', ?1, ?2, ?1)",
                    rusqlite::params![started, ended],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

    let removed = storage
        .purge(PurgeKind::DowntimeEvents, t - 730 * DAY, 100)
        .await
        .unwrap();

    assert_eq!(removed, 1);
    assert_eq!(count(&storage, "downtime_events").await, 2);
}

#[tokio::test]
async fn finished_commands_go_with_their_audit_pending_ones_stay() {
    let storage = Storage::new_in_memory().unwrap();
    let t = now();
    storage
        .with_conn(move |conn| {
            for (id, status, created, resolved) in [
                ("old_acked", "acked", t - 900 * DAY, Some(t - 900 * DAY)),
                ("old_failed", "failed", t - 900 * DAY, Some(t - 900 * DAY)),
                ("old_sent", "sent", t - 900 * DAY, None),
                ("old_queued", "queued", t - 900 * DAY, None),
                ("new_acked", "acked", t - DAY, Some(t - DAY)),
            ] {
                conn.execute(
                    "INSERT INTO commands (id, data_source_id, device_id, protocol, value, status,
                         policy, created_at, resolved_at, created_by)
                     VALUES (?1, 1, 1, 'modbus', '1', ?2, 'immediate', ?3, ?4, 'op')",
                    rusqlite::params![id, status, created, resolved],
                )?;
                conn.execute(
                    "INSERT INTO command_audit_log (command_id, old_status, new_status, changed_by, created_at)
                     VALUES (?1, 'pending', ?2, 'system', ?3)",
                    rusqlite::params![id, status, created],
                )?;
            }
            Ok(())
        })
        .await
        .unwrap();

    let removed = storage
        .purge(PurgeKind::FinishedCommands, t - 730 * DAY, 100)
        .await
        .unwrap();

    assert_eq!(removed, 2);
    assert_eq!(count(&storage, "commands").await, 3);
    assert_eq!(count(&storage, "command_audit_log").await, 3);
}

#[tokio::test]
async fn age_based_tables_purge_across_several_batches() {
    let storage = Storage::new_in_memory().unwrap();
    let t = now();
    storage
        .with_conn(move |conn| {
            let tx = conn.transaction()?;
            for i in 0..2_500i64 {
                let created = if i % 2 == 0 { t - 900 * DAY } else { t - DAY };
                tx.execute(
                    "INSERT INTO security_log (event_type, severity, created_at) VALUES ('login_failed', 'info', ?1)",
                    [created],
                )?;
                tx.execute(
                    "INSERT INTO audit_log (action, username, resource, created_at) VALUES ('x', 'u', 'r', ?1)",
                    [created],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();

    // 1250 old rows in batches of 100: thirteen batches, the last one short.
    let removed = storage
        .purge(PurgeKind::SecurityLog, t - 730 * DAY, 100)
        .await
        .unwrap();
    assert_eq!(removed, 1_250);
    assert_eq!(count(&storage, "security_log").await, 1_250);

    let removed = storage
        .purge(PurgeKind::AuditLog, t - 730 * DAY, 1_250)
        .await
        .unwrap();
    assert_eq!(
        removed, 1_250,
        "an exact multiple of the batch ends cleanly"
    );
    assert_eq!(count(&storage, "audit_log").await, 1_250);
}

#[tokio::test]
async fn a_new_database_file_shrinks_after_a_purge_and_an_old_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let fresh_path = dir.path().join("fresh.db");
    let legacy_path = dir.path().join("legacy.db");

    // A database as older versions created it: no auto_vacuum.
    {
        let conn = rusqlite::Connection::open(&legacy_path).unwrap();
        conn.execute_batch("PRAGMA auto_vacuum = NONE; CREATE TABLE seed (x);")
            .unwrap();
    }

    for (path, expect_shrink) in [(&fresh_path, true), (&legacy_path, false)] {
        let storage = Storage::new(path.to_str().unwrap()).unwrap();
        let mode: i64 = storage
            .with_conn(|conn| Ok(conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(mode, if expect_shrink { 2 } else { 0 }, "{path:?}");

        // 20k rows of ~300 bytes: a few MB.
        storage
            .with_conn(|conn| {
                let tx = conn.transaction()?;
                for i in 0..20_000i64 {
                    tx.execute(
                        "INSERT INTO security_log (event_type, severity, details, created_at)
                         VALUES ('login_failed', 'info', ?1, 1000)",
                        [format!("{i:0>300}")],
                    )?;
                }
                tx.commit()?;
                conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
                Ok(())
            })
            .await
            .unwrap();
        let full = std::fs::metadata(path).unwrap().len();
        assert!(full > 4_000_000, "{full}");

        storage
            .purge(PurgeKind::SecurityLog, now(), 5_000)
            .await
            .unwrap();
        let report = storage.incremental_vacuum(500).await.unwrap();
        storage
            .with_conn(
                |conn| Ok(conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?),
            )
            .await
            .unwrap();
        let after = std::fs::metadata(path).unwrap().len();

        assert_eq!(report.supported, expect_shrink, "{path:?}");
        if expect_shrink {
            assert!(report.freed_pages > 1_000, "{report:?}");
            assert_eq!(report.free_pages_left, 0);
            assert!(after < full / 2, "file {full} -> {after}");
        } else {
            assert_eq!(report.freed_pages, 0);
            assert!(report.free_pages_left > 1_000, "{report:?}");
            assert!(
                after >= full,
                "an old file keeps its size: {full} -> {after}"
            );
        }
    }
}

#[tokio::test]
async fn full_vacuum_switches_an_old_database_to_incremental() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA auto_vacuum = NONE; CREATE TABLE seed (x);")
            .unwrap();
    }
    let storage = Storage::new(path.to_str().unwrap()).unwrap();
    storage
        .with_conn(|conn| {
            let tx = conn.transaction()?;
            for i in 0..20_000i64 {
                tx.execute(
                    "INSERT INTO security_log (event_type, severity, details, created_at)
                     VALUES ('login_failed', 'info', ?1, 1000)",
                    [format!("{i:0>300}")],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    storage
        .purge(PurgeKind::SecurityLog, now(), 5_000)
        .await
        .unwrap();

    let report = storage.vacuum_full().await.unwrap();

    assert!(report.after_bytes < report.before_bytes / 2, "{report:?}");
    let mode: i64 = storage
        .with_conn(|conn| Ok(conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(mode, 2, "from now on the retention worker can shrink it");
}

#[tokio::test]
async fn the_wal_has_a_size_limit() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path().join("a.db").to_str().unwrap()).unwrap();
    assert_eq!(
        storage.query_pragma("journal_size_limit").await.unwrap(),
        (64 * 1024 * 1024).to_string()
    );
}
