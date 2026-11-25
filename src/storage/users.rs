use super::Storage;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use crate::models::User;

/// Username of the account that keeps the annotations and handovers of
/// deleted users (their `author_id` is a foreign key). It is disabled, has no
/// usable password and is left out of [`Storage::list_users`].
pub const DELETED_USER: &str = "[deleted user]";

/// Bumped whenever who may do what changes: a user's role, password,
/// disabled flag or deletion, a sign-in session revoked, an API key revoked.
/// Open WebSockets watch it to re-check their grant at once instead of at
/// their next periodic check (live Designer sessions, docs/collaboration.md).
pub static ACCESS: std::sync::LazyLock<tokio::sync::watch::Sender<u64>> =
    std::sync::LazyLock::new(|| tokio::sync::watch::Sender::new(0));

/// Note a change of access (see [`ACCESS`]).
pub fn access_changed() {
    ACCESS.send_modify(|g| *g = g.wrapping_add(1));
}

impl Storage {
    // ── Users ──

    pub async fn get_user_by_username(&self, username: &str) -> Result<Option<User>> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(disabled, 0) FROM users WHERE username = ?1")
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
                        disabled: row.get(6)?,
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
                "INSERT INTO users (username, password_hash, salt, role) VALUES (?1, ?2, ?3, ?4)",
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
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(disabled, 0) FROM users WHERE username != ?1 ORDER BY id")
                .context("Prepare failed")?;
            let rows = stmt
                .query_map(rusqlite::params![DELETED_USER], |row| {
                    Ok(User {
                        id: row.get(0)?,
                        username: row.get(1)?,
                        password_hash: row.get(2)?,
                        salt: row.get(3)?,
                        role: row.get(4)?,
                        must_change_password: row.get(5)?,
                        disabled: row.get(6)?,
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
                .prepare("SELECT id, username, password_hash, salt, COALESCE(role, 'user'), COALESCE(must_change_password, 0), COALESCE(disabled, 0) FROM users WHERE id = ?1")
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
                        disabled: row.get(6)?,
                    })
                })
                .ok();
            Ok(result)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn update_user_role(&self, id: i64, role: &str) -> Result<()> {
        let r: Result<_> = async {
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
        .await;
        access_changed();
        r
    }

    pub async fn set_must_change_password(&self, username: &str, flag: bool) -> Result<()> {
        let r: Result<_> = async {
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
        .await;
        super::access_changed();
        r
    }

    /// Set a new password (the user's own change, an admin's reset or the
    /// CLI's): it also clears every failed sign-in count and lock of the
    /// account, which is how an admin unlocks one.
    pub async fn update_user_password(&self, id: i64, password_hash: &str) -> Result<()> {
        let r: Result<_> = async {
            let conn = self.conn.clone();
            let password_hash = password_hash.to_string();
            tokio::task::spawn_blocking(move || {
                let mut conn = conn.lock().unwrap();
                let tx = conn.transaction().context("Begin update password failed")?;
                tx.execute(
                    "UPDATE users SET password_hash = ?1, must_change_password = 0 WHERE id = ?2",
                    rusqlite::params![password_hash, id],
                )
                .context("Update user password failed")?;
                let username: Option<String> = tx
                    .query_row(
                        "SELECT username FROM users WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .context("Find user failed")?;
                if let Some(username) = username {
                    super::login_failures::clear_login_failures_sync(&tx, &username)?;
                }
                tx.commit().context("Commit update password failed")?;
                Ok(())
            })
            .await
            .context("Task join failed")?
        }
        .await;
        access_changed();
        r
    }

    // ── Refresh token management ──

    /// A refresh token of a session that began at `session_started_at`, its
    /// user last active at `last_activity_at` (both Unix seconds).
    pub async fn create_session_refresh_token(
        &self,
        username: &str,
        token_hash: &str,
        expires_at: i64,
        created_at: i64,
        session_started_at: i64,
        last_activity_at: i64,
    ) -> Result<i64> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO refresh_tokens (token_hash, username, expires_at, created_at, session_started_at, last_activity_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    token_hash,
                    username,
                    expires_at,
                    created_at,
                    session_started_at,
                    last_activity_at
                ],
            )
            .context("Create refresh token failed")?;
            Ok(conn.last_insert_rowid())
        })
        .await
        .context("Task join failed")?
    }

    /// Record user activity on the live (not revoked) refresh token
    /// `token_hash`. Returns whether a token was updated.
    pub async fn touch_refresh_token_activity(&self, token_hash: &str, now: i64) -> Result<bool> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let n = conn
                .execute(
                    "UPDATE refresh_tokens SET last_activity_at = ?1
                     WHERE token_hash = ?2 AND revoked_at IS NULL",
                    rusqlite::params![now, token_hash],
                )
                .context("Record session activity failed")?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    /// Set when a refresh token's session began and its last activity
    /// (tests and migrations of old sessions).
    pub async fn set_refresh_token_session_times(
        &self,
        token_hash: &str,
        session_started_at: i64,
        last_activity_at: i64,
    ) -> Result<()> {
        let conn = self.conn.clone();
        let token_hash = token_hash.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE refresh_tokens SET session_started_at = ?1, last_activity_at = ?2
                 WHERE token_hash = ?3",
                rusqlite::params![session_started_at, last_activity_at, token_hash],
            )
            .context("Update refresh token session failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

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
                "SELECT id, token_hash, username, expires_at, created_at, revoked_at, session_started_at, last_activity_at FROM refresh_tokens WHERE token_hash = ?1"
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
                        session_started_at: row.get(6)?,
                        last_activity_at: row.get(7)?,
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
        let r: Result<_> = async {
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
        .await;
        access_changed();
        r
    }

    /// Revoke every live refresh token of `username` except the one whose
    /// hash is `keep` (the session that asked, e.g. for a password change).
    pub async fn revoke_other_user_refresh_tokens(
        &self,
        username: &str,
        keep: Option<&str>,
    ) -> Result<()> {
        let r: Result<_> = async {
        let conn = self.conn.clone();
        let username = username.to_string();
        let keep = keep.unwrap_or_default().to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "UPDATE refresh_tokens SET revoked_at = ?1 WHERE username = ?2 AND revoked_at IS NULL AND token_hash <> ?3",
                rusqlite::params![crate::timeutil::now_secs(), username, keep],
            )
            .context("Revoke other refresh tokens failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
        }
        .await;
        access_changed();
        r
    }

    /// Whether the sign-in session of `username` that began at
    /// `session_started_at` is still open: one of its refresh tokens is
    /// unexpired and not revoked, or was revoked less than `grace_secs` ago
    /// (a refresh revokes the old token just before it stores the new one).
    pub async fn session_alive(
        &self,
        username: &str,
        session_started_at: i64,
        grace_secs: i64,
    ) -> Result<bool> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let now = crate::timeutil::now_secs();
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM refresh_tokens
                     WHERE username = ?1 AND COALESCE(session_started_at, created_at) = ?2
                       AND expires_at > ?3 AND (revoked_at IS NULL OR revoked_at >= ?4)",
                    rusqlite::params![username, session_started_at, now, now - grace_secs],
                    |row| row.get(0),
                )
                .context("Session lookup failed")?;
            Ok(n > 0)
        })
        .await
        .context("Task join failed")?
    }

    pub async fn revoke_all_user_refresh_tokens(&self, username: &str) -> Result<()> {
        let r: Result<_> = async {
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
        .await;
        access_changed();
        r
    }

    /// Delete a user. Their notifications go with them; annotations and
    /// shift handovers they wrote are kept and re-attributed to the
    /// [`DELETED_USER`] account, since both reference `users(id)`.
    pub async fn delete_user(&self, id: i64) -> Result<()> {
        let r: Result<_> = async {
            let conn = self.conn.clone();
            tokio::task::spawn_blocking(move || {
                let mut conn = conn.lock().unwrap();
                let tx = conn.transaction().context("Begin delete user failed")?;
                tx.execute(
                    "DELETE FROM user_notifications WHERE user_id = ?1 OR from_user_id = ?1",
                    rusqlite::params![id],
                )
                .context("Delete user notifications failed")?;
                let authored: i64 = tx
                    .query_row(
                        "SELECT (SELECT COUNT(*) FROM annotations WHERE author_id = ?1)
                          + (SELECT COUNT(*) FROM shift_handovers WHERE author_id = ?1)",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                    .context("Count authored records failed")?;
                if authored > 0 {
                    tx.execute(
                    "INSERT OR IGNORE INTO users (username, password_hash, salt, role, disabled)
                     VALUES (?1, '!', '', 'viewer', 1)",
                    rusqlite::params![DELETED_USER],
                )
                .context("Insert deleted-user account failed")?;
                    let tombstone: i64 = tx
                        .query_row(
                            "SELECT id FROM users WHERE username = ?1",
                            rusqlite::params![DELETED_USER],
                            |row| row.get(0),
                        )
                        .context("Find deleted-user account failed")?;
                    tx.execute(
                        "UPDATE annotations SET author_id = ?1 WHERE author_id = ?2",
                        rusqlite::params![tombstone, id],
                    )
                    .context("Re-attribute annotations failed")?;
                    tx.execute(
                        "UPDATE shift_handovers SET author_id = ?1 WHERE author_id = ?2",
                        rusqlite::params![tombstone, id],
                    )
                    .context("Re-attribute handovers failed")?;
                }
                let username: Option<String> = tx
                    .query_row(
                        "SELECT username FROM users WHERE id = ?1",
                        rusqlite::params![id],
                        |r| r.get(0),
                    )
                    .optional()
                    .context("Find user failed")?;
                if let Some(username) = username {
                    super::login_failures::clear_login_failures_sync(&tx, &username)?;
                }
                tx.execute("DELETE FROM users WHERE id = ?1", rusqlite::params![id])
                    .context("Delete user failed")?;
                tx.commit().context("Commit delete user failed")?;
                Ok(())
            })
            .await
            .context("Task join failed")?
        }
        .await;
        access_changed();
        r
    }

    pub async fn set_user_disabled(&self, id: i64, disabled: bool) -> Result<()> {
        let r: Result<_> = async {
            let conn = self.conn.clone();
            tokio::task::spawn_blocking(move || {
                let conn = conn.lock().unwrap();
                conn.execute(
                    "UPDATE users SET disabled = ?1 WHERE id = ?2",
                    rusqlite::params![disabled as i64, id],
                )
                .context("Set user disabled failed")?;
                Ok(())
            })
            .await
            .context("Task join failed")?
        }
        .await;
        access_changed();
        r
    }

    /// Enabled admins other than `except_id`: the accounts that could still
    /// administer the plant if `except_id` were demoted, disabled or deleted.
    pub async fn count_other_active_admins(&self, except_id: i64) -> Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.query_row(
                "SELECT COUNT(*) FROM users WHERE role = 'admin' AND COALESCE(disabled, 0) = 0 AND id != ?1",
                rusqlite::params![except_id],
                |row| row.get(0),
            )
            .context("Count admins failed")
        })
        .await
        .context("Task join failed")?
    }
}
