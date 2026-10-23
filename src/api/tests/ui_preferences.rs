use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_get_ui_theme_default() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/ui-theme")
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
    assert_eq!(json["theme"], "dark");
}

#[tokio::test]
async fn test_update_ui_theme() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    // Update to light
    let update_body = serde_json::json!({"theme": "light"});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings/ui-theme")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie))
                .body(Body::from(serde_json::to_string(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify persisted
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/ui-theme")
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
    assert_eq!(json["theme"], "light");
}

#[tokio::test]
async fn test_update_ui_theme_rejects_invalid() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let update_body = serde_json::json!({"theme": "neon"});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings/ui-theme")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie))
                .body(Body::from(serde_json::to_string(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_get_ui_lang_default() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/ui-lang")
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
    assert_eq!(json["lang"], "en");
}

#[tokio::test]
async fn test_update_ui_lang() {
    let app = test_app_with_seeded_admin().await;
    let cookie = seed_and_get_cookie(&app).await;

    // Update to zh
    let update_body = serde_json::json!({"lang": "zh"});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/settings/ui-lang")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie))
                .body(Body::from(serde_json::to_string(&update_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify persisted
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/ui-lang")
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
    assert_eq!(json["lang"], "zh");
}

#[tokio::test]
async fn test_ui_theme_requires_auth() {
    let app = test_app_with_seeded_admin().await;

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/settings/ui-theme")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
