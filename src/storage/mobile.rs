//! Phones paired with the Voltrus SCADA app.
//!
//! A paired phone holds a device token (`vdt_` + 64 hex characters) and acts
//! as the user who paired it: the user's current role applies to every
//! request, and a disabled or deleted user's phones stop working at once.
//! Only the SHA-256 of a token or of a pairing code is stored. Removing the
//! row revokes the phone.

use super::Storage;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

/// Paired phones per site. The push relay enforces the same cap.
pub const MOBILE_DEVICE_LIMIT: i64 = 50;

/// Prefix of a device token, so the auth middleware can tell it from an
/// add-on edition's bearer token without a lookup.
pub const DEVICE_TOKEN_PREFIX: &str = "vdt_";

/// How often `last_seen_at` is written for a busy phone.
const LAST_SEEN_RESOLUTION_SECS: i64 = 60;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct MobileDevice {
    pub id: i64,
    pub username: String,
    pub name: String,
    /// `android` or `ios`.
    pub platform: String,
    /// The phone's push token (an Expo push token), `None` until the app
    /// registers one or when the user turned notifications off.
    #[serde(skip_serializing)]
    pub push_token: Option<String>,
    /// Whether the phone receives alarm pushes.
    #[serde(default)]
    pub push: bool,
    /// Lowest alarm priority pushed to the phone: `info`, `warning` or `critical`.
    pub min_priority: String,
    /// Language of the notifications: `id` or `en`.
    pub lang: String,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    /// Set for phones of a shared demo account, which expire on their own.
    pub expires_at: Option<i64>,
}

/// A phone being paired.
#[derive(Debug, Clone, Default)]
pub struct NewMobileDevice {
    pub username: String,
    pub name: String,
    pub platform: String,
    pub push_token: Option<String>,
    pub min_priority: String,
    pub lang: String,
    pub expires_at: Option<i64>,
}

/// Fields of a paired phone its owner may change. `None` leaves a field as it is;
/// `push_token: Some(None)` turns pushes off.
#[derive(Debug, Clone, Default)]
pub struct MobileDeviceUpdate {
    pub name: Option<String>,
    pub push_token: Option<Option<String>>,
    pub min_priority: Option<String>,
    pub lang: Option<String>,
}

pub(crate) fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn random_hex() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::rng().random();
    hex::encode(bytes)
}

const DEVICE_COLUMNS: &str = "id, username, name, platform, push_token, min_priority, lang, created_at, last_seen_at, expires_at";

fn device_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MobileDevice> {
    let push_token: Option<String> = row.get(4)?;
    Ok(MobileDevice {
        id: row.get(0)?,
        username: row.get(1)?,
        name: row.get(2)?,
        platform: row.get(3)?,
        push: push_token.is_some(),
        push_token,
        min_priority: row.get(5)?,
        lang: row.get(6)?,
        created_at: row.get(7)?,
        last_seen_at: row.get(8)?,
        expires_at: row.get(9)?,
    })
}

/// Why a phone could not be paired.
#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    /// The site already has [`MOBILE_DEVICE_LIMIT`] phones.
    LimitReached,
}

