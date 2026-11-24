//! `shared_accounts` and `login_hint`: one read-only login shared with the
//! public on a demo server.

use super::*;
use crate::api::shared_accounts::{PublicDemo, check_no_shared_admins};
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

const GUEST_PW: &str = "voltrus-demo";

/// Seeded admin plus `guest` (viewer, shared) and `ops` (viewer, not shared).
async fn demo_state(shared: &[&str], hint: Option<&str>) -> AppState {
    let mut state = seeded_admin_app_state().await;
    for name in ["guest", "ops"] {
        state
            .storage
            .create_user(
                name,
                &crate::api::auth::hash_password(GUEST_PW),
                "",
                "viewer",
            )
            .await
            .unwrap();
    }
    state.public_demo = Arc::new(PublicDemo {
        shared_accounts: shared.iter().map(|s| s.to_string()).collect(),
        login_hint: hint.map(str::to_string),
    });
    state
}

/// POST /auth/login from `ip`; returns the status, the body and the
/// access-token cookie (on success).
async fn login(
    app: &Router,
    username: &str,
    password: &str,
    ip: &str,
) -> (StatusCode, Value, Option<String>) {
    let mut req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"username": username, "password": password}).to_string(),
        ))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        format!("{ip}:40000")
            .parse::<std::net::SocketAddr>()
            .unwrap(),
    ));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let cookie =
        extract_cookie_value(resp.headers(), "voltrus_token").map(|t| format!("voltrus_token={t}"));
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body, cookie)
}

