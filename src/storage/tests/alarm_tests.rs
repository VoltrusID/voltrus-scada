use super::*;
use crate::models::AlarmTransition;

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

/// SCADA-16 S4: the shift report read the newest 500 alarms and filtered them,
/// so a busy shift lost its oldest alarms.
#[tokio::test]
async fn test_list_alarms_between_is_half_open_and_uncapped() {
    let storage = Storage::new_in_memory().unwrap();
    for i in 0..600 {
        storage
            .create_alarm(&format!("tag{i}"), "warning", 1.0, 0.0, "m", 0.0)
            .await
            .unwrap();
    }
    // Alarms 1..=600 get one second each from 1000: ids 1..=600 -> 1000..1599.
    {
        let conn = storage.conn.lock().unwrap();
        conn.execute("UPDATE alarms SET triggered_at = 999 + id", [])
            .unwrap();
    }

    let alarms = storage.list_alarms_between(1000, 1600).await.unwrap();
    assert_eq!(alarms.len(), 600, "no row cap");
    assert_eq!(alarms[0].triggered_at, 1599, "newest first");

    let alarms = storage.list_alarms_between(1000, 1001).await.unwrap();
    assert_eq!(alarms.len(), 1, "the end is exclusive");
    assert_eq!(alarms[0].triggered_at, 1000, "the start is inclusive");

    assert!(
        storage
            .list_alarms_between(2000, 3000)
            .await
            .unwrap()
            .is_empty()
    );
}

/// SCADA-17 S9: the summary counted only the exact priorities `critical`,
/// `warning` and `info`; any other waiting alarm was in no bucket.
#[tokio::test]
async fn test_alarm_summary_counts_every_waiting_alarm() {
    let storage = Storage::new_in_memory().unwrap();
    {
        let conn = storage.conn.lock().unwrap();
        for (key, priority) in [
            ("a", "critical"),
            ("b", "Critical"),
            ("c", "warning"),
            ("d", "high"),
            ("e", "medium"),
            ("f", "info"),
            ("g", "low"),
            ("h", "advisory"),
        ] {
            conn.execute(
                "INSERT INTO alarms (source_key, severity, value, threshold, message, triggered_at, priority, state)
                 VALUES (?1, 'warning', 1, 0, 'm', 100, ?2, 'active')",
                rusqlite::params![key, priority],
            )
            .unwrap();
        }
    }

    let summary = storage.get_alarm_summary().await.unwrap();
    assert_eq!(summary.active_critical, 2);
    assert_eq!(summary.active_warning, 3);
    assert_eq!(summary.active_info, 3);
}

/// SCADA-17 S9: the lifecycle calls report what changed, for the WebSocket push.
#[tokio::test]
async fn test_lifecycle_calls_report_the_alarms_they_changed() {
    let storage = Storage::new_in_memory().unwrap();
    let id = storage
        .create_alarm("tag1", "critical", 100.0, 90.0, "Over", 0.0)
        .await
        .unwrap();
    let change = |state: &str| AlarmTransition {
        alarm_id: id,
        source_key: "tag1".to_string(),
        state: state.to_string(),
    };

    assert_eq!(
        storage.acknowledge_alarm(id, "admin", 0).await.unwrap(),
        Some(change("acknowledged"))
    );
    assert_eq!(
        storage.acknowledge_alarm(999, "admin", 0).await.unwrap(),
        None
    );

    let until = crate::timeutil::now_secs() - 10;
    assert_eq!(
        storage
            .shelf_alarm(id, until, "admin", "Expired")
            .await
            .unwrap(),
        Some(change("shelved"))
    );
    assert_eq!(
        storage.auto_unshelf_expired_alarms().await.unwrap(),
        [change("acknowledged")]
    );
    assert_eq!(
        storage.unshelf_alarm(id).await.unwrap(),
        Some(change("acknowledged"))
    );

    assert_eq!(
        storage.resolve_active_alarms("tag1", None).await.unwrap(),
        [change("resolved")]
    );
    assert!(
        storage
            .resolve_active_alarms("tag1", None)
            .await
            .unwrap()
            .is_empty(),
        "already resolved: nothing changes"
    );

    storage
        .create_alarm("tag2", "info", 1.0, 0.0, "Low", 0.0)
        .await
        .unwrap();
    let all = storage.acknowledge_all("operator", 0).await.unwrap();
    assert_eq!(all.len(), 1, "only tag2 was still unacknowledged");
    assert_eq!(all[0].source_key, "tag2");
    assert_eq!(all[0].state, "acknowledged");
}

