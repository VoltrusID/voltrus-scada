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
