//! The alarm management answers, as the server serializes its models
//! (`src/models/alarm_types.rs`), decode into the console's mirrors.

use voltrus_scada_native::api::{
    Alarm, AlarmAnalyticsEntry, AlarmAuditEntry, AlarmJournalEntry, AlarmSummary,
};

#[test]
fn the_summary_decodes() {
    // `GET /api/v1/alarms/summary`: `AVG(...)` is a float, 0 without acks.
    let summary: AlarmSummary = serde_json::from_str(
        r#"{"active_critical":2,"active_warning":5,"active_info":0,"shelved":1,
            "recent_24h":37,"avg_ack_time_secs":93.5}"#,
    )
    .unwrap();
    assert_eq!(
        summary,
        AlarmSummary {
            active_critical: 2,
            active_warning: 5,
            active_info: 0,
            shelved: 1,
            recent_24h: 37,
            avg_ack_time_secs: 93.5,
        }
    );
    let quiet: AlarmSummary = serde_json::from_str(
        r#"{"active_critical":0,"active_warning":0,"active_info":0,"shelved":0,
            "recent_24h":0,"avg_ack_time_secs":0.0}"#,
    )
    .unwrap();
    assert_eq!(quiet, AlarmSummary::default());
}

#[test]
fn analytics_rows_decode_in_server_order() {
    // `GET /api/v1/alarms/analytics?from=&to=`, most triggered first.
    let rows: Vec<AlarmAnalyticsEntry> = serde_json::from_str(
        r#"[
            {"source_key":"inv1_temp","source_name":"Inverter 1 temperature",
             "trigger_count":14,"avg_time_to_ack_secs":42.0,"last_triggered_at":1761390000},
            {"source_key":"ac_current_l1","source_name":"AC current L1",
             "trigger_count":3,"avg_time_to_ack_secs":0.0,"last_triggered_at":1761380000}
        ]"#,
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].source_key, "inv1_temp");
    assert_eq!(rows[0].trigger_count, 14);
    assert!((rows[0].avg_time_to_ack_secs - 42.0).abs() < f64::EPSILON);
    assert_eq!(rows[1].last_triggered_at, 1_761_380_000);
}

#[test]
fn audit_entries_keep_their_details_text() {
    // `GET /api/v1/alarms/audit`: a shelve's details are JSON in a string.
    let entries: Vec<AlarmAuditEntry> = serde_json::from_str(
        r#"[
            {"id":9,"alarm_id":41,"action":"shelved","performed_by":"admin",
             "details":"{\"reason\":\"Sensor swap\",\"until\":1761401800}","created_at":1761400000},
            {"id":8,"alarm_id":41,"action":"acknowledged","performed_by":"operator",
             "details":"","created_at":1761399000}
        ]"#,
    )
    .unwrap();
    assert_eq!(entries[0].action, "shelved");
    assert_eq!(
        entries[0].details,
        r#"{"reason":"Sensor swap","until":1761401800}"#
    );
    assert_eq!(entries[1].performed_by, "operator");
    assert_eq!(entries[1].created_at, 1_761_399_000);
}

#[test]
fn journal_entries_decode_with_and_without_an_actor() {
    // `GET /api/v1/alarms/{id}/journal`: the system raises, people act.
    let journal: Vec<AlarmJournalEntry> = serde_json::from_str(
        r#"[
            {"id":1,"alarm_id":41,"event":"triggered","at":1761398000,"actor":null},
            {"id":2,"alarm_id":41,"event":"acknowledged","at":1761399000,"actor":"operator"},
            {"id":3,"alarm_id":41,"event":"shelved","at":1761400000,"actor":"admin"}
        ]"#,
    )
    .unwrap();
    assert_eq!(
        journal
            .iter()
            .map(|entry| (entry.event.as_str(), entry.actor.as_deref()))
            .collect::<Vec<_>>(),
        [
            ("triggered", None),
            ("acknowledged", Some("operator")),
            ("shelved", Some("admin")),
        ]
    );
}

#[test]
fn a_shelved_alarm_carries_who_why_and_until() {
    // `GET /api/v1/alarms?status=all` rows, a resolved one and a shelved one.
    let alarms: Vec<Alarm> = serde_json::from_str(
        r#"[
            {"id":41,"source_key":"inv1_temp","source_name":"Inverter 1 temperature",
             "severity":"critical","priority":"critical","value":86.4,"threshold":85.0,
             "message":"Inverter 1 temperature above 85","triggered_at":1761398000,
             "resolved_at":null,"acknowledged_at":1761399000,"acknowledged_by":"operator",
             "cooldown_until":null,"shelved_until":1761401800,"shelved_by":"admin",
             "shelved_reason":"Sensor swap","deadband":0.5,"state":"shelved","alarm_type":"high"},
            {"id":40,"source_key":"grid_freq","source_name":"Grid frequency",
             "severity":"warning","priority":"warning","value":49.1,"threshold":49.5,
             "message":"Grid frequency below 49.5","triggered_at":1761390000,
             "resolved_at":1761390600,"acknowledged_at":null,"acknowledged_by":null,
             "cooldown_until":1761390900,"shelved_until":null,"shelved_by":null,
             "shelved_reason":null,"deadband":0.0,"state":"resolved","alarm_type":"low"}
        ]"#,
    )
    .unwrap();
    assert_eq!(alarms[0].shelved_reason.as_deref(), Some("Sensor swap"));
    assert_eq!(alarms[0].shelved_by.as_deref(), Some("admin"));
    assert_eq!(alarms[0].shelved_until, Some(1_761_401_800));
    assert_eq!(alarms[0].priority, "critical");
    assert_eq!(alarms[1].resolved_at, Some(1_761_390_600));
    assert_eq!(alarms[1].alarm_type, "low");
    assert_eq!(alarms[1].state, "resolved");
}
