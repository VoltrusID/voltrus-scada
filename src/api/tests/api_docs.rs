//! The interactive API reference ships in default builds and needs a sign-in.
#![cfg(feature = "openapi")]

use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

async fn get(app: &Router, uri: &str, cookie: Option<&str>) -> http::Response<Body> {
    let mut req = Request::builder().method("GET").uri(uri);
    if let Some(c) = cookie {
        req = req.header("cookie", c);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn spec_and_docs_require_authentication() {
    let app = test_app_with_seeded_admin().await;
    for uri in ["/api/v1/openapi.json", "/api/docs"] {
        let resp = get(&app, uri, None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn signed_in_user_gets_the_spec_and_the_scalar_page() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let resp = get(&app, "/api/v1/openapi.json", Some(&cookie)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let spec: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(spec["openapi"].as_str().unwrap().starts_with("3.1"));
    assert!(spec["paths"]["/api/v1/tags/{id}/write"].is_object());
    assert!(
        spec["paths"]["/api/v1/stream"].is_object(),
        "the SSE stream is documented"
    );

    let resp = get(&app, "/api/docs", Some(&cookie)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    // The page loads Scalar from jsDelivr, which the default policy would block.
    let csp = resp.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("script-src 'self' https://cdn.jsdelivr.net"),
        "{csp}"
    );
    let html = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("api-reference") && html.contains("/api/v1/openapi.json"));

    // Everything else keeps the strict policy.
    let resp = get(&app, "/api/v1/health", None).await;
    let csp = resp.headers()["content-security-policy"].to_str().unwrap();
    assert!(!csp.contains("jsdelivr"), "{csp}");
}
