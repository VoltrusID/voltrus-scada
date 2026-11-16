//! The built-in roles: `operator` runs the plant (tag writes within the write
//! policy, alarm handling, recipe loads, flow start/stop) but cannot
//! configure; `viewer` (and the legacy `user`) only reads.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn app_with_users() -> (Router, AppState) {
    let state = seeded_admin_app_state().await;
    for (name, role) in [
        ("olivia", "operator"),
        ("victor", "viewer"),
        ("ursula", "user"),
    ] {
        let hash = crate::api::auth::hash_password("password-123");
        state
            .storage
            .create_user(name, &hash, "", role)
            .await
            .unwrap();
    }
    (create_router(state.clone()), state)
}

async fn cookie(app: &Router, username: &str) -> String {
    let password = if username == "admin" {
        "admin"
    } else {
        "password-123"
    };
    format!(
        "voltrus_token={}",
        login_cookie(app, username, password).await
    )
}

async fn writable_tag(app: &Router, state: &AppState) -> i64 {
    let id = state
        .storage
        .create_data_source(
            "dev1.speed",
            "Speed",
            "rpm",
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
    let admin = cookie(app, "admin").await;
    let (status, _) = send_request(
        app,
        "PATCH",
        &format!("/api/v1/data-sources/{id}/write-policy"),
        None,
        Some(&admin),
        Some(json!({"writable": true, "write_max": 80.0})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    id
}

#[test]
fn role_tiers() {
    use crate::api::{role_allows_configuration, role_allows_operation};
    for role in ["admin", "demo", "integration", "operator"] {
        assert!(role_allows_operation(role), "{role} operates");
    }
    for role in ["viewer", "user", "api", "", "Operator"] {
        assert!(!role_allows_operation(role), "{role} only reads");
    }
    assert!(!role_allows_configuration("operator"));
    assert!(role_allows_configuration("demo"));
}

#[tokio::test]
async fn an_operator_writes_tags_and_handles_alarms() {
    let (app, state) = app_with_users().await;
    let tag = writable_tag(&app, &state).await;
    let olivia = cookie(&app, "olivia").await;

    // The write dialog reads the policy first.
    let (status, _) = send_request(
        &app,
        "GET",
        &format!("/api/v1/data-sources/{tag}/write-policy"),
        None,
        Some(&olivia),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send_request(
        &app,
        "POST",
        &format!("/api/v1/tags/{tag}/write"),
        None,
        Some(&olivia),
        Some(json!({"value": 42.0})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        state
            .storage
            .get_command(body["command_id"].as_str().unwrap())
            .await
            .unwrap()
            .unwrap()
            .created_by,
        "olivia"
    );

    // Within the write policy only.
    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/tags/{tag}/write"),
        None,
        Some(&olivia),
        Some(json!({"value": 95.0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let alarm = state
        .storage
        .create_alarm("dev1.speed", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    for (uri, body) in [
        (format!("/api/v1/alarms/{alarm}/acknowledge"), None),
        (
            format!("/api/v1/alarms/{alarm}/shelf"),
            Some(json!({"duration_secs": 60, "reason": "sensor swap"})),
        ),
        (format!("/api/v1/alarms/{alarm}/unshelf"), None),
        ("/api/v1/alarms".to_string(), None),
    ] {
        let (status, answer) = send_request(&app, "POST", &uri, None, Some(&olivia), body).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {answer}");
    }
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/alarms/summary",
        None,
        Some(&olivia),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for uri in ["/api/v1/recipes", "/api/v1/flows", "/api/v1/shift-handover"] {
        let (status, _) = send_request(&app, "GET", uri, None, Some(&olivia), None).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
}

#[tokio::test]
async fn an_operator_cannot_configure() {
    let (app, state) = app_with_users().await;
    let tag = writable_tag(&app, &state).await;
    let olivia = cookie(&app, "olivia").await;
    let calls = [
        (
            "PATCH",
            format!("/api/v1/data-sources/{tag}/write-policy"),
            Some(json!({"write_max": 100.0})),
        ),
        (
            "PUT",
            format!("/api/v1/data-sources/{tag}"),
            Some(json!({"name": "renamed"})),
        ),
        (
            "POST",
            "/api/v1/screens".to_string(),
            Some(json!({"id": "s2", "name": "Screen 2"})),
        ),
        (
            "POST",
            "/api/v1/recipes".to_string(),
            Some(json!({"name": "r", "device_id": "dev1", "values": "{}"})),
        ),
        (
            "POST",
            "/api/v1/flows".to_string(),
            Some(json!({"name": "f"})),
        ),
        (
            "PUT",
            "/api/v1/settings/alarm-cooldown".to_string(),
            Some(json!({"value": 10})),
        ),
        ("GET", "/api/v1/users".to_string(), None),
    ];
    for (method, uri, body) in calls {
        let (status, _) = send_request(&app, method, &uri, None, Some(&olivia), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

#[tokio::test]
async fn a_viewer_only_reads() {
    let (app, state) = app_with_users().await;
    let tag = writable_tag(&app, &state).await;
    let alarm = state
        .storage
        .create_alarm("dev1.speed", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    for user in ["victor", "ursula"] {
        let c = cookie(&app, user).await;
        let (status, _) = send_request(
            &app,
            "POST",
            &format!("/api/v1/tags/{tag}/write"),
            None,
            Some(&c),
            Some(json!({"value": 42.0})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{user} cannot write");
        let (status, _) = send_request(
            &app,
            "POST",
            &format!("/api/v1/alarms/{alarm}/acknowledge"),
            None,
            Some(&c),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{user} cannot acknowledge");
        let (status, _) = send_request(&app, "GET", "/api/v1/alarms", None, Some(&c), None).await;
        assert_eq!(status, StatusCode::OK, "{user} reads alarms");
    }
}

#[tokio::test]
async fn a_read_only_key_cannot_acknowledge() {
    let (app, state) = app_with_users().await;
    let alarm = state
        .storage
        .create_alarm("dev1.speed", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    let (_, key) = state
        .storage
        .create_api_key("dash", "admin", None, "[]", "api")
        .await
        .unwrap();
    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/alarms/{alarm}/acknowledge"),
        Some(&key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}
