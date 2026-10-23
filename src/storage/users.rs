use super::Storage;
use anyhow::{Context, Result};

use crate::models::User;

impl Storage {
    // ── Users ──

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(failed_attempts, 0), locked_until FROM users WHERE username = ?1")
                .context("Prepare failed")?;
            let mut rows = stmt
                .query_map(rusqlite::params![username], |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        password_hash: row.get(2)?,
                        salt: row.get(3)?,
                        role: row.get(4)?,
                        must_change_password: row.get(5)?,
                        failed_attempts: row.get(6)?,
                        locked_until: row.get(7)?,
                    })
                })
                .context("Query failed")?;
            match rows.next() {
                Some(Ok(user)) => Ok(Some(user)),
                Some(Err(e)) => Err(e).context("Row mapping failed"),
                None => Ok(None),
            }
        })
        .await
        .context("Task join failed")?
    }

    pub async fn create_user(
        &self,
        username: &str,
        password_hash: &str,
        salt: &str,
        role: &str,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let password_hash = password_hash.to_string();
        let salt = salt.to_string();
        let role = role.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO users (username, password_hash, salt, role, failed_attempts, locked_until) VALUES (?1, ?2, ?3, ?4, 0, NULL)",
                rusqlite::params![username, password_hash, salt, role],
            )
            .context("Insert user failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn list_users(&self) -> Result<Vec<User>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(failed_attempts, 0), locked_until FROM users ORDER BY id")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        password_hash: row.get(2)?,
                        salt: row.get(3)?,
                        role: row.get(4)?,
                        must_change_password: row.get(5)?,
                        failed_attempts: row.get(6)?,
                        locked_until: row.get(7)?,
                    })
                })
                .context("Query failed")?;
            rows.collect::<Result<Vec<_>, _>>()
                .context("Row mapping failed")
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_user_by_id(&self, id: i64) -> Result<Option<User>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(failed_attempts, 0), locked_until FROM users WHERE id = ?1")
                .context("Prepare failed")?;
            let result = stmt
                .query_row(rusqlite::params![id], |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        password_hash: row.get(2)?,
                        salt: row.get(3)?,
                        role: row.get(4)?,
                        must_change_password: row.get(5)?,
                        failed_attempts: row.get(6)?,
                        locked_until: row.get(7)?,
                    })
                })
                .ok();
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_user_role(&self, id: i64, role: &str) -> Result<()> {
        let conn = self.conn.clone();
        let role = role.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET role = ?1 WHERE id = ?2",
                rusqlite::params![role, id],
            )
            .context("Update user role failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn set_must_change_password(&self, username: &str, flag: bool) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET must_change_password = ?1 WHERE username = ?2",
                rusqlite::params![flag, username],
            )
            .context("Set must_change_password failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_user_password(&self, id: i64, password_hash: &str) -> Result<()> {
        let conn = self.conn.clone();
        let password_hash = password_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET password_hash = ?1, must_change_password = 0, failed_attempts = 0, locked_until = NULL WHERE id = ?2",
                rusqlite::params![password_hash, id],
            )
            .context("Update user password failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn increment_failed_attempts(&self, username: &str) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET failed_attempts = COALESCE(failed_attempts, 0) + 1 WHERE username = ?1",
                rusqlite::params![username],
            )
            .context("Increment failed attempts failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn lock_account(&self, username: &str, locked_until: i64) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET locked_until = ?1 WHERE username = ?2",
                rusqlite::params![locked_until, username],
            )
            .context("Lock account failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn reset_failed_attempts(&self, username: &str) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE users SET failed_attempts = 0, locked_until = NULL WHERE username = ?1",
                rusqlite::params![username],
            )
            .context("Reset failed attempts failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    // ── Refresh token management ──

    pub async fn create_refresh_token(
        &self,
        username: &str,
        token_hash: &str,
        expires_at: i64,
        created_at: i64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO refresh_tokens (token_hash, username, expires_at, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![token_hash, username, expires_at, created_at],
            )
            .context("Create refresh token failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn get_refresh_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<crate::models::RefreshToken>> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT id, token_hash, username, expires_at, created_at, revoked_at FROM refresh_tokens WHERE token_hash = ?1"
            )
            .context("Prepare failed")?;
            let result = stmt.query_row(
                rusqlite::params![token_hash],
                |row| {
                    Ok(crate::models::RefreshToken {
                        id: row.get(0)?,
                        token_hash: row.get(1)?,
                        username: row.get(2)?,
                        expires_at: row.get(3)?,
                        created_at: row.get(4)?,
                        revoked_at: row.get(5)?,
                    })
                },
            )
            .ok();
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn revoke_refresh_token(&self, token_hash: &str) -> Result<()> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE refresh_tokens SET revoked_at = ?1 WHERE token_hash = ?2",
                rusqlite::params![crate::timeutil::now_secs(), token_hash],
            )
            .context("Revoke refresh token failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn revoke_all_user_refresh_tokens(&self, username: &str) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE refresh_tokens SET revoked_at = ?1 WHERE username = ?2 AND revoked_at IS NULL",
                rusqlite::params![crate::timeutil::now_secs(), username],
            )
            .context("Revoke all user refresh tokens failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    pub async fn delete_user(&self, id: i64) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM users WHERE id = ?1", rusqlite::params![id])
                .context("Delete user failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }
}
