//! Encryption at rest for secrets the server must read back (an LLM API key):
//! AES-256-GCM under a key derived from `auth_secret` with HKDF-SHA256. The
//! stored form is `enc:v1:` + base64(nonce || ciphertext || tag); the purpose
//! string is both the HKDF info and the AEAD associated data, so a value
//! encrypted for one setting cannot be moved into another.
//!
//! Changing `auth_secret` makes stored secrets unreadable; they have to be
//! entered again.

use anyhow::{Result, anyhow};
use base64::Engine as _;
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use ring::hkdf;
use ring::rand::{SecureRandom, SystemRandom};

const PREFIX: &str = "enc:v1:";
const SALT: &[u8] = b"voltrus secrets at rest v1";

fn key(auth_secret: &str, purpose: &str) -> Result<LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, SALT).extract(auth_secret.as_bytes());
    let info = [purpose.as_bytes()];
    let okm = prk
        .expand(&info, &AES_256_GCM)
        .map_err(|_| anyhow!("key derivation failed"))?;
    Ok(LessSafeKey::new(UnboundKey::from(okm)))
}

/// Whether a stored value is in the encrypted form (as opposed to a legacy
/// plain-text value still waiting for its migration).
pub fn is_encrypted(stored: &str) -> bool {
    stored.starts_with(PREFIX)
}

pub fn encrypt_secret(auth_secret: &str, purpose: &str, plaintext: &str) -> Result<String> {
    let key = key(auth_secret, purpose)?;
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow!("no randomness for the nonce"))?;
    let mut data = plaintext.as_bytes().to_vec();
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(purpose.as_bytes()),
        &mut data,
    )
    .map_err(|_| anyhow!("encryption failed"))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&data);
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(out)
    ))
}

pub fn decrypt_secret(auth_secret: &str, purpose: &str, stored: &str) -> Result<String> {
    let encoded = stored
        .strip_prefix(PREFIX)
        .ok_or_else(|| anyhow!("value is not encrypted"))?;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("encrypted value is corrupt"))?;
    if raw.len() < NONCE_LEN {
        return Err(anyhow!("encrypted value is corrupt"));
    }
    let (nonce, sealed) = raw.split_at(NONCE_LEN);
    let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| anyhow!("bad nonce"))?;
    let mut sealed = sealed.to_vec();
    let plain = key(auth_secret, purpose)?
        .open_in_place(nonce, Aad::from(purpose.as_bytes()), &mut sealed)
        .map_err(|_| anyhow!("cannot decrypt: auth_secret changed or value tampered with"))?;
    String::from_utf8(plain.to_vec()).map_err(|_| anyhow!("decrypted value is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_binding() {
        let enc = encrypt_secret("s3cret-0123456789", "ai_api_key", "sk-test").unwrap();
        assert!(is_encrypted(&enc));
        assert!(!enc.contains("sk-test"));
        assert_eq!(
            decrypt_secret("s3cret-0123456789", "ai_api_key", &enc).unwrap(),
            "sk-test"
        );
        assert_ne!(
            enc,
            encrypt_secret("s3cret-0123456789", "ai_api_key", "sk-test").unwrap(),
            "a fresh nonce every time"
        );
        assert!(decrypt_secret("another-secret-xyz", "ai_api_key", &enc).is_err());
        assert!(decrypt_secret("s3cret-0123456789", "other", &enc).is_err());
        assert!(decrypt_secret("s3cret-0123456789", "ai_api_key", "sk-plain").is_err());
    }
}
