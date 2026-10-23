use super::*;
use axum::body::Body;
use http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn test_mqtt_broker_status_requires_auth() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/mqtt-broker/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(!resp.status().is_success());
}

#[tokio::test]
async fn test_mqtt_broker_start_requires_auth() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mqtt-broker/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(!resp.status().is_success());
}