/// A device snapshot with one good metric, as the Modbus poller caches it.
fn wtp_snapshot(metric: &str, value: f64) -> crate::models::DeviceSnapshot {
    crate::models::DeviceSnapshot {
        device_id: "wtp".into(),
        status: crate::models::DeviceStatus::Online,
        timestamp: 1,
        metrics: [(metric.to_string(), value)].into_iter().collect(),
        simulated: false,
        protocol: None,
        ts_ms: 1000,
        quality: Default::default(),
        metric_ts_ms: Default::default(),
    }
}

/// One limit-evaluation pass for stale alarms, as the persistence loop runs it.
async fn clear_stale(storage: &Storage, snaps: &[crate::models::DeviceSnapshot]) -> usize {
    let active = storage.list_unresolved_limit_alarms().await.unwrap();
    let sources = storage.list_data_sources(None).await.unwrap();
    let stale = crate::alarm_eval::stale_alarms(
        &active,
        &sources,
        snaps,
        &mut std::collections::HashMap::new(),
        0,
    );
    storage.resolve_stale_alarms(&stale).await.unwrap().len()
}

async fn intake_flow_tag(storage: &Storage, warn: f64, crit: f64) -> i64 {
    storage.create_device("wtp", "WTP", 0).await.unwrap();
    storage
        .create_data_source(
            "intake_flow",
            "intake_flow",
            "",
            "#94a3b8",
            0.0,
            100.0,
            warn,
            crit,
            50.0,
            0,
            1,
            "modbus",
            "{}",
            "wtp",
            None,
            "sparkline",
        )
        .await
        .unwrap()
}

