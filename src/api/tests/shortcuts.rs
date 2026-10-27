//! SCADA-17 S5: a cleared shortcut survived as an empty row.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn put(app: &Router, cookie: &str, path: &str, body: serde_json::Value) {
    let (status, answer) = send_request(app, "PUT", path, None, Some(cookie), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "PUT {path}: {answer}");
}

async fn get(app: &Router, cookie: &str, path: &str) -> serde_json::Value {
    let (status, body) = send_request(app, "GET", path, None, Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
    body
}

/// The handler says "if value is null or empty string, delete the entry", but
/// the storage wrote `""` for them and `GET` returned it.
#[tokio::test]
async fn test_cleared_tag_shortcut_is_removed_not_kept_empty() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    put(
        &app,
        &cookie,
        "/api/v1/shortcuts",
        json!({"1": "Mod+K", "2": "Mod+J", "3": "Mod+L"}),
    )
    .await;
    assert_eq!(
        get(&app, &cookie, "/api/v1/shortcuts").await,
        json!({"1": "Mod+K", "2": "Mod+J", "3": "Mod+L"})
    );

    put(
        &app,
        &cookie,
        "/api/v1/shortcuts",
        json!({"1": null, "2": "", "3": "Mod+L"}),
    )
    .await;
    assert_eq!(
        get(&app, &cookie, "/api/v1/shortcuts").await,
        json!({"3": "Mod+L"})
    );
}

/// For navigation an empty entry is a choice, not a leftover: it removes the
/// target's default (`screen:all` is `Mod+1`). Entries not named in the body
/// go back to their defaults, since the body is the user's whole map.
#[tokio::test]
async fn test_nav_shortcut_null_overrides_default_with_none() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let defaults = get(&app, &cookie, "/api/v1/nav-shortcuts").await;
    assert_eq!(defaults["screen:all"], "Mod+1");

    put(
        &app,
        &cookie,
        "/api/v1/nav-shortcuts",
        json!({"screen:all": null, "action:alarms": "Mod+Shift+A"}),
    )
    .await;
    let map = get(&app, &cookie, "/api/v1/nav-shortcuts").await;
    assert!(map.get("screen:all").is_none(), "{map}");
    assert_eq!(map["action:alarms"], "Mod+Shift+A");
    assert_eq!(map["screen:system"], "Mod+0");

    put(&app, &cookie, "/api/v1/nav-shortcuts", json!({})).await;
    let map = get(&app, &cookie, "/api/v1/nav-shortcuts").await;
    assert_eq!(map["screen:all"], "Mod+1");
    assert_eq!(map["action:alarms"], "");
}
