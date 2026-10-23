use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

use crate::api::tests::*;

#[tokio::test]
async fn test_audit_log_requires_auth() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/audit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_audit_export_requires_auth() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/audit/export")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_audit_log_returns_empty() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/audit?page=1&limit=10")
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
    assert_eq!(json["entries"], serde_json::json!([]));
    assert_eq!(json["total"], 0);
}
