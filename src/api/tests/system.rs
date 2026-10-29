use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_live_empty() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_health_open() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
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
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_license_tier_header_in_response() {
    let app = test_app();

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let tier_header = resp
        .headers()
        .get("x-license-tier")
        .expect("X-License-Tier header should be present");
    assert_eq!(tier_header, "community");
}

#[tokio::test]
async fn system_info_reports_process_memory_and_shares_one_refresh() {
    use crate::api::system_sampler::SystemSampler;
    use std::sync::Arc;
    use std::time::Duration;

    let state = seeded_admin_app_state().await;
    let sampler = Arc::new(SystemSampler::new(Duration::from_secs(60)));

    let calls: Vec<_> = (0..16)
        .map(|_| {
            let sampler = sampler.clone();
            let state = state.clone();
            tokio::spawn(async move {
                crate::api::system::compute_system_info_from(&sampler, &state)
                    .await
                    .unwrap()
            })
        })
        .collect();
    let mut responses = Vec::new();
    for call in calls {
        responses.push(call.await.unwrap());
    }

    assert_eq!(
        sampler.refresh_count(),
        1,
        "16 concurrent requests run one refresh"
    );
    let first = &responses[0];
    assert_eq!(first.process.pid, std::process::id());
    assert!(first.process.memory_mb > 1.0);
    assert!(
        first.process.memory_mb < first.memory_used_mb,
        "process memory {} is below machine-wide used memory {}",
        first.process.memory_mb,
        first.memory_used_mb
    );
    assert!(first.memory_total_mb > 0.0);
    assert!(!first.hostname.is_empty());
    assert!(first.process.uptime_secs < 24 * 3600);
}
