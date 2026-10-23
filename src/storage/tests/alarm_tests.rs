use super::*;

#[tokio::test]
async fn test_create_and_list_alarm() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "critical", 100.0, 90.0, "Over threshold", 5.0)
        .await
        .unwrap();
    assert!(id > 0);

    let alarms = storage.list_alarms(Some("active"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].source_key, "tag1");
    assert_eq!(alarms[0].severity, "critical");
    assert_eq!(alarms[0].deadband, 5.0);
    assert_eq!(alarms[0].state, "active");
}

#[tokio::test]
async fn test_create_alarm_with_info_severity() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "info", 50.0, 0.0, "Informational", 0.0)
        .await
        .unwrap();
    assert!(id > 0);

    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].severity, "info");
    assert_eq!(alarms[0].priority, "info");
}

#[tokio::test]
async fn test_acknowledge_alarm() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 2.0)
        .await
        .unwrap();
    storage.acknowledge_alarm(id, "admin", 300).await.unwrap();

    let alarms = storage.list_alarms(Some("acknowledged"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert_eq!(alarms[0].acknowledged_by, Some("admin".to_string()));
    assert_eq!(alarms[0].state, "acknowledged");
    assert!(alarms[0].cooldown_until.is_some());
}

#[tokio::test]
async fn test_shelf_and_unshelf_alarm() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    let now = crate::timeutil::now_secs();
    let shelved_until = now + 3600;
    storage
        .shelf_alarm(id, shelved_until, "admin", "Maintenance window")
        .await
        .unwrap();

    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms[0].state, "shelved");
    assert_eq!(alarms[0].shelved_by, Some("admin".to_string()));
    assert_eq!(
        alarms[0].shelved_reason,
        Some("Maintenance window".to_string())
    );

    storage.unshelf_alarm(id).await.unwrap();
    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms[0].state, "active");
    assert!(alarms[0].shelved_until.is_none());
}

#[tokio::test]
async fn test_auto_unshelf_expired_alarms() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    let now = crate::timeutil::now_secs();
    let shelved_until = now - 10; // already expired
    storage
        .shelf_alarm(id, shelved_until, "admin", "Expired")
        .await
        .unwrap();

    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms[0].state, "shelved");

    storage.auto_unshelf_expired_alarms().await.unwrap();

    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms[0].state, "active");
    assert!(alarms[0].shelved_until.is_none());
}

