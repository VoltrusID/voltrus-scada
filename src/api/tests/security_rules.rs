//! Who may do what: alarm acknowledgement, the operator-tier routes that create
//! things or spend LLM credit, API key scopes and `?api_key=`.

use super::*;
use http::StatusCode;
use serde_json::json;

struct Fx {
    app: Router,
    storage: Storage,
    admin: String,
    viewer: String,
    ro_key: String,
    int_key: String,
}

async fn key(app: &Router, admin: &str, role: &str, permissions: &str) -> String {
    let (status, body) = send_request(
        app,
        "POST",
        "/api/v1/api-keys",
        None,
        Some(admin),
        Some(json!({"name": format!("k-{role}"), "role": role, "permissions": permissions})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["key"].as_str().unwrap().to_string()
}

async fn fx() -> Fx {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    storage
        .create_user(
            "watcher",
            &crate::api::auth::hash_password("watcher-pw"),
            "",
            "viewer",
        )
        .await
        .unwrap();
    let app = create_router(state);
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let viewer = format!(
        "voltrus_token={}",
        login_cookie(&app, "watcher", "watcher-pw").await
    );
    let ro_key = key(&app, &admin, "api", "[]").await;
    let int_key = key(&app, &admin, "integration", "[]").await;
    Fx {
        app,
        storage,
        admin,
        viewer,
        ro_key,
        int_key,
    }
}

async fn alarm(storage: &Storage) -> i64 {
    storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap()
}

#[tokio::test]
async fn read_only_user_may_acknowledge_but_read_only_key_may_not() {
    let f = fx().await;
    let a = alarm(&f.storage).await;
    let uri = format!("/api/v1/alarms/{a}/acknowledge");

    let (status, body) = send_request(&f.app, "POST", &uri, Some(&f.ro_key), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, _) = send_request(
        &f.app,
        "POST",
        "/api/v1/alarms",
        Some(&f.ro_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "acknowledge-all too");

    let (status, body) = send_request(&f.app, "POST", &uri, None, Some(&f.viewer), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let b = alarm(&f.storage).await;
    let (status, body) = send_request(
        &f.app,
        "POST",
        &format!("/api/v1/alarms/{b}/acknowledge"),
        Some(&f.int_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn shelving_wizard_and_copilot_need_the_operator_tier() {
    let f = fx().await;
    let a = alarm(&f.storage).await;
    let routes = [
        format!("/api/v1/alarms/{a}/shelf"),
        "/api/v1/wizard/create".to_string(),
        "/api/v1/ai/settings/test".to_string(),
        "/api/v1/ai/generate/register-map".to_string(),
        "/api/v1/ai/generate/dashboard-layout".to_string(),
        "/api/v1/ai/generate/alarm-thresholds".to_string(),
        "/api/v1/ai/generate/tag-rename".to_string(),
        "/api/v1/ai/generate/flow-rule".to_string(),
        "/api/v1/ai/generate/from-template".to_string(),
    ];
    for uri in &routes {
        let (status, _) =
            send_request(&f.app, "POST", uri, None, Some(&f.viewer), Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "viewer on {uri}");
        let (status, _) =
            send_request(&f.app, "POST", uri, Some(&f.ro_key), None, Some(json!({}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "read-only key on {uri}");
        let (status, _) =
            send_request(&f.app, "POST", uri, None, Some(&f.admin), Some(json!({}))).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "admin on {uri}");
    }
}

#[tokio::test]
async fn key_scopes_narrow_an_integration_key() {
    let f = fx().await;
    let read_scoped = key(&f.app, &f.admin, "integration", r#"["data:read"]"#).await;
    let write_scoped = key(&f.app, &f.admin, "integration", r#"["read","write"]"#).await;
    let a = alarm(&f.storage).await;
    let uri = format!("/api/v1/alarms/{a}/acknowledge");

    let (status, _) = send_request(&f.app, "POST", &uri, Some(&read_scoped), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "no write scope: read-only");
    let (status, _) = send_request(&f.app, "POST", &uri, Some(&write_scoped), None, None).await;
    assert_eq!(status, StatusCode::OK);

    // Creation refuses scopes the key could never use.
    for (role, perms) in [("api", r#"["write"]"#), ("integration", r#"["admin"]"#)] {
        let (status, body) = send_request(
            &f.app,
            "POST",
            "/api/v1/api-keys",
            None,
            Some(&f.admin),
            Some(json!({"name": "bad", "role": role, "permissions": perms})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{role} {perms}: {body}");
    }
}

#[tokio::test]
async fn api_key_in_the_url_only_on_metrics_and_stream() {
    let f = fx().await;
    let (status, body) = send_request(
        &f.app,
        "GET",
        &format!("/api/v1/data-sources?api_key={}", f.ro_key),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("X-API-Key"));

    let (status, _) = send_request(
        &f.app,
        "GET",
        &format!("/metrics?api_key={}", f.ro_key),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The header keeps working everywhere.
    let (status, _) = send_request(
        &f.app,
        "GET",
        "/api/v1/data-sources",
        Some(&f.ro_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
