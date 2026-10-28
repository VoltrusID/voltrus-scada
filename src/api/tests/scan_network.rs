use super::*;
use http::StatusCode;
use serde_json::json;

async fn scan(range: &str) -> (StatusCode, serde_json::Value) {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    send_request(
        &app,
        "POST",
        "/api/v1/modbus-devices/scan-network",
        None,
        Some(&cookie),
        Some(json!({"range": range, "port": 1})),
    )
    .await
}

#[tokio::test]
async fn oversized_range_is_a_400_that_says_why() {
    let (status, body) = scan("10.0.0.0/16").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let message = body["error"].as_str().unwrap();
    assert!(
        message.contains("/24") && message.contains("65534"),
        "{message}"
    );
}

#[tokio::test]
async fn malformed_or_empty_range_is_a_400() {
    for range in ["", "  ", "not-a-cidr", "10.0.0.0/33", "10.0.0.0"] {
        let (status, body) = scan(range).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{range:?}: {body}");
    }
}

#[tokio::test]
async fn range_with_nothing_listening_finds_no_devices() {
    let (status, body) = scan("127.0.0.1/32").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 0);
    assert_eq!(body["devices"], json!([]));
}
