//! SCADA-17 S20: notification settings validation and the SMTP-less test.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn create(
    app: &Router,
    cookie: &str,
    channel: &str,
    config: &str,
) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "POST",
        "/api/v1/notifications/settings",
        None,
        Some(cookie),
        Some(
            json!({"source_key": "k", "channel_type": channel, "config": config, "enabled": true}),
        ),
    )
    .await
}

async fn update(
    app: &Router,
    cookie: &str,
    id: i64,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "PUT",
        &format!("/api/v1/notifications/settings/{id}"),
        None,
        Some(cookie),
        Some(body),
    )
    .await
}

/// `POST` answered a refused setting with 500 and `PUT` skipped the email and
/// URL shape checks `POST` makes.
#[tokio::test]
async fn test_create_and_update_share_the_setting_checks() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = create(&app, &cookie, "webhook", r#"["ftp://hook"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = create(&app, &cookie, "email", r#"["nobody"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = create(&app, &cookie, "fax", r#"["x"]"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, hook) = create(&app, &cookie, "webhook", r#"["https://hook.example.com"]"#).await;
    assert_eq!(status, StatusCode::CREATED, "{hook}");
    let hook_id = hook["id"].as_i64().unwrap();

    // The same shape checks on update.
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"config": r#"["ftp://hook"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("webhook URL"),
        "{body}"
    );
    let (status, body) = update(&app, &cookie, hook_id, json!({"config": "[]"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Switching the channel checks the stored config against it.
    let (status, body) = update(&app, &cookie, hook_id, json!({"channel_type": "email"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = update(&app, &cookie, hook_id, json!({"channel_type": "fax"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // Valid changes, and one that touches neither channel nor config, pass.
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"channel_type": "email", "config": r#"["ops@example.com"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = update(&app, &cookie, hook_id, json!({"enabled": false})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = update(
        &app,
        &cookie,
        hook_id,
        json!({"config": r#"["not-an-address"]"#}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (status, body) = update(&app, &cookie, 9999, json!({"config": r#"["a@b.c"]"#})).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // What was refused changed nothing.
    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/notifications/settings",
        None,
        Some(&cookie),
        None,
    )
    .await;
    let row = &list.as_array().unwrap()[0];
    assert_eq!(row["channel_type"], "email");
    assert_eq!(row["config"], r#"["ops@example.com"]"#);
}

/// The test of an email setting without SMTP configured answered 500.
#[tokio::test]
async fn test_notification_test_without_smtp_is_a_client_error() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "email", "config": r#"["ops@example.com"]"#})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "SMTP is not configured");

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/notifications/test",
        None,
        Some(&cookie),
        Some(json!({"channel_type": "push", "config": "[]"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
