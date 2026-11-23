//! Outbound webhooks: secrets masked for readers who may not configure, and
//! the address rules of `security::outbound` on create and on the test button.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn app() -> (Router, AppState, String, String) {
    let state = seeded_admin_app_state().await;
    let hash = crate::api::auth::hash_password("password-123");
    state
        .storage
        .create_user("victor", &hash, "", "viewer")
        .await
        .unwrap();
    let app = create_router(state.clone());
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let viewer = format!(
        "voltrus_token={}",
        login_cookie(&app, "victor", "password-123").await
    );
    (app, state, admin, viewer)
}

#[tokio::test]
async fn viewers_see_webhook_secrets_masked() {
    let (app, state, admin, viewer) = app().await;
    state
        .storage
        .create_outbound_webhook(
            "erp",
            &format!("https://svc:{}@erp.example.com/in", "hunter2"),
            r#"["alarm"]"#,
            r#""all""#,
            r#"{"Authorization":"Bearer s3cr3t-token"}"#,
            true,
            3,
            1000,
        )
        .await
        .unwrap();
    let uri = "/api/v1/integration/webhooks/outbound";
    let (status, body) = send_request(&app, "GET", uri, None, Some(&viewer), None).await;
    assert_eq!(status, StatusCode::OK);
    let text = body.to_string();
    assert!(
        !text.contains("hunter2") && !text.contains("s3cr3t-token"),
        "{text}"
    );
    assert!(text.contains("erp.example.com"));

    let (_, body) = send_request(&app, "GET", uri, None, Some(&admin), None).await;
    assert!(
        body.to_string().contains("s3cr3t-token"),
        "admins configure it"
    );
}

#[tokio::test]
async fn metadata_and_loopback_addresses_are_refused() {
    let (app, state, admin, _) = app().await;
    for url in [
        "http://169.254.169.254/latest/meta-data/",
        "http://127.0.0.1:8080/admin",
    ] {
        let (status, body) = send_request(
            &app,
            "POST",
            "/api/v1/integration/webhooks/outbound",
            None,
            Some(&admin),
            Some(json!({"name": "x", "url": url})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{url}: {body}");
    }

    // One stored before the rules (or by hand) is refused by the test button,
    // without a request and without a body in the answer.
    let id = state
        .storage
        .create_outbound_webhook(
            "meta",
            "http://169.254.169.254/latest/meta-data/",
            r#"["alarm"]"#,
            r#""all""#,
            "{}",
            true,
            0,
            1000,
        )
        .await
        .unwrap();
    let (status, body) = send_request(
        &app,
        "POST",
        &format!("/api/v1/integration/webhooks/outbound/{id}/test"),
        None,
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], false, "{body}");
    assert!(body.get("response").is_none());
}
