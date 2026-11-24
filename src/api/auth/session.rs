//! Session limits from the edition's [`SessionPolicy`] (idle timeout and
//! absolute lifetime). A session is a chain of rotated refresh tokens; each
//! one carries when the session began and when the user was last active
//! (`POST /api/v1/auth/activity`). The refresh endpoint ends a session past
//! either limit, and access tokens never outlive what is left of it, so an
//! idle session stops working within its idle timeout.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde_json::json;

use super::*;
use crate::edition::SessionPolicy;

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// No user activity for longer than the idle timeout.
    Idle,
    /// Older than the absolute lifetime.
    Expired,
}

impl SessionEnd {
    pub fn code(self) -> &'static str {
        match self {
            Self::Idle => "session_idle",
            Self::Expired => "session_expired",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Idle => "Signed out after a period of inactivity",
            Self::Expired => "The session reached its maximum length; sign in again",
        }
    }
}

/// Whether a session that began at `started` (last active at `last_active`)
/// is over at `now` under `policy`. All times in Unix seconds.
pub fn session_ended(
    policy: &SessionPolicy,
    started: i64,
    last_active: i64,
    now: i64,
) -> Option<SessionEnd> {
    if let Some(limit) = policy.absolute_timeout_secs
        && now - started >= limit as i64
    {
        return Some(SessionEnd::Expired);
    }
    if let Some(idle) = policy.idle_timeout_secs
        && now - last_active >= idle as i64
    {
        return Some(SessionEnd::Idle);
    }
    None
}

/// How long an access token issued at `now` may live: the core's 15 minutes,
/// no longer than the idle timeout and no longer than what is left of the
/// session's absolute lifetime.
pub fn access_ttl_ms(policy: &SessionPolicy, started: i64, now: i64) -> u64 {
    let mut ttl = ACCESS_TOKEN_EXPIRY_MS;
    if let Some(idle) = policy.idle_timeout_secs {
        ttl = ttl.min(idle.saturating_mul(1000));
    }
    if let Some(limit) = policy.absolute_timeout_secs {
        let left = (started + limit as i64 - now).max(0) as u64;
        ttl = ttl.min(left.saturating_mul(1000));
    }
    ttl.max(1000)
}

/// The policy as clients see it (sign-in and `/auth/me`).
pub fn session_json(policy: &SessionPolicy) -> serde_json::Value {
    json!({
        "idle_timeout_secs": policy.idle_timeout_secs,
        "absolute_timeout_secs": policy.absolute_timeout_secs,
    })
}

/// The access cookie for a token valid `ttl_ms`.
pub(crate) fn access_cookie(
    token: String,
    secure: bool,
    ttl_ms: u64,
) -> axum_extra::extract::cookie::Cookie<'static> {
    axum_extra::extract::cookie::Cookie::build(("voltrus_token", token))
        .http_only(true)
        .secure(secure)
        .same_site(axum_extra::extract::cookie::SameSite::Strict)
        .max_age(time::Duration::seconds((ttl_ms / 1000).max(1) as i64))
        .path("/")
        .build()
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/activity",
    responses(
        (status = 200, body = serde_json::Value),
        (status = 401),
    ),
    tag = "Auth",
)]
/// POST /api/v1/auth/activity: the user did something (keyboard, mouse,
/// touch). Clients send it, throttled, while a session has an idle timeout;
/// background polling must not. Uses the refresh cookie to find the session.
pub async fn record_activity(State(state): State<AppState>, jar: CookieJar) -> impl IntoResponse {
    let Some(refresh) = jar.get("voltrus_refresh").map(|c| c.value().to_string()) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    let hash = hash_refresh_token(&refresh);
    let now = crate::timeutil::now_secs();
    let Ok(Some(rt)) = state.storage.get_refresh_token_by_hash(&hash).await else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    };
    if rt.revoked_at.is_some() || rt.expires_at <= now {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    // Activity after the session already ended does not revive it.
    let role = state
        .storage
        .get_user_by_username(&rt.username)
        .await
        .ok()
        .flatten()
        .map(|u| u.role)
        .unwrap_or_default();
    let policy = state.ext.session_policy(&rt.username, &role).await;
    let started = rt.session_started_at.unwrap_or(rt.created_at);
    let last = rt.last_activity_at.unwrap_or(rt.created_at);
    if let Some(end) = session_ended(&policy, started, last, now) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": end.message(), "code": end.code()})),
        )
            .into_response();
    }
    match state.storage.touch_refresh_token_activity(&hash, now).await {
        Ok(true) => Json(json!({"ok": true})).into_response(),
        _ => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_policy_never_ends_a_session() {
        let policy = SessionPolicy::default();
        assert_eq!(session_ended(&policy, 0, 0, 10_000_000), None);
        assert_eq!(
            access_ttl_ms(&policy, 0, 10_000_000),
            ACCESS_TOKEN_EXPIRY_MS
        );
    }

    #[test]
    fn idle_and_absolute_limits() {
        let policy = SessionPolicy {
            idle_timeout_secs: Some(300),
            absolute_timeout_secs: Some(3600),
        };
        assert_eq!(session_ended(&policy, 0, 100, 399), None);
        assert_eq!(session_ended(&policy, 0, 100, 400), Some(SessionEnd::Idle));
        assert_eq!(
            session_ended(&policy, 0, 3599, 3600),
            Some(SessionEnd::Expired)
        );
        // The access token never outlives the idle timeout or the session.
        assert_eq!(access_ttl_ms(&policy, 0, 0), 300_000);
        assert_eq!(access_ttl_ms(&policy, 0, 3500), 100_000);
        assert_eq!(access_ttl_ms(&policy, 0, 4000), 1000);
    }
}
