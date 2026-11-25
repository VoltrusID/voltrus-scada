//! The compliance hooks of `crate::edition::Extension` with a test edition:
//! electronic signatures (428 flow, wrong-password lockout, API keys and MCP
//! refused), password policy and expiry, session idle and absolute limits,
//! and the previous value in the tag-write audit trail.

use std::sync::Mutex;

use super::*;
use crate::edition::{
    ActionContext, ActionOrigin, BoxFuture, Extension, PasswordChangeKind, PasswordCheck,
    SessionPolicy, SignatureRecord, SignatureRequirement, SignedAction,
};
use crate::security::{AuditEvent, AuditLogger};
use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::json;
use tokio::sync::mpsc::Receiver;
use tower::ServiceExt;

const MEANING: &str = "Approved by operator";

/// An edition that wants every signable action signed and keeps what the
/// core hands it.
#[derive(Default)]
struct TestEdition {
    records: Mutex<Vec<SignatureRecord>>,
    asked: Mutex<Vec<(SignedAction, ActionOrigin, String)>>,
    password_changes: Mutex<Vec<(String, PasswordChangeKind)>>,
    session: SessionPolicy,
    /// Let flows write without a signature.
    allow_flows: bool,
    /// Report every password as expired (not only `stale`'s).
    expire_all: std::sync::atomic::AtomicBool,
}

impl Extension for TestEdition {
    fn validate_new_password<'a>(
        &'a self,
        check: &'a PasswordCheck,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if check.new_password.len() < 12 {
                return Err("Password must have at least 12 characters".to_string());
            }
            if check.new_password.contains(&check.username) {
                return Err("Password must not contain the username".to_string());
            }
            Ok(())
        })
    }

    fn password_changed<'a>(
        &'a self,
        username: &'a str,
        password_hash: &'a str,
        kind: PasswordChangeKind,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            assert!(
                password_hash.starts_with("$argon2"),
                "a hash, never the password"
            );
            self.password_changes
                .lock()
                .unwrap()
                .push((username.to_string(), kind));
        })
    }

    fn password_expired<'a>(&'a self, user: &'a crate::models::User) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            user.username == "stale" || self.expire_all.load(std::sync::atomic::Ordering::SeqCst)
        })
    }

    fn session_policy<'a>(
        &'a self,
        _username: &'a str,
        _role: &'a str,
    ) -> BoxFuture<'a, SessionPolicy> {
        Box::pin(async move { self.session })
    }

    fn signature_required<'a>(
        &'a self,
        ctx: &'a ActionContext,
    ) -> BoxFuture<'a, Option<SignatureRequirement>> {
        Box::pin(async move {
            self.asked
                .lock()
                .unwrap()
                .push((ctx.action, ctx.origin, ctx.resource.clone()));
            if ctx.origin == ActionOrigin::Flow && self.allow_flows {
                return None;
            }
            Some(SignatureRequirement {
                meaning: MEANING.to_string(),
            })
        })
    }

    fn record_signature<'a>(
        &'a self,
        record: &'a SignatureRecord,
    ) -> BoxFuture<'a, anyhow::Result<Option<String>>> {
        Box::pin(async move {
            let mut records = self.records.lock().unwrap();
            records.push(record.clone());
            Ok(Some(format!("sig-{}", records.len())))
        })
    }
}

struct Fixture {
    app: Router,
    state: AppState,
    edition: Arc<TestEdition>,
    audit: Receiver<AuditEvent>,
}

async fn fixture(edition: TestEdition) -> Fixture {
    let edition = Arc::new(edition);
    let mut state = seeded_admin_app_state().await;
    state.ext = edition.clone();
    let (logger, audit) = AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state.clone());
    Fixture {
        app,
        state,
        edition,
        audit,
    }
}

fn drain(rx: &mut Receiver<AuditEvent>) -> Vec<AuditEvent> {
    std::iter::from_fn(|| rx.try_recv().ok()).collect()
}

async fn add_user(storage: &Storage, username: &str, password: &str, role: &str) {
    let hash = crate::api::auth::hash_password(password);
    storage
        .create_user(username, &hash, "", role)
        .await
        .unwrap();
}

async fn cookie(app: &Router, username: &str, password: &str) -> String {
    format!(
        "voltrus_token={}",
        login_cookie(app, username, password).await
    )
}

/// A writable tag `dev1.temp` whose live value is 21.5.
async fn writable_tag(state: &AppState) -> i64 {
    let id = state
        .storage
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
        .unwrap();
    state
        .storage
        .update_write_policy(
            id,
            &crate::models::WritePolicyUpdate {
                writable: Some(true),
                write_min: None,
                write_max: None,
                write_max_rate: None,
                write_requires_reason: None,
                write_deadband: None,
            },
        )
        .await
        .unwrap();
    state
        .cache
        .update(
            "dev1",
            std::collections::HashMap::from([("temp".to_string(), 21.5)]),
            false,
        )
        .await;
    id
}

