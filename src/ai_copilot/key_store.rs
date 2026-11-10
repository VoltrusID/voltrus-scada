//! The copilot's LLM API key, encrypted at rest (see `crate::security::secrets`).

use anyhow::Result;

use crate::security::secrets::{decrypt_secret, encrypt_secret, is_encrypted};
use crate::storage::Storage;

/// Setting name (stored as `ai_api_key`) and the encryption purpose.
const SETTING: &str = "api_key";
const PURPOSE: &str = "ai_api_key";

/// What is stored for the key.
pub enum StoredKey {
    None,
    Readable(String),
    /// Present but cannot be decrypted (auth_secret changed): it must be
    /// entered again.
    Unreadable,
}

pub async fn load(storage: &Storage, auth_secret: &str) -> Result<StoredKey> {
    let Some(stored) = storage.get_ai_setting(SETTING).await? else {
        return Ok(StoredKey::None);
    };
    if !is_encrypted(&stored) {
        // Written before encryption and not migrated yet (migration runs at start).
        return Ok(StoredKey::Readable(stored));
    }
    Ok(match decrypt_secret(auth_secret, PURPOSE, &stored) {
        Ok(key) => StoredKey::Readable(key),
        Err(e) => {
            tracing::warn!(
                "AI copilot API key cannot be read ({e}); enter it again in the AI settings"
            );
            StoredKey::Unreadable
        }
    })
}

pub async fn save(storage: &Storage, auth_secret: &str, key: &str) -> Result<()> {
    let stored = encrypt_secret(auth_secret, PURPOSE, key)?;
    storage.set_ai_setting(SETTING, &stored).await
}

/// Encrypt a key stored in plain text by an older version. Returns whether
/// anything was rewritten.
pub async fn migrate_plaintext(storage: &Storage, auth_secret: &str) -> Result<bool> {
    match storage.get_ai_setting(SETTING).await? {
        Some(stored) if !is_encrypted(&stored) => {
            save(storage, auth_secret, &stored).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// A hint that identifies the key without revealing it: its last four
/// characters, and only for keys long enough that four characters say little.
pub fn masked_hint(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() > 12 {
        let last4: String = chars[chars.len() - 4..].iter().collect();
        format!("****{last4}")
    } else {
        "****".to_string()
    }
}
