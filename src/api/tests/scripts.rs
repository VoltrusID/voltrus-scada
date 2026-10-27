use super::*;
use http::StatusCode;

/// SCADA-16 S14: saving fixed source kept the old `syntax_error`, so the
/// script was still refused by the test route and skipped by flow nodes.
#[tokio::test]
async fn test_update_script_clears_syntax_error_when_source_is_fixed() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/scripts",
        None,
        Some(&cookie),
        Some(serde_json::json!({"name": "s", "source": "function ("})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        created["syntax_error"].is_string(),
        "broken source is flagged"
    );
    let id = created["id"].as_i64().unwrap();
    let uri = format!("/api/v1/scripts/{id}");

    // Updating without a source leaves the flag alone.
    let (status, renamed) = send_request(
        &app,
        "PUT",
        &uri,
        None,
        Some(&cookie),
        Some(serde_json::json!({"name": "renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(renamed["syntax_error"].is_string());

    // Fixed source clears it.
    let (status, fixed) = send_request(
        &app,
        "PUT",
        &uri,
        None,
        Some(&cookie),
        Some(serde_json::json!({"source": "1 + 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(fixed["syntax_error"].is_null(), "got {fixed}");

    // Breaking it again sets it again.
    let (_, broken) = send_request(
        &app,
        "PUT",
        &uri,
        None,
        Some(&cookie),
        Some(serde_json::json!({"source": "function ("})),
    )
    .await;
    assert!(broken["syntax_error"].is_string());
}