fn signature(password: &str) -> serde_json::Value {
    json!({"password": password, "meaning": MEANING, "reason": "Batch 42 setpoint"})
}

#[tokio::test]
async fn a_write_the_edition_wants_signed_is_answered_428_then_runs_once_signed() {
    let mut f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let uri = format!("/api/v1/tags/{tag}/write");

    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 30.0})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["code"], "signature_required");
    assert_eq!(body["signature_required"]["meaning"], MEANING);
    assert_eq!(body["signature_required"]["action"], "tag_write");
    assert!(
        f.state.storage.count_recent_writes(tag, 60).await.unwrap() == 0,
        "nothing was queued"
    );

    // A signature for another meaning is asked again.
    let (status, _) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 30.0, "signature": {"password": "admin", "meaning": "something else", "reason": "x"}})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);

    // No reason: 400.
    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 30.0, "signature": {"password": "admin", "meaning": MEANING, "reason": "  "}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    drain(&mut f.audit);
    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 30.0, "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    let records = f.edition.records.lock().unwrap().clone();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.username, "admin");
    assert_eq!(record.action, SignedAction::TagWrite);
    assert_eq!(record.origin, ActionOrigin::Session);
    assert_eq!(record.resource, format!("data_source:{tag}"));
    assert_eq!(record.meaning, MEANING);
    assert_eq!(record.reason, "Batch 42 setpoint");
    assert_eq!(record.details["previous"], 21.5);
    assert_eq!(record.details["value"], 30.0);
    assert_eq!(record.details["tag_id"], tag);

    let entries = drain(&mut f.audit);
    let write = entries
        .iter()
        .find(|e| e.action == "tag_write")
        .expect("tag_write audit entry");
    let details = write.details.as_deref().unwrap();
    assert!(details.contains("previous=21.5 value=30"), "{details}");
    assert!(details.contains("signature=sig-1"), "{details}");
    assert!(
        !entries
            .iter()
            .any(|e| e.details.as_deref().is_some_and(|d| d.contains("password"))),
        "the password never reaches the audit log"
    );
    assert!(entries.iter().any(|e| e.action == "signature"));
}

#[tokio::test]
async fn wrong_signature_passwords_lock_signing_and_end_sessions() {
    let f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    add_user(&f.state.storage, "olivia", "olivia-pass-123", "operator").await;
    let olivia = cookie(&f.app, "olivia", "olivia-pass-123").await;
    let uri = format!("/api/v1/tags/{tag}/write");

    for attempt in 1..super::super::signature::MAX_FAILED_ATTEMPTS {
        let (status, body) = send_request(
            &f.app,
            "POST",
            &uri,
            None,
            Some(&olivia),
            Some(json!({"value": 30.0, "signature": signature("wrong")})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "attempt {attempt}: {body}");
        assert_eq!(body["code"], "signature_invalid");
        assert_eq!(body["signature_required"]["meaning"], MEANING);
    }
    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&olivia),
        Some(json!({"value": 30.0, "signature": signature("wrong")})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "account_locked");

    // Locked: even the right password is refused.
    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&olivia),
        Some(json!({"value": 30.0, "signature": signature("olivia-pass-123")})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "account_locked");
    // Signing in again works (sign-in is counted per address, apart from
    // signatures), but signing stays locked.
    let olivia = cookie(&f.app, "olivia", "olivia-pass-123").await;
    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&olivia),
        Some(json!({"value": 30.0, "signature": signature("olivia-pass-123")})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["code"], "account_locked");
    assert!(f.edition.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_right_signature_password_resets_the_failed_count() {
    let f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let uri = format!("/api/v1/tags/{tag}/write");
    send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 30.0, "signature": signature("wrong")})),
    )
    .await;
    let source = crate::storage::SESSION_SOURCE;
    assert_eq!(
        f.state.storage.login_failure_count("admin", source).await,
        1
    );
    let (status, _) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"value": 31.0, "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(
        f.state.storage.login_failure_count("admin", source).await,
        0
    );
}

