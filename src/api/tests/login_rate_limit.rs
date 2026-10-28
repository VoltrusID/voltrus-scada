use super::*;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::{Request, StatusCode};
use std::net::SocketAddr;
use tower::ServiceExt;

async fn login_from(
    app: &Router,
    peer: &str,
    xff: Option<&str>,
    username: &str,
    password: &str,
) -> StatusCode {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json");
    if let Some(xff) = xff {
        req = req.header("x-forwarded-for", xff);
    }
    let mut req = req
        .body(Body::from(
            json!({"username": username, "password": password}).to_string(),
        ))
        .unwrap();
    let addr: SocketAddr = format!("{peer}:40000").parse().unwrap();
    req.extensions_mut().insert(ConnectInfo(addr));
    app.clone().oneshot(req).await.unwrap().status()
}

use serde_json::json;

/// Distinct usernames keep the per-username limit out of the way so only the
/// per-IP limit can produce the 429.
async fn fail_n_times(app: &Router, peer: &str, xff_prefix: Option<&str>, n: u32) {
    for i in 0..n {
        let xff = xff_prefix.map(|p| format!("{p}.{i}"));
        let status = login_from(app, peer, xff.as_deref(), &format!("ghost{i}"), "x").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {i}");
    }
}

#[tokio::test]
async fn ip_limit_blocks_one_ip_but_not_another() {
    let app = create_router(seeded_admin_app_state().await);
    fail_n_times(&app, "203.0.113.10", None, crate::security::LOGIN_MAX_IP_ATTEMPTS).await;

    // Even correct credentials are refused from the exhausted IP.
    assert_eq!(
        login_from(&app, "203.0.113.10", None, "admin", "admin").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        login_from(&app, "198.51.100.7", None, "admin", "admin").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn spoofed_forwarded_for_from_remote_peer_does_not_evade_limit() {
    let app = create_router(seeded_admin_app_state().await);
    // Each attempt claims a fresh client address; the real peer is what counts.
    fail_n_times(
        &app,
        "203.0.113.20",
        Some("192.0.2"),
        crate::security::LOGIN_MAX_IP_ATTEMPTS,
    )
    .await;
    assert_eq!(
        login_from(&app, "203.0.113.20", Some("192.0.2.200"), "admin", "admin").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn loopback_proxy_forwarded_ip_is_limited_per_real_client() {
    let app = create_router(seeded_admin_app_state().await);
    let n = crate::security::LOGIN_MAX_IP_ATTEMPTS;
    for i in 0..n {
        let s = login_from(&app, "127.0.0.1", Some("198.51.100.30"), &format!("g{i}"), "x").await;
        assert_eq!(s, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(
        login_from(&app, "127.0.0.1", Some("198.51.100.30"), "admin", "admin").await,
        StatusCode::TOO_MANY_REQUESTS
    );
    // A different client behind the same proxy is unaffected.
    assert_eq!(
        login_from(&app, "127.0.0.1", Some("198.51.100.31"), "admin", "admin").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn successful_logins_do_not_consume_the_ip_budget() {
    let app = create_router(seeded_admin_app_state().await);
    for _ in 0..3 {
        // Stay below the per-username limit too: it counts failures only.
        assert_eq!(
            login_from(&app, "203.0.113.40", None, "admin", "admin").await,
            StatusCode::OK
        );
    }
    fail_n_times(&app, "203.0.113.40", None, crate::security::LOGIN_MAX_IP_ATTEMPTS).await;
}
