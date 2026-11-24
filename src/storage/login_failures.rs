//! Failed sign-ins, counted so that brute force stays slow without letting
//! anyone who can reach the login page lock other people out.
//!
//! Every row of `login_failures` is one counter, keyed by `(username, source)`
//! where `source` is the client address the server resolved
//! ([`crate::security::client_ip`], which honours proxy headers only from
//! `trusted_proxy`):
//!
//! * `(user, address)`: wrong passwords for one account from one address.
//!   [`PAIR_MAX_FAILURES`] within [`FAILURE_WINDOW_SECS`] lock that pair for
//!   [`PAIR_LOCK_SECS`]. Other addresses can still sign in to the account.
//! * `("", address)`: wrong passwords (and unknown usernames) from one address
//!   across all accounts, against password spraying. [`SOURCE_MAX_FAILURES`]
//!   lock the address for [`SOURCE_LOCK_SECS`].
//! * `(user, "")`: wrong passwords for one account from all addresses, against
//!   a distributed attack. [`ACCOUNT_MAX_FAILURES`] start a short backoff
//!   ([`ACCOUNT_BACKOFF_SECS`], renewed by every further failure in the
//!   window) that applies only to addresses the user has not signed in from
//!   in the last [`KNOWN_SOURCE_SECS`], so stations the user works at keep
//!   working.
//! * `(user, SIGNATURE_SOURCE)`: wrong electronic-signature passwords, typed
//!   in an already signed-in session; locked like a pair.
//!
//! A successful sign-in clears the pair and remembers the address as known.
//! Changing or resetting the password clears every counter of the account.

use super::Storage;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};

/// How long failures count towards a limit.
pub const FAILURE_WINDOW_SECS: i64 = 900;
/// Wrong passwords for one account from one address before that pair locks.
pub const PAIR_MAX_FAILURES: i64 = 10;
/// How long a locked (account, address) pair stays locked.
pub const PAIR_LOCK_SECS: i64 = 900;
/// Wrong passwords from one address across all accounts before the address
/// is locked out of sign-in.
pub const SOURCE_MAX_FAILURES: i64 = 30;
/// How long a locked address stays locked.
pub const SOURCE_LOCK_SECS: i64 = 900;
/// Wrong passwords for one account from all addresses before new addresses
/// have to wait.
pub const ACCOUNT_MAX_FAILURES: i64 = 50;
/// How long new addresses wait once an account is over [`ACCOUNT_MAX_FAILURES`].
pub const ACCOUNT_BACKOFF_SECS: i64 = 60;
/// An address the user signed in from within this time skips the account backoff.
pub const KNOWN_SOURCE_SECS: i64 = 30 * 24 * 3600;
/// The source wrong signature passwords are counted under. Never an address.
pub const SIGNATURE_SOURCE: &str = "signature";

/// Why a sign-in is refused before its password is checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginBlock {
    /// The address has too many failures across all accounts.
    Source { until: i64 },
    /// This account has too many failures from this address.
    Pair { until: i64 },
    /// The account has too many failures from many addresses and this one is
    /// not one the user has signed in from.
    Account { until: i64 },
}

/// What a recorded failure led to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FailureOutcome {
    /// Failures of the pair in its window, counting this one.
    pub pair_failures: i64,
    /// This failure locked the pair.
    pub pair_locked: bool,
    /// This failure locked the address.
    pub source_locked: bool,
    /// This failure started or renewed the account backoff.
    pub account_backoff: bool,
}

/// One counter: add a failure, and lock once `max` is reached. With
/// `reset_on_lock` the count starts over at the lock (it then takes another
/// full run to lock again); without it every further failure in the window
/// renews the lock. Returns (failures counted, locked now).
fn bump(
    conn: &Connection,
    username: &str,
    source: &str,
    now: i64,
    max: i64,
    lock_secs: i64,
    reset_on_lock: bool,
) -> rusqlite::Result<(i64, bool)> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT failures, window_start FROM login_failures WHERE username = ?1 AND source = ?2",
            rusqlite::params![username, source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (failures, window_start) = match row {
        Some((n, start)) if now - start < FAILURE_WINDOW_SECS => (n + 1, start),
        _ => (1, now),
    };
    let locked = failures >= max;
    let (stored, window_start) = if locked && reset_on_lock {
        (0, now)
    } else {
        (failures, window_start)
    };
    let lock = locked.then_some(now + lock_secs);
    conn.execute(
        "INSERT INTO login_failures (username, source, failures, window_start, locked_until, last_failed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(username, source) DO UPDATE SET
             failures = excluded.failures,
             window_start = excluded.window_start,
             locked_until = COALESCE(excluded.locked_until, login_failures.locked_until),
             last_failed_at = excluded.last_failed_at",
        rusqlite::params![username, source, stored, window_start, lock, now],
    )?;
    Ok((failures, locked))
}

fn locked_until(
    conn: &Connection,
    username: &str,
    source: &str,
    now: i64,
) -> rusqlite::Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT locked_until FROM login_failures WHERE username = ?1 AND source = ?2",
            rusqlite::params![username, source],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten()
        .filter(|t| *t > now))
}