#[tokio::test]
async fn api_keys_cannot_sign() {
    let f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    let (_, key) = f
        .state
        .storage
        .create_api_key("plc-bridge", "admin", None, "[]", "integration")
        .await
        .unwrap();

    for body in [
        json!({"value": 30.0}),
        json!({"value": 30.0, "signature": signature("admin")}),
    ] {
        let (status, answer) = send_request(
            &f.app,
            "POST",
            &format!("/api/v1/tags/{tag}/write"),
            Some(&key),
            None,
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
        assert_eq!(answer["code"], "signature_not_possible");
    }
    let asked = f.edition.asked.lock().unwrap().clone();
    assert!(
        asked
            .iter()
            .all(|(_, origin, _)| *origin == ActionOrigin::ApiKey)
    );
    assert!(f.edition.records.lock().unwrap().is_empty());
}

#[tokio::test]
async fn mcp_and_flows_cannot_sign() {
    let f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    let admin = crate::api::AuthUser {
        username: "admin".into(),
        role: "admin".into(),
    };
    let body = crate::models::WriteRequest {
        value: json!(30.0),
        policy: "immediate".into(),
        scheduled_at: None,
        reason: None,
        signature: None,
    };
    let resp = crate::api::commands::write_tag_as(&f.state, &admin, ActionOrigin::Mcp, tag, body)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // A flow's write is refused when the edition wants it signed...
    let submission = || crate::api::commands::TagWriteSubmission {
        tag_id: tag,
        value: 31.0,
        policy: "immediate".into(),
        scheduled_at: None,
        reason: Some("flow f1".into()),
        command_id: None,
    };
    let checked =
        crate::api::commands::check_write(&f.state.storage, &f.state.cache, &submission())
            .await
            .unwrap();
    let refused = crate::api::commands::unattended_write_allowed(
        f.edition.as_ref(),
        "flow:f1",
        &submission(),
        &checked,
    )
    .await;
    assert!(matches!(
        refused,
        Err(crate::api::commands::WriteError::Signature(
            crate::api::signature::SignatureError::NotPossible(_)
        ))
    ));
    // ...and goes through when its policy lets flows write unsigned.
    let lenient = TestEdition {
        allow_flows: true,
        ..Default::default()
    };
    assert!(
        crate::api::commands::unattended_write_allowed(
            &lenient,
            "flow:f1",
            &submission(),
            &checked
        )
        .await
        .is_ok()
    );
    // Core never refuses a flow's write.
    assert!(
        crate::api::commands::unattended_write_allowed(
            &crate::edition::Community,
            "flow:f1",
            &submission(),
            &checked
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn alarm_acknowledge_and_shelve_need_the_signature_too() {
    let f = fixture(TestEdition::default()).await;
    let alarm = f
        .state
        .storage
        .create_alarm("dev1.temp", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    let admin = cookie(&f.app, "admin", "admin").await;
    let ack = format!("/api/v1/alarms/{alarm}/acknowledge");

    let (status, body) = send_request(&f.app, "POST", &ack, None, Some(&admin), None).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "alarm_ack");
    let (status, _) = send_request(
        &f.app,
        "POST",
        &ack,
        None,
        Some(&admin),
        Some(json!({"signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) =
        send_request(&f.app, "POST", "/api/v1/alarms", None, Some(&admin), None).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);

    let shelf = format!("/api/v1/alarms/{alarm}/shelf");
    let (status, body) = send_request(
        &f.app,
        "POST",
        &shelf,
        None,
        Some(&admin),
        Some(json!({"duration_secs": 600, "reason": "maintenance"})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    assert_eq!(body["signature_required"]["action"], "alarm_shelve");
    let (status, _) = send_request(
        &f.app,
        "POST",
        &shelf,
        None,
        Some(&admin),
        Some(
            json!({"duration_secs": 600, "reason": "maintenance", "signature": signature("admin")}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let records = f.edition.records.lock().unwrap().clone();
    let actions: Vec<_> = records.iter().map(|r| r.action).collect();
    assert_eq!(actions, [SignedAction::AlarmAck, SignedAction::AlarmShelve]);
    assert_eq!(records[0].resource, format!("alarm:{alarm}"));
}

#[tokio::test]
async fn mode_switches_need_the_signature() {
    let f = fixture(TestEdition::default()).await;
    let tag = writable_tag(&f.state).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let uri = format!("/api/v1/data-sources/{tag}/write-policy");
    let (status, body) = send_request(
        &f.app,
        "PATCH",
        &uri,
        None,
        Some(&admin),
        Some(json!({"write_max": 50.0})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "mode_switch");
    let (status, body) = send_request(
        &f.app,
        "PATCH",
        &uri,
        None,
        Some(&admin),
        Some(json!({"write_max": 50.0, "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let policy = f
        .state
        .storage
        .get_write_policy(tag)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(policy.write_max, Some(50.0));
    let record = f.edition.records.lock().unwrap()[0].clone();
    assert_eq!(record.resource, format!("data_source:{tag}:write_policy"));

    let (status, _) = send_request(
        &f.app,
        "POST",
        "/api/v1/devices/dev1/simulate",
        None,
        Some(&admin),
        Some(json!({"simulation_mode": true})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
}

#[tokio::test]
async fn community_asks_for_no_signature() {
    let mut state = seeded_admin_app_state().await;
    let (logger, mut audit) = AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state.clone());
    let tag = writable_tag(&state).await;
    let admin = cookie(&app, "admin", "admin").await;
    let (status, body) = send_request(
        &app,
        "POST",
        &format!("/api/v1/tags/{tag}/write"),
        None,
        Some(&admin),
        Some(json!({"value": 30.0, "reason": "trim"})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let command_id = body["command_id"].as_str().unwrap().to_string();

    // Previous and new value in the security audit entry...
    let entries = drain(&mut audit);
    let details = entries
        .iter()
        .find(|e| e.action == "tag_write")
        .and_then(|e| e.details.clone())
        .unwrap();
    assert_eq!(
        details,
        format!("command={command_id} previous=21.5 value=30 reason=trim")
    );
    // ...and the previous value on the command's first audit entry.
    let (status, trail) = send_request(
        &app,
        "GET",
        &format!("/api/v1/commands/{command_id}/audit"),
        None,
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trail[0]["previous_value"], 21.5);
}

#[tokio::test]
async fn the_audit_export_has_previous_and_new_value_columns() {
    let storage = Storage::new_in_memory().unwrap();
    storage
        .log_audit_event(
            "tag_write",
            "olivia",
            "data_source:7",
            Some("command=abc previous=21.5 value=30 signature=sig-9 meaning=\"Approved by operator\" reason=value=99 previous=1"),
        )
        .await
        .unwrap();
    storage
        .log_audit_event("create_user", "admin", "user:bob", Some("role=viewer"))
        .await
        .unwrap();
    let csv = storage
        .export_audit_logs_csv(None, None, None, None, None)
        .await
        .unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "id,action,username,resource,details,created_at,previous_value,new_value,signature"
    );
    let rows: Vec<&str> = lines.collect();
    let write = rows.iter().find(|l| l.contains("\"tag_write\"")).unwrap();
    assert!(
        write.ends_with(",\"21.5\",\"30\",\"sig-9\""),
        "the reason text cannot fake a field: {write}"
    );
    let other = rows.iter().find(|l| l.contains("\"create_user\"")).unwrap();
    assert!(other.ends_with(",\"\",\"\",\"\""), "{other}");
}

#[tokio::test]
async fn the_password_policy_is_asked_on_every_password_change() {
    let f = fixture(TestEdition::default()).await;
    let admin = cookie(&f.app, "admin", "admin").await;

    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/users",
        None,
        Some(&admin),
        Some(json!({"username": "bob", "password": "short-pw", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Password must have at least 12 characters");
    assert!(
        f.state
            .storage
            .get_user_by_username("bob")
            .await
            .unwrap()
            .is_none()
    );

    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/users",
        None,
        Some(&admin),
        Some(json!({"username": "bob", "password": "long-enough-pw", "role": "operator"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bob_id = body["id"].as_i64().unwrap();

    // Admin reset
    let (status, body) = send_request(
        &f.app,
        "PUT",
        &format!("/api/v1/users/{bob_id}"),
        None,
        Some(&admin),
        Some(json!({"password": "bob-is-my-name-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Password must not contain the username");

    // Self service
    let bob = cookie(&f.app, "bob", "long-enough-pw").await;
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(&bob),
        Some(json!({"old_password": "long-enough-pw", "new_password": "tiny-pw-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "Password must have at least 12 characters");
    let (status, _) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(&bob),
        Some(json!({"old_password": "long-enough-pw", "new_password": "a-much-better-pw"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let changes = f.edition.password_changes.lock().unwrap().clone();
    assert_eq!(
        changes,
        [
            ("bob".to_string(), PasswordChangeKind::AdminCreate),
            ("bob".to_string(), PasswordChangeKind::SelfService),
        ]
    );
}

#[tokio::test]
async fn an_expired_password_is_answered_like_must_change_password() {
    let f = fixture(TestEdition::default()).await;
    add_user(&f.state.storage, "stale", "old-password-1", "operator").await;
    add_user(&f.state.storage, "fresh", "new-password-1", "operator").await;
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        None,
        Some(json!({"username": "stale", "password": "old-password-1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["must_change_password"], true);
    assert_eq!(body["password_expired"], true);

    // The session may only change the password until it has.
    let stale = cookie(&f.app, "stale", "old-password-1").await;
    let (status, body) =
        send_request(&f.app, "GET", "/api/v1/screens", None, Some(&stale), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "password_change_required");
    let (_, me) = send_request(&f.app, "GET", "/api/v1/auth/me", None, Some(&stale), None).await;
    assert_eq!(me["must_change_password"], true);
    assert_eq!(me["password_expired"], true);
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(&stale),
        Some(json!({"old_password": "old-password-1", "new_password": "a-much-newer-pass-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) =
        send_request(&f.app, "GET", "/api/v1/screens", None, Some(&stale), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/login",
        None,
        None,
        Some(json!({"username": "fresh", "password": "new-password-1"})),
    )
    .await;
    assert_eq!(body["must_change_password"], false);
    assert_eq!(body["password_expired"], false);
}

/// Sign in and return the (access, refresh) cookie values and the login body.
async fn sign_in(
    app: &Router,
    username: &str,
    password: &str,
) -> (String, String, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"username": username, "password": password}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let access = extract_cookie_value(resp.headers(), "voltrus_token").unwrap();
    let refresh = extract_cookie_value(resp.headers(), "voltrus_refresh").unwrap();
    let max_age = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|c| c.starts_with("voltrus_token="))
        .unwrap()
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["_access_cookie"] = json!(max_age);
    (access, refresh, body)
}

async fn refresh(app: &Router, refresh: &str) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(&format!("voltrus_refresh={refresh}")),
        None,
    )
    .await
}

#[tokio::test]
async fn an_idle_session_ends_at_refresh() {
    let f = fixture(TestEdition {
        session: SessionPolicy {
            idle_timeout_secs: Some(300),
            absolute_timeout_secs: None,
        },
        ..Default::default()
    })
    .await;
    let (_, refresh_token, body) = sign_in(&f.app, "admin", "admin").await;
    assert_eq!(body["session"]["idle_timeout_secs"], 300);
    assert!(
        body["_access_cookie"]
            .as_str()
            .unwrap()
            .contains("Max-Age=300"),
        "the access token lives no longer than the idle timeout: {}",
        body["_access_cookie"]
    );

    let hash = crate::api::auth::hash_refresh_token(&refresh_token);
    let now = crate::timeutil::now_secs();

    // Active 100 s ago: the session goes on.
    f.state
        .storage
        .set_refresh_token_session_times(&hash, now - 1000, now - 100)
        .await
        .unwrap();
    let (status, _) = send_request(
        &f.app,
        "POST",
        "/api/v1/auth/activity",
        None,
        Some(&format!("voltrus_refresh={refresh_token}")),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let rt = f
        .state
        .storage
        .get_refresh_token_by_hash(&hash)
        .await
        .unwrap()
        .unwrap();
    assert!(rt.last_activity_at.unwrap() >= now, "activity recorded");

    // Idle for 400 s: refresh refuses.
    f.state
        .storage
        .set_refresh_token_session_times(&hash, now - 1000, now - 400)
        .await
        .unwrap();
    let (status, body) = refresh(&f.app, &refresh_token).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "session_idle");
}

#[tokio::test]
async fn a_session_ends_at_its_absolute_lifetime_and_keeps_its_start_across_refreshes() {
    let f = fixture(TestEdition {
        session: SessionPolicy {
            idle_timeout_secs: None,
            absolute_timeout_secs: Some(3600),
        },
        ..Default::default()
    })
    .await;
    let (_, first, _) = sign_in(&f.app, "admin", "admin").await;
    let now = crate::timeutil::now_secs();
    f.state
        .storage
        .set_refresh_token_session_times(
            &crate::api::auth::hash_refresh_token(&first),
            now - 3000,
            now - 3000,
        )
        .await
        .unwrap();

    // Still inside the hour: a new refresh token, same session start.
    let resp = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/refresh")
                .header("cookie", format!("voltrus_refresh={first}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let second = extract_cookie_value(resp.headers(), "voltrus_refresh").unwrap();
    let rt = f
        .state
        .storage
        .get_refresh_token_by_hash(&crate::api::auth::hash_refresh_token(&second))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rt.session_started_at, Some(now - 3000));

    f.state
        .storage
        .set_refresh_token_session_times(
            &crate::api::auth::hash_refresh_token(&second),
            now - 4000,
            now,
        )
        .await
        .unwrap();
    let (status, body) = refresh(&f.app, &second).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "session_expired");
}

#[tokio::test]
async fn community_sessions_have_no_limit() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let (_, refresh_token, body) = sign_in(&app, "admin", "admin").await;
    assert_eq!(
        body["session"]["idle_timeout_secs"],
        serde_json::Value::Null
    );
    assert!(
        body["_access_cookie"]
            .as_str()
            .unwrap()
            .contains("Max-Age=900")
    );
    let now = crate::timeutil::now_secs();
    state
        .storage
        .set_refresh_token_session_times(
            &crate::api::auth::hash_refresh_token(&refresh_token),
            now - 86_400 * 5,
            now - 86_400 * 5,
        )
        .await
        .unwrap();
    let (status, _) = refresh(&app, &refresh_token).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_password_that_expires_during_a_session_holds_it_at_refresh() {
    let f = fixture(TestEdition::default()).await;
    add_user(&f.state.storage, "olga", "olga-password-1", "operator").await;
    let (_, refresh_token, body) = sign_in(&f.app, "olga", "olga-password-1").await;
    assert_eq!(body["must_change_password"], false);

    f.edition
        .expire_all
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (status, _) = refresh(&f.app, &refresh_token).await;
    assert_eq!(status, StatusCode::OK);
    let olga = f
        .state
        .storage
        .get_user_by_username("olga")
        .await
        .unwrap()
        .unwrap();
    assert!(olga.must_change_password, "the refresh flags the account");
}

#[tokio::test]
async fn a_config_rollback_needs_the_signature_and_is_audited() {
    let mut f = fixture(TestEdition::default()).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let snap = f
        .state
        .storage
        .create_config_snapshot(Some("before"), None, "manual", None)
        .await
        .unwrap();
    let uri = format!("/api/v1/config/snapshots/{snap}/rollback");
    let (status, body) = send_request(&f.app, "POST", &uri, None, Some(&admin), None).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "mode_switch");
    drain(&mut f.audit);

    let (status, body) = send_request(
        &f.app,
        "POST",
        &uri,
        None,
        Some(&admin),
        Some(json!({"signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let record = f.edition.records.lock().unwrap()[0].clone();
    assert_eq!(record.resource, format!("config_snapshot:{snap}:rollback"));
    let events = drain(&mut f.audit);
    let event = events
        .iter()
        .find(|e| e.action == "config_rollback")
        .expect("an audit entry for the rollback");
    assert!(
        event
            .details
            .as_deref()
            .unwrap()
            .contains("signature=sig-1"),
        "{event:?}"
    );
}

#[tokio::test]
async fn a_direct_plc_write_follows_the_tag_write_policy_and_signature() {
    let mut f = fixture(TestEdition::default()).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let plc = f
        .state
        .storage
        .create_voltrus_plc_device("PLC", "http://127.0.0.1:1", "http", 1000, false)
        .await
        .unwrap();
    let tag = f
        .state
        .storage
        .create_data_source(
            "speed",
            "Speed",
            "",
            "#fff",
            0.0,
            100.0,
            0.0,
            0.0,
            0.0,
            0,
            1,
            "voltrus_plc",
            "{}",
            &plc.to_string(),
            None,
            "line",
        )
        .await
        .unwrap();
    let uri = format!("/api/v1/voltrus-plc/devices/{plc}/write");
    let write = |body: serde_json::Value| {
        send_request(&f.app, "POST", &uri, None, Some(&admin), Some(body))
    };

    // Not writable yet.
    let (status, _) = write(json!({"variable": "speed", "value": 10.0})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // A variable no tag reads.
    let (status, _) = write(json!({"variable": "other", "value": 10.0})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    f.state
        .storage
        .update_write_policy(
            tag,
            &crate::models::WritePolicyUpdate {
                writable: Some(true),
                write_min: Some(Some(0.0)),
                write_max: Some(Some(50.0)),
                write_max_rate: None,
                write_requires_reason: None,
                write_deadband: None,
            },
        )
        .await
        .unwrap();
    let (status, body) = write(json!({"variable": "speed", "value": 80.0})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "above write_max: {body}");
    let (status, body) = write(json!({"variable": "speed", "value": 10.0})).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "tag_write");
    drain(&mut f.audit);

    let (status, body) =
        write(json!({"variable": "speed", "value": 10.0, "signature": signature("admin")})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events = drain(&mut f.audit);
    let event = events
        .iter()
        .find(|e| e.action == "voltrus_plc_write")
        .expect("the write is audited");
    let details = event.details.as_deref().unwrap();
    assert!(details.contains("\"value\":10.0"), "{details}");
    assert!(details.contains("signature=sig-1"), "{details}");

    // The write is in the command trail and counts toward write_max_rate.
    let recent = f.state.storage.count_recent_writes(tag, 60).await.unwrap();
    assert_eq!(recent, 1);
    f.state
        .storage
        .update_write_policy(
            tag,
            &crate::models::WritePolicyUpdate {
                writable: None,
                write_min: None,
                write_max: None,
                write_max_rate: Some(Some(1)),
                write_requires_reason: None,
                write_deadband: None,
            },
        )
        .await
        .unwrap();
    let (status, body) =
        write(json!({"variable": "speed", "value": 20.0, "signature": signature("admin")})).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
}

/// An equipment write that changes a tag's write policy (here: instantiating
/// a pump makes its command tag writable) needs the signature the
/// write-policy route needs; the dry run does not, and lists the change. The
/// audit entry lists every policy field before and after.
#[tokio::test]
async fn equipment_changes_to_write_policies_need_the_signature() {
    let mut f = fixture(TestEdition::default()).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let call = |method: &'static str, uri: &'static str, body: Option<serde_json::Value>| {
        let app = f.app.clone();
        let admin = admin.clone();
        async move { send_request(&app, method, uri, None, Some(&admin), body).await }
    };
    // Installing types changes no tag: no signature.
    let (status, body) = call(
        "POST",
        "/api/v1/equipment-types/library/install",
        Some(json!({"ids": ["pump"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = call(
        "POST",
        "/api/v1/modbus-devices",
        Some(json!({"id": "plc", "name": "PLC", "ip": "127.0.0.1", "port": 1, "registers": []})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let inst = json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0});
    let (status, dry) = call(
        "POST",
        "/api/v1/equipment-instances?dry_run=true",
        Some(inst.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    let cmd = dry["changes"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["key"] == "p1_cmd")
        .unwrap()
        .clone();
    assert!(
        cmd["diff"]
            .as_array()
            .unwrap()
            .contains(&json!({"field": "write.writable", "before": null, "after": true})),
        "{cmd}"
    );

    let (status, body) = call("POST", "/api/v1/equipment-instances", Some(inst.clone())).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "mode_switch");
    assert!(
        f.state
            .storage
            .get_equipment_instance("P-1")
            .await
            .unwrap()
            .is_none(),
        "nothing written without the signature"
    );

    drain(&mut f.audit);
    let mut signed = inst.clone();
    signed["signature"] = signature("admin");
    let (status, body) = call("POST", "/api/v1/equipment-instances", Some(signed)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let record = f.edition.records.lock().unwrap().last().cloned().unwrap();
    assert_eq!(record.resource, "equipment_instance:P-1:write_policy");
    assert!(
        record.details.to_string().contains("p1_cmd"),
        "{}",
        record.details
    );
    let entries = drain(&mut f.audit);
    let entry = entries
        .iter()
        .find(|e| e.action == "create_equipment_instance")
        .expect("audit entry");
    let details = entry.details.clone().unwrap_or_default();
    assert!(
        details.contains("write.writable") && details.contains("signature=sig-"),
        "{details}"
    );

    // An MCP client cannot sign: refused, nothing written.
    let user = crate::api::AuthUser {
        username: "admin".into(),
        role: "admin".into(),
    };
    let resp = crate::api::equipment_types::create_instance_as(
        &f.state,
        &user,
        ActionOrigin::Mcp,
        false,
        json!({"name": "P-2", "type": "pump", "device": "plc", "base": 100}),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(
        f.state
            .storage
            .get_equipment_instance("P-2")
            .await
            .unwrap()
            .is_none()
    );
}

/// Publishing a Designer screen is the `design_publish` signed action: the
/// author signs the request and a second admin signs the approval (21 CFR
/// Part 11), and MCP cannot sign (docs/version-history.md).
#[tokio::test]
async fn a_screen_publish_and_its_approval_are_signed() {
    let mut f = fixture(TestEdition::default()).await;
    add_user(&f.state.storage, "reviewer", "reviewer-pass-1", "admin").await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let reviewer = cookie(&f.app, "reviewer", "reviewer-pass-1").await;
    let rows = json!({"equipment": [{"equipment_id": "a", "equipment_type": "label", "equipment_name": "",
        "x": 0, "y": 0, "width": 10, "height": 10, "rotation": 0, "sensor_bindings": "[]", "config": "{}"}],
        "connections": []});
    let (status, _) = send_request(
        &f.app,
        "PUT",
        "/api/v1/screens/s1/scada-layout",
        None,
        Some(&admin),
        Some(rows),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Direct publish: 428, then published once signed.
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/screens/s1/publish",
        None,
        Some(&admin),
        Some(json!({"message": "m"})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "design_publish");
    drain(&mut f.audit);
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/screens/s1/publish",
        None,
        Some(&admin),
        Some(json!({"message": "m", "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["version"]["signature"], "sig-1");
    let record = f.edition.records.lock().unwrap()[0].clone();
    assert_eq!(record.action, SignedAction::DesignPublish);
    assert_eq!(record.resource, "screen:s1:publish");
    assert_eq!(record.details["stage"], "publish");
    let entries = drain(&mut f.audit);
    let e = entries
        .iter()
        .find(|e| e.action == "design_publish")
        .unwrap();
    assert!(e.details.as_deref().unwrap().contains("signature=sig-1"));

    // With approval required: the request and the approval each signed.
    let (status, _) = send_request(
        &f.app,
        "PUT",
        "/api/v1/settings/design-history",
        None,
        Some(&admin),
        Some(json!({"approval": "required", "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/screens/s1/publish",
        None,
        Some(&admin),
        Some(json!({"name": "r2", "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "pending");
    let (status, _) = send_request(
        &f.app,
        "POST",
        "/api/v1/screens/s1/versions/r2/approve",
        None,
        Some(&reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/screens/s1/versions/r2/approve",
        None,
        Some(&reviewer),
        Some(json!({"signature": signature("reviewer-pass-1")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let stages: Vec<String> = f
        .edition
        .records
        .lock()
        .unwrap()
        .iter()
        .map(|r| format!("{}:{}", r.username, r.details["stage"].as_str().unwrap()))
        .collect();
    assert_eq!(
        stages,
        [
            "admin:publish",
            "admin:settings",
            "admin:request",
            "reviewer:approve"
        ]
    );
    assert_eq!(body["version"]["signature"], "sig-4");
}

/// "Publish after import" is the `design_publish` action too: without the
/// signature (the `X-Voltrus-Signature` header) the import writes nothing;
/// with approval required it files publish requests; settings changes that
/// touch publishing are signed.
#[tokio::test]
async fn an_import_that_publishes_is_signed() {
    let f = fixture(TestEdition::default()).await;
    let admin = cookie(&f.app, "admin", "admin").await;
    let yaml = "format: voltrus-project/1\nscreens:\n- id: s9\n  name: Nine\n  scada:\n    equipment:\n    - {equipment_id: a, equipment_type: label, x: 4, config: {text: A}}\n";
    let import = |sig: Option<serde_json::Value>| {
        let app = f.app.clone();
        let admin = admin.clone();
        async move {
            let mut req = http::Request::builder()
                .method("POST")
                .uri("/api/v1/project/import?publish=1")
                .header("cookie", &admin)
                .header("content-type", "application/yaml");
            if let Some(s) = sig {
                req = req.header("x-voltrus-signature", s.to_string());
            }
            let resp = tower::ServiceExt::oneshot(
                app,
                req.body(axum::body::Body::from(yaml.to_string())).unwrap(),
            )
            .await
            .unwrap();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let (status, body) = import(None).await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED, "{body}");
    assert_eq!(body["signature_required"]["action"], "design_publish");
    let (status, _) = send_request(
        &f.app,
        "GET",
        "/api/v1/screens/s9/scada-layout",
        None,
        Some(&admin),
        None,
    )
    .await;
    assert!(status.is_success());
    let (_, h) = send_request(
        &f.app,
        "GET",
        "/api/v1/screens/s9/history",
        None,
        Some(&admin),
        None,
    )
    .await;
    assert!(
        h["changes"].as_array().unwrap().is_empty(),
        "nothing written: {h}"
    );

    let (status, plan) = import(Some(signature("admin"))).await;
    assert_eq!(status, StatusCode::OK, "{plan}");
    assert_eq!(plan["published"], json!(["s9"]));
    let record = f.edition.records.lock().unwrap().last().cloned().unwrap();
    assert_eq!(record.resource, "project:import:publish");
    assert_eq!(record.details["stage"], "publish");
    assert_eq!(
        record.details["document_sha256"].as_str().unwrap().len(),
        64
    );

    // Requiring approval is signed; the import then files a request.
    let (status, _) = send_request(
        &f.app,
        "PUT",
        "/api/v1/settings/design-history",
        None,
        Some(&admin),
        Some(json!({"approval": "required"})),
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_REQUIRED);
    let (status, _) = send_request(
        &f.app,
        "PUT",
        "/api/v1/settings/design-history",
        None,
        Some(&admin),
        Some(json!({"approval": "required", "signature": signature("admin")})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let changed = yaml.replace("x: 4", "x: 8");
    let req = http::Request::builder()
        .method("POST")
        .uri("/api/v1/project/import?publish=1")
        .header("cookie", &admin)
        .header("content-type", "application/yaml")
        .header("x-voltrus-signature", signature("admin").to_string())
        .body(axum::body::Body::from(changed))
        .unwrap();
    let resp = tower::ServiceExt::oneshot(f.app.clone(), req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(plan["publish_requested"], json!(["s9"]), "{plan}");
    let record = f.edition.records.lock().unwrap().last().cloned().unwrap();
    assert_eq!(record.details["stage"], "request");
}