async fn only_reason(storage: &Storage, alarm_id: i64) -> String {
    let audit = storage
        .list_alarm_audit(Some(alarm_id), None, None, 10)
        .await
        .unwrap();
    let resolved: Vec<_> = audit
        .iter()
        .filter(|a| a.action == "auto_resolved")
        .collect();
    assert_eq!(resolved.len(), 1, "{audit:?}");
    let details: serde_json::Value = serde_json::from_str(&resolved[0].details).unwrap();
    details["reason"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn first_start_tags_have_no_alarm_limits() {
    let storage = Storage::new_in_memory().unwrap();
    let device: crate::models::DeviceConfig = serde_json::from_value(serde_json::json!({
        "id": "wtp", "ip": "127.0.0.1",
        "registers": [{"name": "intake_flow", "address": 0}, {"name": "voltage", "address": 2}],
    }))
    .unwrap();
    storage.seed_database(&[device]).await.unwrap();
    let sources = storage.list_data_sources(None).await.unwrap();
    assert_eq!(sources.len(), 2);
    for tag in &sources {
        let limits = crate::alarm_eval::Thresholds::of(tag);
        assert_eq!(limits.condition(1178.0), None, "{}", tag.key);
        assert_eq!(limits.condition(-1178.0), None, "{}", tag.key);
    }
    assert!(
        storage
            .tags_with_old_seeded_limits()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn tags_seeded_with_the_old_default_limits_are_found() {
    let storage = Storage::new_in_memory().unwrap();
    let id = intake_flow_tag(&storage, 80.0, 90.0).await;
    assert_eq!(
        storage
            .tags_with_old_seeded_limits()
            .await
            .unwrap()
            .first_start,
        vec!["intake_flow".to_string()]
    );
    storage
        .update_data_source(id, &serde_json::json!({"name": "Raw Water Intake Flow"}))
        .await
        .unwrap();
    assert!(
        storage
            .tags_with_old_seeded_limits()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn removing_the_limit_clears_an_alarm_raised_on_it() {
    // The wtp-hmi first start: an alarm on the old default limit 90, then the
    // tag is configured with no high limit while the flow stays at 1178.
    let storage = Storage::new_in_memory().unwrap();
    let id = intake_flow_tag(&storage, 80.0, 90.0).await;
    let alarm = storage
        .create_alarm_with_type("intake_flow", "critical", 1178.0, 90.0, "high", 0.0, "high")
        .await
        .unwrap();
    let snaps = [wtp_snapshot("intake_flow", 1178.0)];
    assert_eq!(clear_stale(&storage, &snaps).await, 0, "limit unchanged");

    storage
        .update_data_source(id, &serde_json::json!({"warn_val": 0, "crit_val": 0}))
        .await
        .unwrap();
    // Normal evaluation alone never clears it: 1178 is not below 90.
    assert!(
        storage
            .resolve_active_alarms("intake_flow", Some(1178.0))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(clear_stale(&storage, &snaps).await, 1);
    assert!(
        storage
            .list_alarms(Some("active"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(only_reason(&storage, alarm).await, "limit_removed");
    assert_eq!(clear_stale(&storage, &snaps).await, 0, "resolved once");
}

#[tokio::test]
async fn raising_the_limit_above_the_value_clears_the_alarm() {
    let storage = Storage::new_in_memory().unwrap();
    let id = intake_flow_tag(&storage, 0.0, 90.0).await;
    let alarm = storage
        .create_alarm_with_type("intake_flow", "critical", 1178.0, 90.0, "high", 0.0, "high")
        .await
        .unwrap();
    let snaps = [wtp_snapshot("intake_flow", 1178.0)];
    storage
        .update_data_source(id, &serde_json::json!({"crit_val": 1000}))
        .await
        .unwrap();
    assert_eq!(
        clear_stale(&storage, &snaps).await,
        0,
        "1178 is still over 1000"
    );
    storage
        .update_data_source(id, &serde_json::json!({"crit_val": 3000}))
        .await
        .unwrap();
    assert_eq!(clear_stale(&storage, &snaps).await, 1);
    assert_eq!(only_reason(&storage, alarm).await, "limit_changed");
}

#[tokio::test]
async fn deleting_the_tag_clears_its_alarms_but_not_flow_alarms() {
    let storage = Storage::new_in_memory().unwrap();
    let id = intake_flow_tag(&storage, 80.0, 90.0).await;
    let alarm = storage
        .create_alarm_with_type("intake_flow", "warning", 85.0, 80.0, "high", 0.0, "high")
        .await
        .unwrap();
    // A flow's alarm on the same key, and one stored before flows had their own type.
    let flow = storage
        .create_alarm_with_type("intake_flow", "critical", 0.0, 0.0, "flow", 0.0, "event")
        .await
        .unwrap();
    let legacy_flow = storage
        .create_alarm("intake_flow", "critical", 0.0, 0.0, "old flow", 0.0)
        .await
        .unwrap();
    // The upgrade's migration retypes it.
    rerun_migrations(&storage);
    storage.delete_data_source(id).await.unwrap();
    assert_eq!(clear_stale(&storage, &[]).await, 1);
    assert_eq!(only_reason(&storage, alarm).await, "tag_deleted");
    let active: Vec<i64> = storage
        .list_alarms(Some("active"), 10)
        .await
        .unwrap()
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(active.len(), 2);
    assert!(active.contains(&flow) && active.contains(&legacy_flow));
}

fn rerun_migrations(storage: &Storage) {
    let conn = storage.conn.lock().unwrap();
    crate::storage::schema::init_schema(&conn).unwrap();
}

fn alarm_type_of(storage: &Storage, id: i64) -> String {
    let conn = storage.conn.lock().unwrap();
    conn.query_row("SELECT alarm_type FROM alarms WHERE id = ?1", [id], |r| {
        r.get(0)
    })
    .unwrap()
}

#[tokio::test]
async fn old_flow_alarms_become_events_but_a_real_zero_limit_alarm_stays_high() {
    let storage = Storage::new_in_memory().unwrap();
    let flow = storage
        .create_alarm("pump", "critical", 0.0, 0.0, "Pump tripped (flow)", 0.0)
        .await
        .unwrap();
    // A limit alarm on an enabled high limit of 0, reached at exactly 0.
    let limit = storage
        .create_alarm(
            "pump",
            "warning",
            0.0,
            0.0,
            "Pump reached 0.0 (warning: 0.0)",
            0.0,
        )
        .await
        .unwrap();
    rerun_migrations(&storage);
    assert_eq!(alarm_type_of(&storage, flow), "event");
    assert_eq!(alarm_type_of(&storage, limit), "high");
    let active: Vec<i64> = storage
        .list_unresolved_limit_alarms()
        .await
        .unwrap()
        .iter()
        .map(|a| a.id)
        .collect();
    assert_eq!(active, vec![limit]);
}

#[tokio::test]
async fn unresolved_alarms_have_a_partial_index() {
    let storage = Storage::new_in_memory().unwrap();
    let conn = storage.conn.lock().unwrap();
    let plan: String = conn
        .query_row(
            "EXPLAIN QUERY PLAN SELECT id FROM alarms
             WHERE resolved_at IS NULL AND severity IN ('warning', 'critical')
               AND alarm_type IN ('high', 'low')",
            [],
            |r| r.get(3),
        )
        .unwrap();
    assert!(plan.contains("idx_alarms_unresolved"), "{plan}");
}

#[tokio::test]
async fn an_event_alarm_does_not_hold_back_limit_alarms_on_its_key() {
    let storage = Storage::new_in_memory().unwrap();
    let event = storage
        .create_alarm_with_type(
            "tank",
            "critical",
            0.0,
            0.0,
            "Overfill (flow)",
            0.0,
            "event",
        )
        .await
        .unwrap();
    assert!(
        !storage
            .has_unresolved_alarm("tank", "critical")
            .await
            .unwrap()
    );
    assert!(!storage.has_any_unresolved_alarm("tank").await.unwrap());
    // Acknowledging it with a cooldown starts no cooldown for limit alarms.
    storage.acknowledge_alarm(event, "op", 600).await.unwrap();
    assert!(!storage.is_in_cooldown("tank", "critical").await.unwrap());

    storage
        .create_alarm_with_type("tank", "critical", 99.0, 95.0, "high", 0.0, "high")
        .await
        .unwrap();
    assert!(
        storage
            .has_unresolved_alarm("tank", "critical")
            .await
            .unwrap()
    );
    assert!(storage.has_any_unresolved_alarm("tank").await.unwrap());
}

#[tokio::test]
async fn acknowledging_an_event_alarm_resolves_it() {
    let storage = Storage::new_in_memory().unwrap();
    let event = storage
        .create_alarm_with_type(
            "tank",
            "critical",
            0.0,
            0.0,
            "Overfill (flow)",
            0.0,
            "event",
        )
        .await
        .unwrap();
    let limit = storage
        .create_alarm_with_type("tank", "warning", 90.0, 85.0, "high", 0.0, "high")
        .await
        .unwrap();
    let change = storage.acknowledge_alarm(event, "op", 0).await.unwrap();
    assert_eq!(change.unwrap().state, "resolved");
    let audit = storage
        .list_alarm_audit(Some(event), None, None, 10)
        .await
        .unwrap();
    let resolved = audit.iter().find(|a| a.action == "resolved").unwrap();
    assert_eq!(resolved.performed_by, "op");
    assert!(resolved.details.contains("event_acknowledged"));

    // A limit alarm acknowledged stays raised until its value is back.
    let change = storage.acknowledge_alarm(limit, "op", 0).await.unwrap();
    assert_eq!(change.unwrap().state, "acknowledged");

    // Acknowledge-all resolves the event alarms among them.
    let second = storage
        .create_alarm_with_type(
            "tank",
            "warning",
            0.0,
            0.0,
            "Level switch (flow)",
            0.0,
            "event",
        )
        .await
        .unwrap();
    let all = storage.acknowledge_all("op", 0).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(
        (all[0].alarm_id, all[0].state.as_str()),
        (second, "resolved")
    );
}

/// A tag as the old vertical template instantiator wrote it: orange, named
/// after its key, base in the middle of the range, the template id in
/// source_config.
async fn old_template_tag(
    storage: &Storage,
    key: &str,
    name: &str,
    (min, max): (f64, f64),
    (warn, crit): (f64, f64),
    cfg: &str,
) -> i64 {
    storage
        .create_data_source(
            key,
            name,
            "",
            "#f97316",
            min,
            max,
            warn,
            crit,
            (min + max) / 2.0,
            0,
            1,
            "modbus",
            cfg,
            "vt_t",
            None,
            "gauge",
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn template_tags_with_the_old_limits_are_found() {
    let storage = Storage::new_in_memory().unwrap();
    storage.create_device("vt_t", "T", 0).await.unwrap();
    let cfg = r#"{"template_id":"maritime","data_type":"float32"}"#;
    // Range as limits, untouched.
    old_template_tag(
        &storage,
        "vt_t:GPS_SPD",
        "GPS_SPD",
        (0.0, 40.0),
        (0.0, 40.0),
        cfg,
    )
    .await;
    // A critical "above" alarm replaced the maximum, the minimum (0) is no limit.
    old_template_tag(
        &storage,
        "vt_t:ENG1_TMP",
        "ENG1_TMP",
        (0.0, 120.0),
        (0.0, 95.0),
        cfg,
    )
    .await;
    // A warning "below" alarm (tank low at 15) stored as a high limit; the
    // range maximum is still the critical limit.
    old_template_tag(
        &storage,
        "vt_t:TK-2_LVL",
        "TK-2_LVL",
        (0.0, 100.0),
        (15.0, 100.0),
        cfg,
    )
    .await;
    // Configured since (flag written by the tag editor).
    old_template_tag(
        &storage,
        "vt_t:TK-3_LVL",
        "TK-3_LVL",
        (0.0, 100.0),
        (15.0, 100.0),
        r#"{"template_id":"maritime","warn_enabled":true}"#,
    )
    .await;
    // FEED template: "<signal> — <description>", range minimum above 0.
    old_template_tag(
        &storage,
        "vt_t:PT-101",
        "PT-101 — Discharge pressure",
        (4.0, 20.0),
        (4.0, 20.0),
        r#"{"template_id":"feed_x","feed_template":true,"io_type":"AI"}"#,
    )
    .await;
    // Renamed since: no longer the template's tag as written.
    old_template_tag(
        &storage,
        "vt_t:GPS_HDG",
        "Heading",
        (0.0, 360.0),
        (0.0, 360.0),
        cfg,
    )
    .await;
    let old = storage.tags_with_old_seeded_limits().await.unwrap();
    assert!(old.first_start.is_empty());
    assert_eq!(
        old.template_range,
        vec!["vt_t:GPS_SPD", "vt_t:TK-2_LVL", "vt_t:PT-101"]
    );
    assert_eq!(old.below_as_high, vec!["vt_t:TK-2_LVL"]);
}

#[tokio::test]
async fn fault_bits_with_a_critical_limit_at_1_are_not_flagged() {
    let storage = Storage::new_in_memory().unwrap();
    storage.create_device("vt_t", "T", 0).await.unwrap();
    // The demo's fault bits: orange (the API default), range 0–1, critical
    // at 1, base 0, renamed, set up by scenarios/wtp-hmi/seed.py.
    storage
        .create_data_source(
            "pv_inv1_fault",
            "Inverter 1 Fault",
            "",
            "#f97316",
            0.0,
            1.0,
            0.0,
            1.0,
            0.0,
            0,
            0,
            "modbus",
            "{}",
            "vt_t",
            None,
            "led",
        )
        .await
        .unwrap();
    // Even a template bit left exactly as the old instantiator wrote it:
    // critical at 1 is how a fault bit alarms, so it can't be told apart.
    old_template_tag(
        &storage,
        "vt_t:PMP1_FLT",
        "PMP1_FLT",
        (0.0, 1.0),
        (0.0, 1.0),
        r#"{"template_id":"wastewater_treatment","data_type":"bool"}"#,
    )
    .await;
    // A bool/coil tag with another range.
    old_template_tag(
        &storage,
        "vt_t:SAFE_ESTOP",
        "SAFE_ESTOP",
        (0.0, 2.0),
        (0.0, 2.0),
        r#"{"template_id":"elevator_ski_lift","register_area":"coil"}"#,
    )
    .await;
    // FEED digital input.
    old_template_tag(
        &storage,
        "vt_t:XS-101",
        "XS-101 — Valve open",
        (0.0, 1.0),
        (0.0, 1.0),
        r#"{"template_id":"feed_x","feed_template":true,"io_type":"DI"}"#,
    )
    .await;
    assert!(
        storage
            .tags_with_old_seeded_limits()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn first_start_fixed_limits_need_the_whole_seeded_tag() {
    let storage = Storage::new_in_memory().unwrap();
    storage.create_device("plts_1", "PLTS 1", 0).await.unwrap();
    let id = storage
        .create_data_source(
            "frequency",
            "Frequency",
            "Hz",
            "#22c55e",
            45.0,
            55.0,
            51.0,
            52.0,
            50.0,
            0,
            2,
            "modbus",
            "{}",
            "plts_1",
            None,
            "sparkline",
        )
        .await
        .unwrap();
    let old = storage.tags_with_old_seeded_limits().await.unwrap();
    assert_eq!(old.first_start, vec!["frequency"]);
    // Same limits on a renamed tag: configured on purpose.
    storage
        .update_data_source(id, &serde_json::json!({"name": "Grid Frequency"}))
        .await
        .unwrap();
    assert!(
        storage
            .tags_with_old_seeded_limits()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn renaming_a_tag_keeps_its_active_alarms() {
    let storage = Storage::new_in_memory().unwrap();
    let id = intake_flow_tag(&storage, 0.0, 90.0).await;
    let alarm = storage
        .create_alarm_with_type("intake_flow", "critical", 95.0, 90.0, "high", 0.0, "high")
        .await
        .unwrap();
    let event = storage
        .create_alarm_with_type("intake_flow", "warning", 0.0, 0.0, "flow", 0.0, "event")
        .await
        .unwrap();
    storage
        .update_data_source(id, &serde_json::json!({"key": "wtp.intake_flow"}))
        .await
        .unwrap();
    let key_of = |id: i64| {
        let conn = storage.conn.lock().unwrap();
        conn.query_row("SELECT source_key FROM alarms WHERE id = ?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .unwrap()
    };
    assert_eq!(key_of(alarm), "wtp.intake_flow");
    assert_eq!(key_of(event), "intake_flow", "a flow alarm keeps its key");
    let active = storage.list_unresolved_limit_alarms().await.unwrap();
    let sources = storage.list_data_sources(None).await.unwrap();
    let stale = crate::alarm_eval::stale_alarms(
        &active,
        &sources,
        &[wtp_snapshot("intake_flow", 95.0)],
        &mut std::collections::HashMap::new(),
        0,
    );
    assert!(stale.is_empty(), "{stale:?}");
}
