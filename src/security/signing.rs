#![allow(dead_code)]
// HMAC-SHA256 request signature validation.
// Prepared for outbound webhook response verification (not yet wired into the API).

use axum::extract::Request;

/// Validate an HMAC-SHA256 signature on a request.
/// Expected header: `X-Signature: <hex_hmac>`
/// The signature is computed over the request body using a shared secret.
pub fn validate_request_signature(req: &Request, body: &[u8], secret: &str) -> bool {
    let expected = req
        .headers()
        .get("X-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(body);
    let result = hex::encode(mac.finalize().into_bytes());

    // Constant-time comparison to prevent timing attacks
    if expected.len() != result.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(result.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}
