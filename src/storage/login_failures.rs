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
//! * `(user, SESSION_SOURCE)`: wrong passwords typed in an already signed-in
//!   session (electronic signatures, the current password when changing it);
//!   locked like a pair, and the lock ends the user's sessions.
//!
//! A successful sign-in clears the pair and remembers the address as known;
//! a known address also skips the address lock, so someone behind the same
//! NAT cannot lock the user out of their own station. Failures from this
//! machine (loopback) don't count for the address. IPv6 addresses count per
//! /64 ([`crate::security::throttle_key`]).
//!
//! Attempts are counted before the password is checked
//! ([`Storage::begin_login_attempt`]) and taken back when it was right, so
//! parallel requests cannot all slip past a limit. Windows are fixed (they
//! start at the first failure), so up to twice a limit can land around the
//! end of one window.
//!
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
/// The source wrong passwords typed inside a signed-in session (electronic
/// signatures, the current password when changing it) are counted under.
/// Never an address.
pub const SESSION_SOURCE: &str = "session";

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

/// A password check in progress, counted as a failure from the start (see
/// [`Storage::begin_login_attempt`]). End it with
/// [`Storage::fail_login_attempt`] or [`Storage::succeed_login_attempt`].
#[derive(Debug)]
#[must_use]
pub struct LoginAttempt {
    username: Option<String>,
    source: String,
    count_source: bool,
    now: i64,
    account_backoff: bool,
}

struct Counter {
    failures: i64,
    window_start: i64,
    locked_until: Option<i64>,
    last_success_at: Option<i64>,
}

impl Counter {
    /// Failures in the current window.
    fn current(&self, now: i64) -> i64 {
        if now - self.window_start < FAILURE_WINDOW_SECS {
            self.failures
        } else {
            0
        }
    }

    fn locked(&self, now: i64) -> Option<i64> {
        self.locked_until.filter(|t| *t > now)
    }
}

fn counter(conn: &Connection, username: &str, source: &str) -> rusqlite::Result<Option<Counter>> {
    conn.query_row(
        "SELECT failures, window_start, locked_until, last_success_at
         FROM login_failures WHERE username = ?1 AND source = ?2",
        rusqlite::params![username, source],
        |r| {
            Ok(Counter {
                failures: r.get(0)?,
                window_start: r.get(1)?,
                locked_until: r.get(2)?,
                last_success_at: r.get(3)?,
            })
        },
    )
    .optional()
}

/// Add one failure to a counter (starting a new window when the last one
/// ran out). Returns the failures in the window.
fn count(conn: &Connection, username: &str, source: &str, now: i64) -> rusqlite::Result<i64> {
    let (failures, window_start) = match counter(conn, username, source)? {
        Some(c) if now - c.window_start < FAILURE_WINDOW_SECS => (c.failures + 1, c.window_start),
        _ => (1, now),
    };
    conn.execute(
        "INSERT INTO login_failures (username, source, failures, window_start, last_failed_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(username, source) DO UPDATE SET
             failures = excluded.failures,
             window_start = excluded.window_start,
             last_failed_at = excluded.last_failed_at",
        rusqlite::params![username, source, failures, window_start, now],
    )?;
    Ok(failures)
}

/// Take back one failure counted by [`count`] (the password was right).
fn uncount(conn: &Connection, username: &str, source: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE login_failures SET failures = MAX(failures - 1, 0)
         WHERE username = ?1 AND source = ?2",
        rusqlite::params![username, source],
    )?;
    Ok(())
}

/// Lock a counter that reached `max` in its window. With `reset` the count
/// starts over at the lock (it then takes another full run to lock again);
/// without it every further failure in the window renews the lock.
fn lock_if_over(
    conn: &Connection,
    username: &str,
    source: &str,
    now: i64,
    max: i64,
    lock_secs: i64,
    reset: bool,
) -> rusqlite::Result<bool> {
    let over = counter(conn, username, source)?.is_some_and(|c| c.current(now) >= max);
    if over {
        conn.execute(
            "UPDATE login_failures SET locked_until = ?3,
                 failures = CASE WHEN ?4 THEN 0 ELSE failures END,
                 window_start = CASE WHEN ?4 THEN ?5 ELSE window_start END
             WHERE username = ?1 AND source = ?2",
            rusqlite::params![username, source, now + lock_secs, reset, now],
        )?;
    }
    Ok(over)
}

