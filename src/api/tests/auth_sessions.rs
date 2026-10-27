use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

/// Log in and return the `cookie` header value carrying both the access
/// and the refresh token.
async fn login(app: &Router, username: &str, password: &str) -> String {
    let body = serde_json::json!({"username": username, "password": password});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let access = extract_cookie_value(resp.headers(), "voltrus_token").unwrap();
    let refresh = extract_cookie_value(resp.headers(), "voltrus_refresh").unwrap();
    format!("voltrus_token={access}; voltrus_refresh={refresh}")
}

/// Admin creates a second admin `ops`, who logs in. Returns (app, admin, ops id, ops cookies).
async fn setup_second_admin() -> (Router, String, i64, String) {
    let app = test_app_with_seeded_admin().await;
    let admin = login(&app, "admin", "admin").await;
    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/users",
        None,
        Some(&admin),
        Some(serde_json::json!({"username": "ops", "password": "pw1", "role": "admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ops = login(&app, "ops", "pw1").await;
    let (status, _) = send_request(&app, "GET", "/api/v1/users", None, Some(&ops), None).await;
    assert_eq!(status, StatusCode::OK, "ops starts as admin");
    (app, admin, created["id"].as_i64().unwrap(), ops)
}

async fn refresh_status(app: &Router, cookies: &str) -> StatusCode {
    send_request(
        app,
        "POST",
        "/api/v1/auth/refresh",
        None,
        Some(cookies),
        None,
    )
    .await
    .0
}

/// SCADA-15 S16: the role signed into the access token kept working after a
/// demotion, and the refresh token kept minting new sessions.
#[tokio::test]
async fn test_role_change_takes_effect_on_open_session() {
    let (app, admin, ops_id, ops) = setup_second_admin().await;
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{ops_id}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"role": "viewer"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send_request(&app, "GET", "/api/v1/users", None, Some(&ops), None).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "old admin token no longer admin"
    );
    let (_, me) = send_request(&app, "GET", "/api/v1/auth/me", None, Some(&ops), None).await;
    assert_eq!(me["role"], "viewer");
    assert_eq!(refresh_status(&app, &ops).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_password_change_revokes_refresh_tokens() {
    let (app, admin, ops_id, ops) = setup_second_admin().await;
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{ops_id}"),
        None,
        Some(&admin),
        Some(serde_json::json!({"password": "pw2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(refresh_status(&app, &ops).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_deleted_user_session_ends() {
    let (app, admin, ops_id, ops) = setup_second_admin().await;
    let (status, _) = send_request(
        &app,
        "DELETE",
        &format!("/api/v1/users/{ops_id}"),
        None,
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send_request(&app, "GET", "/api/v1/users", None, Some(&ops), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(refresh_status(&app, &ops).await, StatusCode::UNAUTHORIZED);
}

/// SCADA-16 S6: signing in on a second browser or device used to revoke every
/// refresh token of the user, ending the first session at its next renewal.
#[tokio::test]
async fn test_second_login_keeps_other_session_refreshable() {
    let app = test_app_with_seeded_admin().await;
    let first = login(&app, "admin", "admin").await;
    let second = login(&app, "admin", "admin").await;

    assert_eq!(refresh_status(&app, &first).await, StatusCode::OK);
    assert_eq!(refresh_status(&app, &second).await, StatusCode::OK);
}

/// Signing in again from a client that still holds a refresh token replaces
/// that token (it must not stay usable next to the new one).
#[tokio::test]
async fn test_login_with_presented_refresh_token_revokes_it() {
    let app = test_app_with_seeded_admin().await;
    let first = login(&app, "admin", "admin").await;
    let other = login(&app, "admin", "admin").await;

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        Some(&first),
        Some(serde_json::json!({"username": "admin", "password": "admin"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(refresh_status(&app, &first).await, StatusCode::UNAUTHORIZED);
    assert_eq!(refresh_status(&app, &other).await, StatusCode::OK);
}