impl Storage {
    /// A one-time pairing code for `username`, valid for `ttl_secs`. Returns the
    /// plain code; only its hash is stored. Expired codes are dropped here.
    pub async fn create_mobile_pairing(&self, username: &str, ttl_secs: i64) -> Result<String> {
        let code = random_hex();
        let code_hash = sha256_hex(&code);
        let username = username.to_string();
        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM mobile_pairings WHERE expires_at < ?1",
                rusqlite::params![now],
            )
            .context("Purge mobile pairings failed")?;
            conn.execute(
                "INSERT INTO mobile_pairings (code_hash, username, created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![code_hash, username, now, now + ttl_secs],
            )
            .context("Insert mobile pairing failed")?;
            Ok(code)
        })
        .await
        .context("Task join failed")?
    }

    /// Use up a pairing code: the user it was made for, or `None` when it is
    /// unknown, used or expired. A code works once.
    pub async fn take_mobile_pairing(&self, code: &str) -> Result<Option<String>> {
        let code_hash = sha256_hex(code);
        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let row: Option<(String, i64)> = conn
                .query_row(
                    "DELETE FROM mobile_pairings WHERE code_hash = ?1 RETURNING username, expires_at",
                    rusqlite::params![code_hash],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .context("Take mobile pairing failed")?;
            Ok(row.and_then(|(username, expires_at)| (expires_at >= now).then_some(username)))
        })
        .await
        .context("Task join failed")?
    }

    /// Pair a phone. Returns the device and its plain token (shown once), or
    /// [`PairError::LimitReached`] when the site has no room. Expired demo
    /// phones are dropped first and do not count.
    pub async fn create_mobile_device(
        &self,
        device: NewMobileDevice,
    ) -> Result<std::result::Result<(MobileDevice, String), PairError>> {
        let token = format!("{DEVICE_TOKEN_PREFIX}{}", random_hex());
        let token_hash = sha256_hex(&token);
        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction().context("Begin failed")?;
            tx.execute(
                "DELETE FROM mobile_devices WHERE expires_at IS NOT NULL AND expires_at < ?1",
                rusqlite::params![now],
            )
            .context("Purge expired mobile devices failed")?;
            let count: i64 = tx
                .query_row("SELECT COUNT(*) FROM mobile_devices", [], |r| r.get(0))
                .context("Count mobile devices failed")?;
            if count >= MOBILE_DEVICE_LIMIT {
                return Ok(Err(PairError::LimitReached));
            }
            tx.execute(
                "INSERT INTO mobile_devices
                    (username, name, platform, token_hash, push_token, min_priority, lang, created_at, last_seen_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)",
                rusqlite::params![
                    device.username,
                    device.name,
                    device.platform,
                    token_hash,
                    device.push_token,
                    device.min_priority,
                    device.lang,
                    now,
                    device.expires_at,
                ],
            )
            .context("Insert mobile device failed")?;
            let id = tx.last_insert_rowid();
            let stored = tx
                .query_row(
                    &format!("SELECT {DEVICE_COLUMNS} FROM mobile_devices WHERE id = ?1"),
                    rusqlite::params![id],
                    device_from_row,
                )
                .context("Read mobile device failed")?;
            tx.commit().context("Commit failed")?;
            Ok(Ok((stored, token)))
        })
        .await
        .context("Task join failed")?
    }

    /// The phone a device token belongs to, if it is paired and not expired.
    /// Records when the phone was last seen (at most once a minute).
    pub async fn authenticate_mobile_device(&self, token: &str) -> Result<Option<MobileDevice>> {
        if !token.starts_with(DEVICE_TOKEN_PREFIX) {
            return Ok(None);
        }
        let token_hash = sha256_hex(token);
        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let device = conn
                .query_row(
                    &format!("SELECT {DEVICE_COLUMNS} FROM mobile_devices WHERE token_hash = ?1"),
                    rusqlite::params![token_hash],
                    device_from_row,
                )
                .optional()
                .context("Look up mobile device failed")?;
            let Some(mut device) = device else {
                return Ok(None);
            };
            if device.expires_at.is_some_and(|exp| exp < now) {
                return Ok(None);
            }
            if device
                .last_seen_at
                .is_none_or(|seen| now - seen >= LAST_SEEN_RESOLUTION_SECS)
            {
                let _ = conn.execute(
                    "UPDATE mobile_devices SET last_seen_at = ?1 WHERE id = ?2",
                    rusqlite::params![now, device.id],
                );
                device.last_seen_at = Some(now);
            }
            Ok(Some(device))
        })
        .await
        .context("Task join failed")?
    }

    /// Paired phones, newest first: of one user, or of everyone for `None`.
    pub async fn list_mobile_devices(&self, username: Option<&str>) -> Result<Vec<MobileDevice>> {
        let username = username.map(str::to_string);
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {DEVICE_COLUMNS} FROM mobile_devices
                     WHERE ?1 IS NULL OR username = ?1
                     ORDER BY created_at DESC, id DESC"
                ))
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![username], device_from_row)
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_mobile_device(&self, id: i64) -> Result<Option<MobileDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM mobile_devices WHERE id = ?1"),
                rusqlite::params![id],
                device_from_row,
            )
            .optional()
            .context("Read mobile device failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Change a phone's settings. Returns the updated phone, `None` if it is gone.
    pub async fn update_mobile_device(
        &self,
        id: i64,
        update: MobileDeviceUpdate,
    ) -> Result<Option<MobileDevice>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(name) = &update.name {
                conn.execute(
                    "UPDATE mobile_devices SET name = ?1 WHERE id = ?2",
                    rusqlite::params![name, id],
                )
                .context("Update mobile device failed")?;
            }
            if let Some(push_token) = &update.push_token {
                conn.execute(
                    "UPDATE mobile_devices SET push_token = ?1 WHERE id = ?2",
                    rusqlite::params![push_token, id],
                )
                .context("Update mobile device failed")?;
            }
            if let Some(min_priority) = &update.min_priority {
                conn.execute(
                    "UPDATE mobile_devices SET min_priority = ?1 WHERE id = ?2",
                    rusqlite::params![min_priority, id],
                )
                .context("Update mobile device failed")?;
            }
            if let Some(lang) = &update.lang {
                conn.execute(
                    "UPDATE mobile_devices SET lang = ?1 WHERE id = ?2",
                    rusqlite::params![lang, id],
                )
                .context("Update mobile device failed")?;
            }
            conn.query_row(
                &format!("SELECT {DEVICE_COLUMNS} FROM mobile_devices WHERE id = ?1"),
                rusqlite::params![id],
                device_from_row,
            )
            .optional()
            .context("Read mobile device failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Unpair a phone. Returns whether it existed.
    pub async fn delete_mobile_device(&self, id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let n = conn
                .execute(
                    "DELETE FROM mobile_devices WHERE id = ?1",
                    rusqlite::params![id],
                )
                .context("Delete mobile device failed")?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// Phones that receive pushes: a push token, not expired.
    pub async fn mobile_push_devices(&self) -> Result<Vec<MobileDevice>> {
        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {DEVICE_COLUMNS} FROM mobile_devices
                     WHERE push_token IS NOT NULL AND (expires_at IS NULL OR expires_at >= ?1)"
                ))
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![now], device_from_row)
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    /// Forget a push token the push service reported as no longer valid (the
    /// app was uninstalled). The phone stays paired.
    pub async fn clear_mobile_push_token(&self, id: i64, push_token: &str) -> Result<()> {
        let push_token = push_token.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE mobile_devices SET push_token = NULL WHERE id = ?1 AND push_token = ?2",
                rusqlite::params![id, push_token],
            )
            .context("Clear push token failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