async fn signed_in(app: &Router, username: &str, password: &str) -> String {
    let (status, body, cookie) = login(app, username, password, "10.0.0.1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    cookie.unwrap()
}

async fn change_password(app: &Router, cookie: &str, old: &str) -> (StatusCode, Value) {
    send_request(
        app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(cookie),
        Some(json!({"old_password": old, "new_password": "something-new-123"})),
    )
    .await
}

async fn failed_attempts(state: &AppState, username: &str) -> i64 {
    state
        .storage
        .get_user_by_username(username)
        .await
        .unwrap()
        .unwrap()
        .failed_attempts
}

#[tokio::test]
async fn shared_account_cannot_change_its_password() {
    let app = create_router(demo_state(&["guest"], None).await);
    let guest = signed_in(&app, "guest", GUEST_PW).await;
    let (status, body) = change_password(&app, &guest, GUEST_PW).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        body["error"],
        "This is a shared demo account; its password can't be changed"
    );
    // The old password still works.
    signed_in(&app, "guest", GUEST_PW).await;

    // Another viewer is not affected.
    let ops = signed_in(&app, "ops", GUEST_PW).await;
    let (status, body) = change_password(&app, &ops, GUEST_PW).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn shared_account_cannot_change_its_preferences() {
    let app = create_router(demo_state(&["guest"], None).await);
    let guest = signed_in(&app, "guest", GUEST_PW).await;
    let ops = signed_in(&app, "ops", GUEST_PW).await;
    let writes = [
        ("/api/v1/settings/ui-theme", json!({"theme": "light"})),
        ("/api/v1/settings/ui-lang", json!({"lang": "de"})),
        ("/api/v1/shortcuts", json!({})),
        ("/api/v1/nav-shortcuts", json!({})),
    ];
    for (uri, body) in &writes {
        let (status, resp) =
            send_request(&app, "PUT", uri, None, Some(&guest), Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {resp}");
        assert!(
            resp["error"]
                .as_str()
                .unwrap()
                .contains("shared demo account"),
            "{resp}"
        );
        let (status, resp) =
            send_request(&app, "PUT", uri, None, Some(&ops), Some(body.clone())).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {resp}");
    }
    // Reading them still works.
    let (status, theme) = send_request(
        &app,
        "GET",
        "/api/v1/settings/ui-theme",
        None,
        Some(&guest),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(theme["theme"], "dark");
}

#[tokio::test]
async fn wrong_passwords_never_lock_a_shared_account() {
    let state = demo_state(&["guest"], None).await;
    let app = create_router(state.clone());

    // Twelve wrong passwords from different addresses: more than the lockout
    // threshold, without tripping the per-address limits.
    for i in 0..12 {
        let (status, body, _) = login(&app, "guest", "wrong", &format!("10.1.0.{i}")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {i}: {body}");
    }
    assert_eq!(failed_attempts(&state, "guest").await, 0);

    // Counts left over from before the account was listed, or a lock, don't
    // keep anyone out either.
    for _ in 0..9 {
        state
            .storage
            .increment_failed_attempts("guest")
            .await
            .unwrap();
    }
    let (status, _, _) = login(&app, "guest", "wrong", "10.2.0.1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    state
        .storage
        .lock_account("guest", crate::timeutil::now_secs() + 900)
        .await
        .unwrap();
    let (status, body, _) = login(&app, "guest", GUEST_PW, "10.2.0.2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shared"], true);
}

#[tokio::test]
async fn one_address_guessing_a_shared_account_does_not_block_others() {
    let app = create_router(demo_state(&["guest"], None).await);
    let mut limited = false;
    for _ in 0..6 {
        let (status, _, _) = login(&app, "guest", "wrong", "10.3.0.1").await;
        limited |= status == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited, "the guessing address is rate limited");
    let (status, body, _) = login(&app, "guest", GUEST_PW, "10.3.0.2").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_normal_account_still_locks_after_ten_failures() {
    let state = demo_state(&["guest"], None).await;
    let app = create_router(state.clone());
    // The per-username limit allows five tries a minute, so put the account at
    // nine failures directly and let the tenth come in over HTTP.
    for _ in 0..9 {
        state
            .storage
            .increment_failed_attempts("ops")
            .await
            .unwrap();
    }
    let (status, _, _) = login(&app, "ops", "wrong", "10.4.0.1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // Locking starts the count over.
    assert_eq!(failed_attempts(&state, "ops").await, 0);
    let (status, body, _) = login(&app, "ops", GUEST_PW, "10.4.0.2").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "Account locked. Please try again later.");
}

/// Once a lock has run out, one wrong password does not lock the account
/// again: it takes another ten.
#[tokio::test]
async fn an_expired_lock_needs_ten_new_failures_to_lock_again() {
    let state = demo_state(&[], None).await;
    let app = create_router(state.clone());
    for _ in 0..9 {
        state
            .storage
            .increment_failed_attempts("ops")
            .await
            .unwrap();
    }
    let (status, _, _) = login(&app, "ops", "wrong", "10.5.0.1").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The lock runs out.
    state
        .storage
        .lock_account("ops", crate::timeutil::now_secs() - 1)
        .await
        .unwrap();
    let (status, _, _) = login(&app, "ops", "wrong", "10.5.0.2").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(failed_attempts(&state, "ops").await, 1);
    let (status, body, _) = login(&app, "ops", GUEST_PW, "10.5.0.3").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn session_reports_a_shared_account_and_never_asks_for_a_new_password() {
    let state = demo_state(&["guest"], None).await;
    state
        .storage
        .set_must_change_password("guest", true)
        .await
        .unwrap();
    let app = create_router(state);
    let (status, body, cookie) = login(&app, "guest", GUEST_PW, "10.5.0.1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shared"], true);
    assert_eq!(body["must_change_password"], false);
    let (_, me) = send_request(
        &app,
        "GET",
        "/api/v1/auth/me",
        None,
        Some(&cookie.unwrap()),
        None,
    )
    .await;
    assert_eq!(me["shared"], true);

    let ops = signed_in(&app, "ops", GUEST_PW).await;
    let (_, me) = send_request(&app, "GET", "/api/v1/auth/me", None, Some(&ops), None).await;
    assert_eq!(me["shared"], false);
}

#[tokio::test]
async fn login_hint_is_on_the_public_providers_endpoint() {
    let hint = "Demo: sign in as guest / voltrus-demo";
    let app = create_router(demo_state(&["guest"], Some(hint)).await);
    let (status, body) =
        send_request(&app, "GET", "/api/v1/auth/providers", None, None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["login_hint"], hint);

    let app = create_router(seeded_admin_app_state().await);
    let (_, body) = send_request(&app, "GET", "/api/v1/auth/providers", None, None, None).await;
    assert_eq!(body["login_hint"], Value::Null);
}

#[tokio::test]
async fn an_admin_in_shared_accounts_stops_the_server_and_is_never_shared() {
    let state = demo_state(&["guest", "admin"], None).await;
    let err = check_no_shared_admins(&state.storage, &state.public_demo)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("'admin'"), "{err}");
    assert!(err.contains("admin account"), "{err}");

    // A viewer, or an account that does not exist yet, is fine.
    let ok = demo_state(&["guest", "not-created-yet"], None).await;
    check_no_shared_admins(&ok.storage, &ok.public_demo)
        .await
        .unwrap();

    // Should an admin be listed anyway (promoted after startup), it keeps the
    // normal rules: it can change its password and is reported as not shared.
    let app = create_router(state);
    let admin = signed_in(&app, "admin", "admin").await;
    let (_, me) = send_request(&app, "GET", "/api/v1/auth/me", None, Some(&admin), None).await;
    assert_eq!(me["shared"], false);
    let (status, body) = change_password(&app, &admin, "admin").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[test]
fn config_trims_and_limits_the_login_hint() {
    use std::io::Write;
    let load = |yaml: &str| {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        crate::config::load_config(f.path().to_str().unwrap())
    };

    let config =
        load("shared_accounts: [\" guest \"]\nlogin_hint: \"  Demo: guest / voltrus-demo \"\n")
            .unwrap();
    assert_eq!(config.shared_accounts, vec!["guest".to_string()]);
    assert_eq!(
        config.login_hint.as_deref(),
        Some("Demo: guest / voltrus-demo")
    );

    let config = load("login_hint: \"   \"\n").unwrap();
    assert_eq!(config.login_hint, None);
    assert!(load("").unwrap().shared_accounts.is_empty());

    let long = "x".repeat(201);
    let err = load(&format!("login_hint: \"{long}\"\n")).unwrap_err();
    assert!(format!("{err:#}").contains("at most 200"), "{err:#}");
    assert!(load(&format!("login_hint: \"{}\"\n", "x".repeat(200))).is_ok());
    assert!(load("login_hint: \"line one\\nline two\"\n").is_err());
    assert!(load("shared_accounts: [\"\"]\n").is_err());
}
