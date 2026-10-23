use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_tag_tree_requires_auth() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags/tree")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_tag_tree_returns_empty_for_no_tags() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/tags/tree")
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
    assert_eq!(json["tree"], serde_json::json!([]));
}

#[tokio::test]
async fn test_update_tag_path_requires_admin() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/tags/1/path")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"path":"Site/Area"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