impl Storage {
    /// Whether a sign-in to `username` (`None`: an unknown or shared account,
    /// only the address counts) from `source` is refused before its password
    /// is checked.
    pub async fn login_block(
        &self,
        username: Option<&str>,
        source: &str,
        now: i64,
    ) -> Result<Option<LoginBlock>> {
        let conn = self.conn.clone();
        let username = username.map(str::to_string);
        let source = source.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            if let Some(until) = locked_until(&conn, "", &source, now)? {
                return Ok(Some(LoginBlock::Source { until }));
            }
            let Some(username) = username else {
                return Ok(None);
            };
            if let Some(until) = locked_until(&conn, &username, &source, now)? {
                return Ok(Some(LoginBlock::Pair { until }));
            }
            if let Some(until) = locked_until(&conn, &username, "", now)? {
                let known: Option<i64> = conn
                    .query_row(
                        "SELECT last_success_at FROM login_failures WHERE username = ?1 AND source = ?2",
                        rusqlite::params![username, source],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                if !known.is_some_and(|t| now - t < KNOWN_SOURCE_SECS) {
                    return Ok(Some(LoginBlock::Account { until }));
                }
            }
            Ok(None)
        })
        .await
        .context("Task join failed")?
    }

    /// When the pair `(username, source)` is locked until, if it is locked now.
    pub async fn login_pair_locked_until(
        &self,
        username: &str,
        source: &str,
        now: i64,
    ) -> Result<Option<i64>> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let source = source.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            Ok(locked_until(&conn, &username, &source, now)?)
        })
        .await
        .context("Task join failed")?
    }

    /// Count a wrong password (or an unknown username, `username` `None`)
    /// from `source`. `count_source` adds it to the address's own counter
    /// across accounts (sign-in does, signatures do not: their source is not
    /// an address).
    pub async fn record_login_failure(
        &self,
        username: Option<&str>,
        source: &str,
        count_source: bool,
        now: i64,
    ) -> Result<FailureOutcome> {
        let conn = self.conn.clone();
        let username = username.map(str::to_string);
        let source = source.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            // Rows with nothing left to say: no lock, no failure in the
            // window, no recent sign-in to remember the address by.
            tx.execute(
                "DELETE FROM login_failures
                 WHERE COALESCE(locked_until, 0) <= ?1
                   AND COALESCE(last_failed_at, 0) <= ?1 - ?2
                   AND COALESCE(last_success_at, 0) <= ?1 - ?3",
                rusqlite::params![now, FAILURE_WINDOW_SECS, KNOWN_SOURCE_SECS],
            )?;
            let mut outcome = FailureOutcome::default();
            if count_source {
                outcome.source_locked = bump(
                    &tx,
                    "",
                    &source,
                    now,
                    SOURCE_MAX_FAILURES,
                    SOURCE_LOCK_SECS,
                    true,
                )?
                .1;
            }
            if let Some(username) = username.as_deref() {
                (outcome.pair_failures, outcome.pair_locked) = bump(
                    &tx,
                    username,
                    &source,
                    now,
                    PAIR_MAX_FAILURES,
                    PAIR_LOCK_SECS,
                    true,
                )?;
                outcome.account_backoff = bump(
                    &tx,
                    username,
                    "",
                    now,
                    ACCOUNT_MAX_FAILURES,
                    ACCOUNT_BACKOFF_SECS,
                    false,
                )?
                .1;
            }
            tx.commit()?;
            Ok(outcome)
        })
        .await
        .context("Task join failed")?
    }

    /// A right password from `source`: the pair starts over and the address
    /// is remembered as one the user signs in from. The account-wide count
    /// is left alone (it runs out with its window), so an attacker does not
    /// get a fresh budget each time the real user signs in.
    pub async fn record_login_success(&self, username: &str, source: &str, now: i64) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        let source = source.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO login_failures (username, source, failures, window_start, last_success_at)
                 VALUES (?1, ?2, 0, ?3, ?3)
                 ON CONFLICT(username, source) DO UPDATE SET
                     failures = 0, locked_until = NULL, last_success_at = ?3",
                rusqlite::params![username, source, now],
            )
            .context("Record login success failed")?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Every failure count and lock of `username` (each address, the
    /// account-wide backoff and signatures): what an admin's password reset
    /// or the user's own password change unlocks. Addresses locked across
    /// all accounts stay locked.
    pub async fn clear_login_failures(&self, username: &str) -> Result<()> {
        let conn = self.conn.clone();
        let username = username.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            clear_login_failures_sync(&conn, &username)?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Failures counted for `(username, source)` in its current window.
    #[cfg(test)]
    pub async fn login_failure_count(&self, username: &str, source: &str) -> i64 {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT failures FROM login_failures WHERE username = ?1 AND source = ?2",
            rusqlite::params![username, source],
            |r| r.get(0),
        )
        .optional()
        .unwrap()
        .unwrap_or(0)
    }
}

/// [`Storage::clear_login_failures`] on an open connection.
pub(super) fn clear_login_failures_sync(conn: &Connection, username: &str) -> Result<()> {
    if username.is_empty() {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM login_failures WHERE username = ?1",
        rusqlite::params![username],
    )
    .context("Clear login failures failed")?;
    Ok(())
}
