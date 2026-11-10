//! Audit entries for the handlers that wrote none, and the audit log's hash chain.

use super::*;
use crate::security::{AuditEvent, AuditLogger};
use http::StatusCode;
use serde_json::json;
use tokio::sync::mpsc::Receiver;

/// An app whose audit entries land in `rx` instead of the database.
async fn audited_app() -> (Router, Storage, String, Receiver<AuditEvent>) {
    let mut state = seeded_admin_app_state().await;
    let (logger, rx) = AuditLogger::new();
    state.audit_logger = logger;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, storage, cookie, rx)
}

fn drain(rx: &mut Receiver<AuditEvent>) -> Vec<AuditEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

async fn data_source(storage: &Storage) -> i64 {
    storage
        .create_data_source(
            "dev1.temp",
            "Temp",
            "",
            "#fff",
            0.0,
            100.0,
            0.0,
            0.0,
            0.0,
            0,
            1,
            "modbus",
            r#"{"address":40001}"#,
            "dev1",
            None,
            "line",
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn operator_writes_are_audited() {
    let (app, storage, cookie, mut rx) = audited_app().await;
    let ds = data_source(&storage).await;
    let screen = "main";

    let calls: Vec<(&str, String, serde_json::Value, &str)> = vec![
        (
            "PUT",
            format!("/api/v1/screens/{screen}/layouts"),
            json!([]),
            "PUT /api/v1/screens/{id}/layouts",
        ),
        (
            "PUT",
            format!("/api/v1/data-sources/{ds}"),
            json!({"name": "Temp 2"}),
            "PUT /api/v1/data-sources/{id}",
        ),
        (
            "POST",
            "/api/v1/config/snapshots".to_string(),
            json!({"name": "before change"}),
            "POST /api/v1/config/snapshots",
        ),
        (
            "PUT",
            "/api/v1/settings/go-live".to_string(),
            json!({}),
            "PUT /api/v1/settings/go-live",
        ),
        (
            "POST",
            "/api/v1/wizard/create".to_string(),
            json!({}),
            "POST /api/v1/wizard/create",
        ),
        (
            "DELETE",
            "/api/v1/widgets/nope/uninstall".to_string(),
            json!(null),
            "DELETE /api/v1/widgets/{id}/uninstall",
        ),
    ];
    for (method, uri, body, action) in calls {
        let body = (!body.is_null()).then_some(body);
        let (status, _) = send_request(&app, method, &uri, None, Some(&cookie), body).await;
        let entries = drain(&mut rx);
        let entry = entries
            .iter()
            .find(|e| e.action == action)
            .unwrap_or_else(|| panic!("no audit entry {action:?} ({status}): {entries:?}"));
        assert_eq!(entry.username, "admin");
        assert_eq!(entry.resource, uri);
        assert_eq!(
            entry.details.as_deref(),
            Some(format!("status={}", status.as_u16()).as_str()),
            "refused attempts are recorded with their status too"
        );
    }

    // Reads are not audited.
    send_request(
        &app,
        "GET",
        "/api/v1/config/snapshots",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn write_policy_change_is_audited_with_the_policy() {
    let (app, storage, cookie, mut rx) = audited_app().await;
    let ds = data_source(&storage).await;
    let (status, _) = send_request(
        &app,
        "PATCH",
        &format!("/api/v1/data-sources/{ds}/write-policy"),
        None,
        Some(&cookie),
        Some(json!({"writable": true, "write_max": 50.0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let entries = drain(&mut rx);
    let entry = entries
        .iter()
        .find(|e| e.action == "update_write_policy")
        .expect("write policy entry");
    assert_eq!(entry.resource, format!("data_source:{ds}"));
    assert!(
        entry
            .details
            .as_deref()
            .unwrap()
            .contains("\"write_max\":50")
    );
}

#[cfg(feature = "scripts")]
#[tokio::test]
async fn script_changes_are_audited() {
    let (app, _storage, cookie, mut rx) = audited_app().await;
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/scripts",
        None,
        Some(&cookie),
        Some(json!({"name": "s1", "source": "1 + 1"})),
    )
    .await;
    let entries = drain(&mut rx);
    assert!(
        entries.iter().any(|e| e.action == "POST /api/v1/scripts"),
        "{status} {body}: {entries:?}"
    );
}

#[cfg(feature = "mqtt-broker")]
#[tokio::test]
async fn mqtt_publish_attempt_is_audited_once_with_the_topic() {
    let mut state = seeded_admin_app_state().await;
    let (logger, mut rx) = AuditLogger::new();
    state.audit_logger = logger;
    state.mqtt_broker_state = Some(crate::mqtt_broker::MqttBrokerState::new());
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    send_request(
        &app,
        "POST",
        "/api/v1/mqtt-broker/publish",
        None,
        Some(&cookie),
        Some(json!({"topic": "plant/valve1/set", "payload": "1"})),
    )
    .await;
    let entries = drain(&mut rx);
    let publishes: Vec<_> = entries
        .iter()
        .filter(|e| e.action.contains("publish"))
        .collect();
    assert_eq!(publishes.len(), 1, "{entries:?}");
    assert_eq!(publishes[0].action, "mqtt_publish");
    assert_eq!(publishes[0].resource, "mqtt_topic:plant/valve1/set");
    assert_eq!(publishes[0].username, "admin");
}

#[tokio::test]
async fn audit_chain_detects_edits_and_deletions() {
    let storage = Storage::new_in_memory().unwrap();
    // A row from before the chain existed.
    storage
        .with_conn(|c| {
            c.execute(
                "INSERT INTO audit_log (action, username, resource, created_at) VALUES ('old', 'u', 'r', 1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    for n in 0..4 {
        storage
            .log_audit_event(&format!("a{n}"), "admin", "res", Some("d"))
            .await
            .unwrap();
    }
    let report = storage.verify_audit_chain().await.unwrap();
    assert!(report.ok, "{report:?}");
    assert_eq!((report.checked, report.unhashed_legacy), (4, 1));

    let ids: Vec<i64> = storage
        .with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT id FROM audit_log WHERE hash IS NOT NULL ORDER BY id")?;
            let ids = stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(ids)
        })
        .await
        .unwrap();

    // Purging the oldest rows (retention) keeps the chain valid.
    let first = ids[0];
    storage
        .with_conn(move |c| {
            c.execute("DELETE FROM audit_log WHERE id <= ?1", [first])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(storage.verify_audit_chain().await.unwrap().ok);

    // Editing a row breaks it at that row.
    let edited = ids[2];
    storage
        .with_conn(move |c| {
            c.execute(
                "UPDATE audit_log SET username = 'someone-else' WHERE id = ?1",
                [edited],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let report = storage.verify_audit_chain().await.unwrap();
    assert!(!report.ok);
    assert_eq!(report.first_bad_id, Some(edited));

    // Deleting a row from the middle breaks it at the next one.
    let storage = Storage::new_in_memory().unwrap();
    for n in 0..3 {
        storage
            .log_audit_event(&format!("a{n}"), "admin", "res", None)
            .await
            .unwrap();
    }
    let report = storage.verify_audit_chain().await.unwrap();
    assert!(report.ok);
    storage
        .with_conn(|c| {
            c.execute("DELETE FROM audit_log WHERE action = 'a1'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let report = storage.verify_audit_chain().await.unwrap();
    assert!(!report.ok);
    assert!(report.first_bad_id.is_some());
}

#[tokio::test]
async fn audit_verify_route_is_admin_only() {
    let (app, storage, cookie, _rx) = audited_app().await;
    storage
        .log_audit_event("x", "admin", "r", None)
        .await
        .unwrap();
    let (status, body) = send_request(
        &app,
        "GET",
        "/api/v1/audit/verify",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], true);
    assert_eq!(body["checked"], 1);
    let (status, _) = send_request(&app, "GET", "/api/v1/audit/verify", None, None, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