#[tokio::test]
async fn test_resolve_active_alarms_with_deadband() {
    let storage = Storage::new_in_memory().unwrap();

    // Create alarm with deadband of 5.0 (threshold 100)
    let _id = storage
        .create_alarm("tag1", "critical", 110.0, 100.0, "Over threshold", 5.0)
        .await
        .unwrap();

    // Value is 96: 96 < 100 - 5 = 95? No, so alarm should NOT resolve
    storage
        .resolve_active_alarms("tag1", Some(96.0))
        .await
        .unwrap();
    let alarms = storage.list_alarms(Some("active"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert!(alarms[0].resolved_at.is_none());

    // Value is 94: 94 < 100 - 5 = 95? Yes, so alarm should resolve
    storage
        .resolve_active_alarms("tag1", Some(94.0))
        .await
        .unwrap();
    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert!(alarms[0].resolved_at.is_some());
    assert_eq!(alarms[0].state, "resolved");
}

#[tokio::test]
async fn test_low_alarm_resolves_on_rise_not_drop() {
    let storage = Storage::new_in_memory().unwrap();

    // Low alarm: threshold 10, deadband 2. Fires when value <= 10.
    let _id = storage
        .create_alarm_with_type(
            "low_tag",
            "critical",
            8.0,
            10.0,
            "Below low limit",
            2.0,
            "low",
        )
        .await
        .unwrap();

    // Dropping further must NOT clear a low alarm. The pre-fix high-inequality
    // resolve logic would have wrongly resolved it here (5 < 10 - 2).
    storage
        .resolve_active_alarms("low_tag", Some(5.0))
        .await
        .unwrap();
    let alarms = storage.list_alarms(Some("active"), 50).await.unwrap();
    assert_eq!(
        alarms.len(),
        1,
        "low alarm must not clear when value drops further"
    );
    assert!(alarms[0].resolved_at.is_none());

    // Inside the hysteresis band (10 < 11 <= 12) must not clear yet.
    storage
        .resolve_active_alarms("low_tag", Some(11.0))
        .await
        .unwrap();
    assert!(
        storage.list_alarms(Some("active"), 50).await.unwrap()[0]
            .resolved_at
            .is_none(),
        "low alarm stays active inside the deadband band"
    );

    // Rising above threshold + deadband (13 > 10 + 2) clears it.
    storage
        .resolve_active_alarms("low_tag", Some(13.0))
        .await
        .unwrap();
    let alarms = storage.list_alarms(Some("all"), 50).await.unwrap();
    assert_eq!(alarms.len(), 1);
    assert!(alarms[0].resolved_at.is_some());
    assert_eq!(alarms[0].state, "resolved");
    assert_eq!(alarms[0].alarm_type, "low");
}

#[tokio::test]
async fn test_alarm_summary() {
    let storage = Storage::new_in_memory().unwrap();

    storage
        .create_alarm("tag1", "critical", 100.0, 90.0, "Critical", 0.0)
        .await
        .unwrap();
    storage
        .create_alarm("tag2", "warning", 80.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    storage
        .create_alarm("tag3", "info", 50.0, 0.0, "Info", 0.0)
        .await
        .unwrap();

    let summary = storage.get_alarm_summary().await.unwrap();
    assert_eq!(summary.active_critical, 1);
    assert_eq!(summary.active_warning, 1);
    assert_eq!(summary.active_info, 1);
}

#[tokio::test]
async fn test_alarm_audit_log() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    storage.acknowledge_alarm(id, "admin", 0).await.unwrap();

    let audit = storage
        .list_alarm_audit(Some(id), None, None, 50)
        .await
        .unwrap();
    assert!(!audit.is_empty());
    assert!(audit.iter().any(|e| e.action == "created"));
    assert!(audit.iter().any(|e| e.action == "acknowledged"));
}

#[tokio::test]
async fn test_alarm_journal_records_every_transition() {
    let storage = Storage::new_in_memory().unwrap();

    // triggered (from create) → acknowledged → shelved → unshelved → resolved
    let id = storage
        .create_alarm_with_type("tag1", "warning", 75.0, 70.0, "Warning", 0.0, "high")
        .await
        .unwrap();
    storage.acknowledge_alarm(id, "admin", 300).await.unwrap();
    storage
        .shelf_alarm(id, crate::timeutil::now_secs() + 60, "admin", "Maintenance")
        .await
        .unwrap();
    storage.unshelf_alarm(id).await.unwrap();
    storage
        .resolve_active_alarms("tag1", Some(10.0))
        .await
        .unwrap();

    let journal = storage.list_alarm_journal(id, 100).await.unwrap();
    let events: Vec<&str> = journal.iter().map(|e| e.event.as_str()).collect();
    assert_eq!(
        events,
        vec![
            "triggered",
            "acknowledged",
            "shelved",
            "unshelved",
            "resolved"
        ]
    );
    // Actor is recorded for operator actions, NULL for system events.
    assert_eq!(journal[1].actor.as_deref(), Some("admin"));
    assert_eq!(journal[2].actor.as_deref(), Some("admin"));
    assert_eq!(journal[0].actor, None);
    assert_eq!(journal[4].actor, None);
    // Timestamps are non-decreasing (append-only, ordered).
    for pair in journal.windows(2) {
        assert!(pair[0].at <= pair[1].at);
    }
}

#[tokio::test]
async fn test_acknowledge_all_writes_journal_rows() {
    let storage = Storage::new_in_memory().unwrap();

    let a = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    let b = storage
        .create_alarm("tag2", "critical", 95.0, 90.0, "Critical", 0.0)
        .await
        .unwrap();
    storage.acknowledge_all("operator", 0).await.unwrap();

    for id in [a, b] {
        let journal = storage.list_alarm_journal(id, 100).await.unwrap();
        assert!(journal.iter().any(|e| e.event == "triggered"));
        assert!(
            journal
                .iter()
                .any(|e| e.event == "acknowledged" && e.actor.as_deref() == Some("operator"))
        );
    }
}

#[tokio::test]
async fn test_auto_unshelf_writes_journal_row() {
    let storage = Storage::new_in_memory().unwrap();

    let id = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Warning", 0.0)
        .await
        .unwrap();
    let now = crate::timeutil::now_secs();
    storage
        .shelf_alarm(id, now - 10, "admin", "Expired")
        .await
        .unwrap();
    storage.auto_unshelf_expired_alarms().await.unwrap();

    let journal = storage.list_alarm_journal(id, 100).await.unwrap();
    assert!(journal.iter().any(|e| e.event == "unshelved"));
}

#[tokio::test]
async fn test_isa_delay_migration_up_and_down() {
    // Up: a fresh in-memory database runs the full migration set, so the
    // delay columns and the journal table must exist.
    let storage = Storage::new_in_memory().unwrap();
    let conn = storage.conn.clone();
    {
        let conn = conn.lock().unwrap();
        let col_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('data_sources')
                 WHERE name IN ('on_delay_secs', 'off_delay_secs')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(col_count, 2, "ISA delay columns must exist after migration");
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'alarm_journal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);

        // Down: the exact rollback statements must be valid SQL against the
        // migrated schema (SQLite >= 3.35 supports DROP COLUMN).
        conn.execute_batch(
            "ALTER TABLE data_sources DROP COLUMN on_delay_secs;
             ALTER TABLE data_sources DROP COLUMN off_delay_secs;
             DROP TABLE IF EXISTS alarm_journal;",
        )
        .unwrap();
        let table_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'alarm_journal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
    }
}
