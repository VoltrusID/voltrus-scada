use super::Storage;
use anyhow::{Context, Result};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ApiKeyInfo {
    pub id: i64,
    pub name: String,
    pub key_prefix: String,
    pub created_at: i64,
    pub created_by: String,
    pub last_used_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub permissions: String,
    pub role: String,
}

impl Storage {
    /// Create a new API key. Returns (id, plain_text_key). The plain key is shown only once.
    /// `role` is "api" (read-only) or "integration" (also allowed on operational write routes).
    pub async fn create_api_key(
        &self,
        name: &str,
        created_by: &str,
        expires_at: Option<i64>,
        permissions: &str,
        role: &str,
    ) -> Result<(i64, String)> {
        use rand::Rng;
        use sha2::{Digest, Sha256};

        // Generate 32 random bytes, hex-encoded = 64-char key
        let key_bytes: [u8; 32] = rand::rng().random();
        let plain_key = hex::encode(key_bytes);

        // Hash with SHA-256 for storage
        let mut hasher = Sha256::new();
        hasher.update(plain_key.as_bytes());
        let key_hash = hex::encode(hasher.finalize());

        // Prefix is first 8 chars of the plain key
        let key_prefix = plain_key[..8].to_string();

        let now = crate::timeutil::now_secs();
        let name = name.to_string();
        let created_by = created_by.to_string();
        let permissions = permissions.to_string();
        let role = role.to_string();

        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO api_keys (name, key_hash, key_prefix, created_at, created_by, expires_at, permissions, role)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![name, key_hash, key_prefix, now, created_by, expires_at, permissions, role],
            ).context("Insert api_key failed")?;
            let id = conn.last_insert_rowid();
            Ok((id, plain_key))
        }).await.context("Task join failed")?
    }

    /// Validate an API key by hashing it and looking it up.
    /// Returns (id, permissions, role) if valid.
    pub async fn validate_api_key(&self, key: &str) -> Result<Option<(i64, String, String)>> {
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        let key_hash = hex::encode(hasher.finalize());

        let now = crate::timeutil::now_secs();
        let conn = self.conn.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT id, permissions, expires_at, role FROM api_keys WHERE key_hash = ?1",
                )
                .context("Prepare failed")?;
            let result = stmt
                .query_row(rusqlite::params![key_hash], |row| {
                    let id: i64 = row.get(0)?;
                    let permissions: String = row.get(1)?;
                    let expires_at: Option<i64> = row.get(2)?;
                    let role: String = row.get(3)?;
                    Ok((id, permissions, expires_at, role))
                })
                .ok();

            match result {
                Some((id, permissions, expires_at, role)) => {
                    // Check expiry
                    if let Some(exp) = expires_at
                        && exp < now
                    {
                        return Ok(None);
                    }
                    // Update last_used_at
                    let _ = conn.execute(
                        "UPDATE api_keys SET last_used_at = ?1 WHERE id = ?2",
                        rusqlite::params![now, id],
                    );
                    Ok(Some((id, permissions, role)))
                }
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    /// List all API keys (never includes the full key hash).
    pub async fn list_api_keys(&self) -> Result<Vec<ApiKeyInfo>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, name, key_prefix, created_at, created_by, last_used_at, expires_at, permissions, role
                 FROM api_keys ORDER BY created_at DESC"
            ).context("Prepare failed")?;
            let rows = stmt.query_map([], |row| {
                Ok(ApiKeyInfo {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    key_prefix: row.get(2)?,
                    created_at: row.get(3)?,
                    created_by: row.get(4)?,
                    last_used_at: row.get(5)?,
                    expires_at: row.get(6)?,
                    permissions: row.get(7)?,
                    role: row.get(8)?,
                })
            }).context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>().context("Row mapping failed")
        }).await.context("Task join failed")?
    }

    /// Revoke (delete) an API key by id.
    pub async fn revoke_api_key(&self, id: i64) -> Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let affected = conn
                .execute("DELETE FROM api_keys WHERE id = ?1", rusqlite::params![id])
                .context("Delete api_key failed")?;
            Ok(affected > 0)
        })
        .await
        .context("Task join failed")?
    }
}
