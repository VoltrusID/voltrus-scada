//! Failed sign-ins lock an (account, address) pair, an address that sprays
//! many accounts, and slow down new addresses when one account is attacked
//! from many: never the account for everyone (storage/login_failures.rs).

use super::*;
use crate::storage::{
    ACCOUNT_MAX_FAILURES, PAIR_MAX_FAILURES, SIGNATURE_SOURCE, SOURCE_MAX_FAILURES,
};
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

const OPS_PW: &str = "ops-password-1";

async fn state_with_ops() -> AppState {
    let state = seeded_admin_app_state().await;
    state
        .storage
        .create_user(
            "ops",
            &crate::api::auth::hash_password(OPS_PW),
            "",
            "operator",
        )
        .await
        .unwrap();
    state
}

/// POST /auth/login from `ip`: the status, the body and the access cookie.
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
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        cookie,
    )
}

/// `n` wrong passwords for `username` from `ip`, counted straight into the
/// database (over HTTP the in-memory per-minute limits would answer 429 long
/// before the database limits are reached).
async fn fail(state: &AppState, username: Option<&str>, ip: &str, n: i64) {
    let now = crate::timeutil::now_secs();
    for _ in 0..n {
        state
            .storage
            .record_login_failure(username, ip, true, now)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn an_attacker_address_is_locked_while_the_user_signs_in_from_another() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let attacker = "203.0.113.66";

    fail(&state, Some("ops"), attacker, PAIR_MAX_FAILURES - 1).await;
    let (status, body, _) = login(&app, "ops", "wrong", attacker).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    // Locked for the attacker's address, even with the right password.
    let (status, body, _) = login(&app, "ops", OPS_PW, attacker).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "Account locked. Please try again later.");

    // The operator at their station is not affected.
    let (status, body, _) = login(&app, "ops", OPS_PW, "10.20.0.5").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // Nor is another account from the attacker's address.
    let (status, body, _) = login(&app, "admin", "admin", attacker).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn spraying_many_accounts_from_one_address_is_throttled() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let sprayer = "203.0.113.77";

    // Unknown and real usernames alike count for the address; no single
    // account gets near its own limit.
    fail(&state, None, sprayer, SOURCE_MAX_FAILURES - 2).await;
    fail(&state, Some("admin"), sprayer, 1).await;
    let (status, body, _) = login(&app, "ops", "wrong", sprayer).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert!(state.storage.login_failure_count("ops", sprayer).await < PAIR_MAX_FAILURES);

    // The address is shut out of every account, right password or not.
    for (user, pw) in [("admin", "admin"), ("ops", OPS_PW), ("nobody", "x")] {
        let (status, body, _) = login(&app, user, pw, sprayer).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{user}: {body}");
    }
    // Everyone else signs in.
    let (status, body, _) = login(&app, "ops", OPS_PW, "10.20.0.6").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_successful_sign_in_clears_that_address_count() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let ip = "10.20.0.7";
    fail(&state, Some("ops"), ip, PAIR_MAX_FAILURES - 1).await;
    let (status, body, _) = login(&app, "ops", OPS_PW, ip).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(state.storage.login_failure_count("ops", ip).await, 0);

    // One more typo does not lock: the count started over.
    let (status, body, _) = login(&app, "ops", "wrong", ip).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, body, _) = login(&app, "ops", OPS_PW, ip).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_distributed_attack_slows_new_addresses_but_not_known_ones() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let station = "10.20.0.8";
    // The operator has signed in from their station before.
    let (status, body, _) = login(&app, "ops", OPS_PW, station).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Many addresses, each below its own pair limit.
    let per_address = PAIR_MAX_FAILURES - 1;
    let addresses = (ACCOUNT_MAX_FAILURES + per_address - 1) / per_address;
    for i in 0..addresses {
        fail(&state, Some("ops"), &format!("198.51.100.{i}"), per_address).await;
    }

    // A new address has to wait, even with the right password...
    let (status, body, _) = login(&app, "ops", OPS_PW, "10.20.0.99").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    // ...the station the operator works at does not.
    let (status, body, _) = login(&app, "ops", OPS_PW, station).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn an_admin_password_reset_clears_every_lock_of_the_account() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let ip = "10.20.0.9";
    fail(&state, Some("ops"), ip, PAIR_MAX_FAILURES).await;
    for i in 0..(ACCOUNT_MAX_FAILURES + PAIR_MAX_FAILURES - 2) / (PAIR_MAX_FAILURES - 1) {
        fail(
            &state,
            Some("ops"),
            &format!("198.51.100.{i}"),
            PAIR_MAX_FAILURES - 1,
        )
        .await;
    }
    let (status, _, _) = login(&app, "ops", OPS_PW, ip).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = login(&app, "ops", OPS_PW, "10.20.0.10").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    let (_, _, admin) = login(&app, "admin", "admin", "10.0.0.1").await;
    let id = state
        .storage
        .get_user_by_username("ops")
        .await
        .unwrap()
        .unwrap()
        .id;
    let (status, body) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/users/{id}"),
        None,
        Some(&admin.unwrap()),
        Some(json!({"password": "new-ops-password-2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for from in [ip, "10.20.0.10"] {
        let (status, body, _) = login(&app, "ops", "new-ops-password-2", from).await;
        assert_eq!(status, StatusCode::OK, "{from}: {body}");
    }
}

#[tokio::test]
async fn wrong_passwords_never_lock_a_shared_account() {
    let mut state = state_with_ops().await;
    state.public_demo = Arc::new(crate::api::shared_accounts::PublicDemo {
        shared_accounts: vec!["ops".to_string()],
        login_hint: None,
    });
    let app = create_router(state.clone());
    // Counts from before the account was listed don't keep anyone out.
    fail(&state, Some("ops"), "10.30.0.1", PAIR_MAX_FAILURES).await;
    let counted = state.storage.login_failure_count("ops", "").await;
    for i in 0..12 {
        let (status, body, _) = login(&app, "ops", "wrong", &format!("10.31.0.{i}")).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "attempt {i}: {body}");
    }
    assert_eq!(state.storage.login_failure_count("ops", "").await, counted);
    let (status, body, _) = login(&app, "ops", OPS_PW, "10.30.0.1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["shared"], true);
}

#[tokio::test]
async fn an_expired_lock_needs_a_full_run_of_failures_to_lock_again() {
    let state = state_with_ops().await;
    let ip = "10.20.0.11";
    let now = crate::timeutil::now_secs();
    for _ in 0..PAIR_MAX_FAILURES {
        state
            .storage
            .record_login_failure(Some("ops"), ip, true, now - crate::storage::PAIR_LOCK_SECS)
            .await
            .unwrap();
    }
    // The lock has run out by now.
    assert_eq!(
        state
            .storage
            .login_block(Some("ops"), ip, now)
            .await
            .unwrap(),
        None
    );
    let outcome = state
        .storage
        .record_login_failure(Some("ops"), ip, true, now)
        .await
        .unwrap();
    assert_eq!(outcome.pair_failures, 1);
    assert!(!outcome.pair_locked);
}

#[tokio::test]
async fn signature_failures_do_not_lock_sign_in() {
    let state = state_with_ops().await;
    let app = create_router(state.clone());
    let now = crate::timeutil::now_secs();
    for _ in 0..PAIR_MAX_FAILURES {
        state
            .storage
            .record_login_failure(Some("ops"), SIGNATURE_SOURCE, false, now)
            .await
            .unwrap();
    }
    assert!(
        state
            .storage
            .login_pair_locked_until("ops", SIGNATURE_SOURCE, now)
            .await
            .unwrap()
            .is_some()
    );
    let (status, body, _) = login(&app, "ops", OPS_PW, "10.20.0.12").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
