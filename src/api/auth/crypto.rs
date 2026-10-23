pub fn hash_password(password: &str) -> String {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use rand::Rng;
    let salt_bytes: [u8; 32] = rand::rng().random();
    let salt = SaltString::encode_b64(&salt_bytes).unwrap();
    let params = argon2::Params::new(19456, 2, 1, None).unwrap();
    let hasher = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let hash = hasher.hash_password(password.as_bytes(), &salt).unwrap();
    hash.to_string()
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::password_hash::PasswordVerifier;
    let parsed = argon2::PasswordHash::new(hash).ok();
    match parsed {
        Some(parsed) => argon2::Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        None => false,
    }
}

pub(crate) fn constant_time_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut r = 0u8;
    for (ca, cb) in a.bytes().zip(b.bytes()) {
        r |= ca ^ cb;
    }
    r == 0
}

pub(crate) fn hmac_sign(payload: &str, secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
