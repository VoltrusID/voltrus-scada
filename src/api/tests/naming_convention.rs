use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

use super::*;

#[tokio::test]
async fn test_get_naming_convention_returns_ok() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/naming-convention")
                .header("cookie", format!("voltrus_token={}", cookie))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["template"], "permissive");
    assert!(json["templates"].is_array());
}

#[tokio::test]
async fn test_update_naming_convention_requires_admin() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings/naming-convention")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"template":"iec-61346"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_create_data_source_validates_naming_convention() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    // Default is permissive: any reasonable name passes
    let body = serde_json::json!({
        "name": "PT001_TEMP",
        "key": "pt001_temp",
        "unit": "C",
        "device_id": "default",
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/data-sources")
                .header("cookie", format!("voltrus_token={}", cookie))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // Permissive default: even informal names pass
    let body2 = serde_json::json!({
        "name": "my_sensor",
        "key": "my_sensor2",
        "unit": "C",
        "device_id": "default",
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/data-sources")
                .header("cookie", format!("voltrus_token={}", cookie))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&body2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    // Permissive default accepts any reasonable name
    assert_eq!(resp.status(), StatusCode::CREATED);
}
