use super::*;
use base64::Engine;

#[test]
fn test_hash_and_verify_password() {
    let hash = hash_password("admin");
    assert!(verify_password("admin", &hash));
    assert!(!verify_password("wrong", &hash));
}

#[test]
fn test_hash_password_unique_salts() {
    let a = hash_password("admin");
    let b = hash_password("admin");
    assert_ne!(a, b); // different salts each time
}

#[test]
fn test_create_and_verify_token() {
    let secret = "test-secret";
    let token = create_token("admin", "admin", secret);
    let claims = verify_token(&token, secret).unwrap();
    assert_eq!(claims.username, "admin");
    assert_eq!(claims.role, "admin");
    assert!(claims.exp > crate::timeutil::now_millis() as u64);
}

#[test]
fn test_verify_token_wrong_secret() {
    let token = create_token("admin", "admin", "secret1");
    assert!(verify_token(&token, "secret2").is_none());
}

#[test]
fn test_verify_token_expired() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as ENCODING;
    let claims = TokenClaims {
        username: "admin".to_string(),
        exp: 0, // expired
        role: "admin".to_string(),
    };
    let payload = serde_json::to_string(&claims).unwrap();
    let sig = hmac_sign(&payload, "secret");
    let token = format!("{}.{}", ENCODING.encode(&payload), sig);
    assert!(verify_token(&token, "secret").is_none());
}

#[test]
fn test_verify_token_malformed() {
    assert!(verify_token("not-a-token", "secret").is_none());
    assert!(verify_token("", "secret").is_none());
}

#[test]
fn test_constant_time_equal() {
    assert!(constant_time_equal("abc", "abc"));
    assert!(!constant_time_equal("abc", "abd"));
    assert!(!constant_time_equal("abc", "ab"));
}

#[tokio::test]
async fn issue_session_sets_verifiable_cookies_and_rotates_only_the_presented_refresh_token() {
    use axum_extra::extract::CookieJar;
    use axum_extra::extract::cookie::Cookie;

    let storage = crate::storage::Storage::new_in_memory().unwrap();
    let secret = "session-secret";

    // A refresh token this browser already holds, and one from another device.
    let now = crate::timeutil::now_secs();
    storage
        .create_refresh_token("alice", &hash_refresh_token("mine"), now + 1000, now)
        .await
        .unwrap();
    storage
        .create_refresh_token(
            "alice",
            &hash_refresh_token("other-device"),
            now + 1000,
            now,
        )
        .await
        .unwrap();
    let jar = CookieJar::new().add(Cookie::new("voltrus_refresh", "mine"));

    let jar = issue_session(&storage, secret, true, jar, "alice", "operator").await;

    let access = jar.get("voltrus_token").unwrap();
    let claims = verify_token(access.value(), secret).unwrap();
    assert_eq!(
        (claims.username.as_str(), claims.role.as_str()),
        ("alice", "operator")
    );
    assert_eq!(access.http_only(), Some(true));
    assert_eq!(access.secure(), Some(true));

    let refresh = jar.get("voltrus_refresh").unwrap();
    assert_ne!(refresh.value(), "mine");
    let stored = storage
        .get_refresh_token_by_hash(&hash_refresh_token(refresh.value()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.username, "alice");
    assert!(stored.revoked_at.is_none());

    let old = storage
        .get_refresh_token_by_hash(&hash_refresh_token("mine"))
        .await
        .unwrap()
        .unwrap();
    assert!(old.revoked_at.is_some(), "the presented token is replaced");
    let other = storage
        .get_refresh_token_by_hash(&hash_refresh_token("other-device"))
        .await
        .unwrap()
        .unwrap();
    assert!(other.revoked_at.is_none(), "other sessions stay signed in");
}