/// Why a sign-in is refused before its password is checked. A counter that
/// reached its limit blocks even before its lock is written, so attempts
/// running at the same time cannot overshoot it.
fn check(
    conn: &Connection,
    username: Option<&str>,
    source: &str,
    count_source: bool,
    now: i64,
) -> rusqlite::Result<Option<LoginBlock>> {
    let pair = match username {
        Some(u) => counter(conn, u, source)?,
        None => None,
    };
    // An address the user signed in from recently: the user's own station,
    // which a neighbour behind the same address can't lock them out of.
    let known = pair
        .as_ref()
        .and_then(|p| p.last_success_at)
        .is_some_and(|t| now - t < KNOWN_SOURCE_SECS);
    if !known && let Some(src) = counter(conn, "", source)? {
        if let Some(until) = src.locked(now) {
            return Ok(Some(LoginBlock::Source { until }));
        }
        if count_source && src.current(now) >= SOURCE_MAX_FAILURES {
            return Ok(Some(LoginBlock::Source {
                until: src.window_start + FAILURE_WINDOW_SECS,
            }));
        }
    }
    let Some(username) = username else {
        return Ok(None);
    };
    if let Some(pair) = &pair {
        if let Some(until) = pair.locked(now) {
            return Ok(Some(LoginBlock::Pair { until }));
        }
        if pair.current(now) >= PAIR_MAX_FAILURES {
            return Ok(Some(LoginBlock::Pair {
                until: pair.window_start + FAILURE_WINDOW_SECS,
            }));
        }
    }
    if !known && let Some(until) = counter(conn, username, "")?.and_then(|a| a.locked(now)) {
        return Ok(Some(LoginBlock::Account { until }));
    }
    Ok(None)
}

/// Apply the pair and address locks for a failure already counted.
fn finish_failure(
    conn: &Connection,
    username: Option<&str>,
    source: &str,
    count_source: bool,
    now: i64,
) -> rusqlite::Result<FailureOutcome> {
    // Rows with nothing left to say: no lock, no failure in the window, no
    // recent sign-in to remember the address by.
    conn.execute(
        "DELETE FROM login_failures
         WHERE COALESCE(locked_until, 0) <= ?1
           AND COALESCE(last_failed_at, 0) <= ?1 - ?2
           AND COALESCE(last_success_at, 0) <= ?1 - ?3",
        rusqlite::params![now, FAILURE_WINDOW_SECS, KNOWN_SOURCE_SECS],
    )?;
    let mut outcome = FailureOutcome::default();
    if count_source {
        outcome.source_locked = lock_if_over(
            conn,
            "",
            source,
            now,
            SOURCE_MAX_FAILURES,
            SOURCE_LOCK_SECS,
            true,
        )?;
    }
    if let Some(username) = username {
        outcome.pair_failures = counter(conn, username, source)?.map_or(0, |c| c.current(now));
        outcome.pair_locked = lock_if_over(
            conn,
            username,
            source,
            now,
            PAIR_MAX_FAILURES,
            PAIR_LOCK_SECS,
            true,
        )?;
    }
    Ok(outcome)
}

