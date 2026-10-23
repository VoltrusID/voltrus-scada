use base64::Engine;
use rand::Rng;

use super::*;

// -- Refresh token helpers --

pub(crate) fn generate_refresh_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub(crate) fn hash_refresh_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn create_token(username: &str, role: &str, secret: &str) -> String {
    let exp = crate::timeutil::now_millis() as u64 + ACCESS_TOKEN_EXPIRY_MS;
    let payload = serde_json::to_string(&TokenClaims {
        username: username.to_string(),
        exp,
        role: role.to_string(),
    })
    .unwrap();
    let sig = hmac_sign(&payload, secret);
    format!(
        "{}.{}",
        base64::engine::general_purpose::STANDARD.encode(&payload),
        sig
    )
}

pub fn verify_token(token: &str, secret: &str) -> Option<TokenClaims> {
    let (payload_b64, sig) = token.split_once('.')?;
    let payload = String::from_utf8(
        base64::engine::general_purpose::STANDARD
            .decode(payload_b64)
            .ok()?,
    )
    .ok()?;
    // Verify HMAC signature
    let expected = hmac_sign(&payload, secret);
    if !constant_time_equal(sig, &expected) {
        return None;
    }
    let claims: TokenClaims = serde_json::from_str(&payload).ok()?;
    let now = crate::timeutil::now_millis() as u64;
    if claims.exp < now {
        return None;
    }
    Some(claims)
}