impl Storage {
    /// Whether a sign-in to `username` (`None`: an unknown account, only the
    /// address counts) from `source` would be refused now.
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
            Ok(check(&conn, username.as_deref(), &source, true, now)?)
        })
        .await
        .context("Task join failed")?
    }

    /// Start a password check for `username` (`None`: an unknown account)
    /// from `source`: `Err` with the reason when it is refused, otherwise the
    /// attempt, already counted as a failure on every counter so that
    /// attempts running at the same time see each other. `count_source`
    /// adds it to the address's own counter across accounts (not for this
    /// machine, nor for password checks inside a session, whose source is
    /// not an address). Opening the account backoff happens here too.
    pub async fn begin_login_attempt(
        &self,
        username: Option<&str>,
        source: &str,
        count_source: bool,
        now: i64,
    ) -> Result<std::result::Result<LoginAttempt, LoginBlock>> {
        let conn = self.conn.clone();
        let username = username.map(str::to_string);
        let source = source.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            if let Some(block) = check(&tx, username.as_deref(), &source, count_source, now)? {
                return Ok(Err(block));
            }
            if count_source {
                count(&tx, "", &source, now)?;
            }
            let mut account_backoff = false;
            if let Some(u) = username.as_deref() {
                count(&tx, u, &source, now)?;
                count(&tx, u, "", now)?;
                account_backoff = lock_if_over(
                    &tx,
                    u,
                    "",
                    now,
                    ACCOUNT_MAX_FAILURES,
                    ACCOUNT_BACKOFF_SECS,
                    false,
                )?;
            }
            tx.commit()?;
            Ok(Ok(LoginAttempt {
                username,
                source,
                count_source,
                now,
                account_backoff,
            }))
        })
        .await
        .context("Task join failed")?
    }

    /// The password was wrong: lock what reached its limit.
    pub async fn fail_login_attempt(&self, attempt: LoginAttempt) -> Result<FailureOutcome> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            let mut outcome = finish_failure(
                &tx,
                attempt.username.as_deref(),
                &attempt.source,
                attempt.count_source,
                attempt.now,
            )?;
            outcome.account_backoff = attempt.account_backoff;
            tx.commit()?;
            Ok(outcome)
        })
        .await
        .context("Task join failed")?
    }

    /// The password was right: the pair starts over and the address is
    /// remembered as one the user signs in from; the attempt is taken back
    /// from the address and account counts. The rest of the account-wide
    /// count is left alone (it runs out with its window), so an attacker
    /// does not get a fresh budget each time the real user signs in.
    pub async fn succeed_login_attempt(&self, attempt: LoginAttempt) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            if attempt.count_source {
                uncount(&tx, "", &attempt.source)?;
            }
            if let Some(username) = attempt.username.as_deref() {
                uncount(&tx, username, "")?;
                tx.execute(
                    "INSERT INTO login_failures (username, source, failures, window_start, last_success_at)
                     VALUES (?1, ?2, 0, ?3, ?3)
                     ON CONFLICT(username, source) DO UPDATE SET
                         failures = 0, locked_until = NULL, last_success_at = ?3",
                    rusqlite::params![username, attempt.source, attempt.now],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// The password could not be checked (an external directory was down,
    /// say): take the attempt back from every counter it was added to, as if
    /// it never happened. A backoff it opened stays.
    pub async fn cancel_login_attempt(&self, attempt: LoginAttempt) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap();
            let tx = conn.transaction()?;
            if attempt.count_source {
                uncount(&tx, "", &attempt.source)?;
            }
            if let Some(username) = attempt.username.as_deref() {
                uncount(&tx, username, "")?;
                uncount(&tx, username, &attempt.source)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .context("Task join failed")?
    }

    /// Count a failure outright, without a check first (what
    /// [`Self::begin_login_attempt`] then [`Self::fail_login_attempt`] do
    /// for an attempt that was let through).
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
            if count_source {
                count(&tx, "", &source, now)?;
            }
            let mut account_backoff = false;
            if let Some(u) = username.as_deref() {
                count(&tx, u, &source, now)?;
                count(&tx, u, "", now)?;
                account_backoff = lock_if_over(
                    &tx,
                    u,
                    "",
                    now,
                    ACCOUNT_MAX_FAILURES,
                    ACCOUNT_BACKOFF_SECS,
                    false,
                )?;
            }
            let mut outcome = finish_failure(&tx, username.as_deref(), &source, count_source, now)?;
            outcome.account_backoff = account_backoff;
            tx.commit()?;
            Ok(outcome)
        })
        .await
        .context("Task join failed")?
    }

    /// Every failure count and lock of `username` (each address, the
    /// account-wide backoff and in-session password checks): what an admin's
    /// password reset or the user's own password change unlocks. Addresses
    /// locked across all accounts stay locked.
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
        counter(&conn, username, source)
            .unwrap()
            .map_or(0, |c| c.current(crate::timeutil::now_secs()))
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
