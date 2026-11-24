use axum::Json;
use axum::extract::{ConnectInfo, Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde_json::json;
use std::net::SocketAddr;

use super::*;

fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/login",
    request_body = crate::models::LoginRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 401),
        (status = 403),
        (status = 429),
    ),
    tag = "Auth",
)]
pub async fn login(
    State(state): State<AppState>,
    Extension(crate::security::ClientIp(client_ip)): Extension<crate::security::ClientIp>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let secure = state
        .cookie_secure
        .resolve(crate::security::request_is_https(
            peer.as_ref().map(|Extension(ConnectInfo(p))| p),
            &headers,
        ));
    let source_ip = Some(client_ip.clone());
    let user_agent = extract_user_agent(&headers);
    // A shared account's per-username limit is kept per address, or one
    // visitor typing wrong passwords would shut everyone else out of it.
    let listed_shared = state.public_demo.is_listed(&body.username);
    let rate_key_user = if listed_shared {
        format!("login:{}@{}", body.username, client_ip)
    } else {
        format!("login:{}", body.username)
    };

    // Check username-based rate limit
    if !state.login_rate_limiter.is_allowed(&rate_key_user) {
        return Ok((
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "Too many login attempts. Please try again later."})),
        )
            .into_response());
    }

    let start_time = std::time::Instant::now();
    let target_duration = std::time::Duration::from_millis(300);

    // Perform login logic
    let user = state.storage.get_user_by_username(&body.username).await?;
    let (response_status, response_json, new_jar) = match user {
        Some(user) => {
            let shared = state.public_demo.is_shared(&user.username, &user.role);
            if listed_shared && !shared {
                tracing::warn!(
                    "shared_accounts lists '{}', but it is an admin account: not treated as shared",
                    user.username
                );
            }
            // Check account lockout (a shared account is never locked)
            let now_secs = crate::timeutil::now_secs();
            if let Some(locked_until) = user.locked_until.filter(|_| !shared) {
                if locked_until > now_secs {
                    let _ = state
                        .storage
                        .insert_security_event(&crate::models::CreateSecurityEvent {
                            event_type: "login_locked".into(),
                            severity: "warning".into(),
                            source_ip: source_ip.clone(),
                            user_agent: user_agent.clone(),
                            username: Some(body.username.clone()),
                            details: Some("Attempted login on locked account".into()),
                        })
                        .await;
                    (
                        StatusCode::FORBIDDEN,
                        json!({"error": "Account locked. Please try again later."}),
                        jar,
                    )
                } else {
                    check_password_and_login(
                        state,
                        body,
                        user,
                        jar,
                        &rate_key_user,
                        now_secs,
                        source_ip.clone(),
                        user_agent.clone(),
                        secure,
                        shared,
                    )
                    .await
                }
            } else {
                check_password_and_login(
                    state,
                    body,
                    user,
                    jar,
                    &rate_key_user,
                    now_secs,
                    source_ip.clone(),
                    user_agent.clone(),
                    secure,
                    shared,
                )
                .await
            }
        }
        None => {
            state.login_rate_limiter.record_attempt(&rate_key_user);
            let _ = state
                .storage
                .insert_security_event(&crate::models::CreateSecurityEvent {
                    event_type: "login_failed".into(),
                    severity: "warning".into(),
                    source_ip: source_ip.clone(),
                    user_agent: user_agent.clone(),
                    username: Some(body.username.clone()),
                    details: Some("Unknown username".into()),
                })
                .await;
            (
                StatusCode::UNAUTHORIZED,
                json!({"error": "Invalid credentials"}),
                jar,
            )
        }
    };

    // Timing attack mitigation: ensure response takes at least target_duration
    let elapsed = start_time.elapsed();
    if elapsed < target_duration {
        tokio::time::sleep(target_duration - elapsed).await;
    }

    Ok((new_jar, (response_status, Json(response_json))).into_response())
}

async fn check_password_and_login(
    state: AppState,
    body: LoginRequest,
    user: User,
    jar: CookieJar,
    rate_key_user: &str,
    now_secs: i64,
    source_ip: Option<String>,
    user_agent: Option<String>,
    secure: bool,
    shared: bool,
) -> (StatusCode, serde_json::Value, CookieJar) {
    if !verify_password(&body.password, &user.password_hash) {
        state.login_rate_limiter.record_attempt(rate_key_user);
        // A shared account counts no failures: anyone could lock it for everyone.
        if !shared {
            let _ = state
                .storage
                .increment_failed_attempts(&body.username)
                .await;
        }

        // Lock account after 10 failed attempts
        if !shared && user.failed_attempts + 1 >= 10 {
            let lock_until = now_secs + 900; // 15 minutes
            let _ = state.storage.lock_account(&body.username, lock_until).await;
            tracing::warn!(
                "Account locked for user {} due to too many failed attempts",
                body.username
            );
        }

        let _ = state
            .storage
            .insert_security_event(&crate::models::CreateSecurityEvent {
                event_type: "login_failed".into(),
                severity: "warning".into(),
                source_ip: source_ip.clone(),
                user_agent: user_agent.clone(),
                username: Some(body.username.clone()),
                details: Some(if shared {
                    "Invalid password (shared account)".to_string()
                } else {
                    format!("Invalid password (attempt {})", user.failed_attempts + 1)
                }),
            })
            .await;

        return (
            StatusCode::UNAUTHORIZED,
            json!({"error": "Invalid credentials"}),
            jar,
        );
    }

    if user.disabled {
        let _ = state
            .storage
            .insert_security_event(&crate::models::CreateSecurityEvent {
                event_type: "login_disabled".into(),
                severity: "warning".into(),
                source_ip,
                user_agent,
                username: Some(body.username.clone()),
                details: Some("Login on a disabled account".into()),
            })
            .await;
        return (
            StatusCode::FORBIDDEN,
            json!({"error": "Account disabled"}),
            jar,
        );
    }

    // Successful login — reset failed attempts, then open the session.
    let _ = state.storage.reset_failed_attempts(&body.username).await;
    state.login_rate_limiter.cleanup();
    // An expired password (add-on password policy) is answered like a
    // password the admin asked to change.
    let password_expired = !shared && state.ext.password_expired(&user).await;
    // Kept on the account until the password is changed, so every session of
    // the user (this one, others already open, the WebSocket) is held to the
    // change, not only this sign-in's answer.
    if password_expired && !user.must_change_password {
        let _ = state
            .storage
            .set_must_change_password(&body.username, true)
            .await;
    }
    let policy = state.ext.session_policy(&body.username, &user.role).await;
    let jar = issue_session_with_policy(
        &state.storage,
        &state.auth_secret,
        secure,
        jar,
        &body.username,
        &user.role,
        &policy,
    )
    .await;
    (
        StatusCode::OK,
        json!({
            "ok": true,
            "username": body.username,
            "role": user.role,
            // Nobody can change a shared account's password, so never ask.
            "must_change_password": (user.must_change_password || password_expired) && !shared,
            "password_expired": password_expired,
            "shared": shared,
            "session": super::session_json(&policy),
        }),
        jar,
    )
}

/// Open a login session for an authenticated user: a signed access token and a
/// rotating refresh token, both set as the `voltrus_token` and `voltrus_refresh`
/// cookies. Local login and add-on sign-in providers (SSO) share it so every
/// session looks the same to the rest of the server.
///
/// Other sessions of the same user (another browser or device) keep their
/// refresh tokens; only the refresh token this client presents is replaced by
/// the new one.
///
/// The session gets no limit beyond the core's; use
/// [`issue_session_with_policy`] with the edition's
/// [`crate::edition::Extension::session_policy`] to cap the first access
/// token too (refreshes apply the policy either way).
pub async fn issue_session(
    storage: &crate::storage::Storage,
    auth_secret: &str,
    cookie_secure: bool,
    jar: CookieJar,
    username: &str,
    role: &str,
) -> CookieJar {
    issue_session_with_policy(
        storage,
        auth_secret,
        cookie_secure,
        jar,
        username,
        role,
        &crate::edition::SessionPolicy::default(),
    )
    .await
}

/// [`issue_session`] under a session policy: the access token lives no longer
/// than the idle timeout or the session's absolute lifetime.
pub async fn issue_session_with_policy(
    storage: &crate::storage::Storage,
    auth_secret: &str,
    cookie_secure: bool,
    jar: CookieJar,
    username: &str,
    role: &str,
    policy: &crate::edition::SessionPolicy,
) -> CookieJar {
    if let Some(old) = jar.get("voltrus_refresh") {
        let old_hash = hash_refresh_token(old.value());
        let _ = storage.revoke_refresh_token(&old_hash).await;
    }

    let now = crate::timeutil::now_secs();
    let ttl_ms = super::access_ttl_ms(policy, now, now);
    let access_token = create_token_with_ttl(username, role, auth_secret, ttl_ms);

    // Generate refresh token
    let refresh_token_raw = generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token_raw);
    let refresh_expires = now + REFRESH_TOKEN_EXPIRY_SECS;
    let _ = storage
        .create_session_refresh_token(username, &refresh_hash, refresh_expires, now, now, now)
        .await;

    let access_cookie = super::access_cookie(access_token, cookie_secure, ttl_ms);

    let refresh_cookie =
        axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", refresh_token_raw))
            .http_only(true)
            .secure(cookie_secure)
            .same_site(axum_extra::extract::cookie::SameSite::Strict)
            .max_age(time::Duration::seconds(REFRESH_TOKEN_EXPIRY_SECS))
            .path("/")
            .build();

    jar.add(access_cookie).add(refresh_cookie)
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/logout",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Auth",
)]
pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> impl IntoResponse {
    // Revoke refresh token if present
    if let Some(refresh_cookie) = jar.get("voltrus_refresh") {
        let refresh_hash = hash_refresh_token(refresh_cookie.value());
        let _ = state.storage.revoke_refresh_token(&refresh_hash).await;
    }

    let access_cookie = axum_extra::extract::cookie::Cookie::build(("voltrus_token", ""))
        .max_age(time::Duration::seconds(0))
        .path("/")
        .build();
    let refresh_cookie = axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", ""))
        .max_age(time::Duration::seconds(0))
        .path("/")
        .build();
    (
        jar.add(access_cookie).add(refresh_cookie),
        Json(json!({"ok": true})),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/me",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn auth_me(State(state): State<AppState>, jar: CookieJar) -> impl IntoResponse {
    let token = jar.get("voltrus_token").map(|c| c.value()).unwrap_or("");
    if token.is_empty() {
        return Json(json!({"authenticated": false}));
    }
    match crate::api::auth_middleware::verify_session_user(&state, token).await {
        Some((claims, user)) => {
            let shared = state.public_demo.is_shared(&claims.username, &claims.role);
            let policy = state
                .ext
                .session_policy(&claims.username, &claims.role)
                .await;
            // A reloaded page learns here that the session may only change
            // the password (every other call answers 403
            // `password_change_required`).
            let must_change = crate::api::auth_middleware::password_change_pending(&state, &user);
            let password_expired = must_change && state.ext.password_expired(&user).await;
            Json(json!({
                "authenticated": true,
                "username": claims.username,
                "role": claims.role,
                "shared": shared,
                "must_change_password": must_change,
                "password_expired": password_expired,
                "session": super::session_json(&policy),
            }))
        }
        None => Json(json!({"authenticated": false})),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/refresh",
    responses(
        (status = 200, body = serde_json::Value),
        (status = 401),
    ),
    tag = "Auth",
)]
pub async fn refresh_token(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let secure = state
        .cookie_secure
        .resolve(crate::security::request_is_https(
            peer.as_ref().map(|Extension(ConnectInfo(p))| p),
            &headers,
        ));
    let refresh_token = jar.get("voltrus_refresh").map(|c| c.value()).unwrap_or("");
    if refresh_token.is_empty() {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response());
    }

    let refresh_hash = hash_refresh_token(refresh_token);
    let valid = match state
        .storage
        .get_refresh_token_by_hash(&refresh_hash)
        .await?
    {
        Some(rt) if rt.revoked_at.is_none() && rt.expires_at > crate::timeutil::now_secs() => {
            // Revoke old token (rotation)
            let _ = state.storage.revoke_refresh_token(&rt.token_hash).await;
            // A deleted or disabled account gets no new session, even from a
            // refresh token that was never revoked.
            state
                .storage
                .get_user_by_username(&rt.username)
                .await?
                .filter(|user| !user.disabled)
                .map(|user| (user, rt))
        }
        _ => None,
    };
    match valid {
        Some((user, rt)) => {
            // Session limits of the add-on edition (none in Core).
            let now = crate::timeutil::now_secs();
            let policy = state.ext.session_policy(&user.username, &user.role).await;
            let started = rt.session_started_at.unwrap_or(rt.created_at);
            let last_active = rt.last_activity_at.unwrap_or(rt.created_at);
            if let Some(end) = super::session_ended(&policy, started, last_active, now) {
                return Ok((
                    clear_session_cookies(jar),
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": end.message(), "code": end.code()})),
                    ),
                )
                    .into_response());
            }

            // A password that expired since sign-in holds the session to a
            // change, as at sign-in (the hook is asked again on every refresh).
            let shared = state.public_demo.is_shared(&user.username, &user.role);
            if !shared && !user.must_change_password && state.ext.password_expired(&user).await {
                let _ = state
                    .storage
                    .set_must_change_password(&user.username, true)
                    .await;
            }

            // Create new tokens
            let ttl_ms = super::access_ttl_ms(&policy, started, now);
            let new_access =
                create_token_with_ttl(&user.username, &user.role, &state.auth_secret, ttl_ms);
            let new_refresh_raw = generate_refresh_token();
            let new_refresh_hash = hash_refresh_token(&new_refresh_raw);
            let refresh_expires = now + REFRESH_TOKEN_EXPIRY_SECS;
            let _ = state
                .storage
                .create_session_refresh_token(
                    &user.username,
                    &new_refresh_hash,
                    refresh_expires,
                    now,
                    started,
                    last_active,
                )
                .await;

            let access_cookie = super::access_cookie(new_access, secure, ttl_ms);

            let refresh_cookie =
                axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", new_refresh_raw))
                    .http_only(true)
                    .secure(secure)
                    .same_site(axum_extra::extract::cookie::SameSite::Strict)
                    .max_age(time::Duration::seconds(REFRESH_TOKEN_EXPIRY_SECS))
                    .path("/")
                    .build();

            let jar = jar.add(access_cookie).add(refresh_cookie);
            Ok((jar, Json(json!({"ok": true}))).into_response())
        }
        None => {
            // Clear cookies on invalid refresh
            let access_cookie = axum_extra::extract::cookie::Cookie::build(("voltrus_token", ""))
                .max_age(time::Duration::seconds(0))
                .path("/")
                .build();
            let refresh_cookie =
                axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", ""))
                    .max_age(time::Duration::seconds(0))
                    .path("/")
                    .build();
            let jar = jar.add(access_cookie).add(refresh_cookie);
            Ok((
                jar,
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "unauthorized"})),
                ),
            )
                .into_response())
        }
    }
}

/// Expire both session cookies.
fn clear_session_cookies(jar: CookieJar) -> CookieJar {
    let access_cookie = axum_extra::extract::cookie::Cookie::build(("voltrus_token", ""))
        .max_age(time::Duration::seconds(0))
        .path("/")
        .build();
    let refresh_cookie = axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", ""))
        .max_age(time::Duration::seconds(0))
        .path("/")
        .build();
    jar.add(access_cookie).add(refresh_cookie)
}

#[utoipa::path(
    post,
    path = "/api/v1/seed",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn seed(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    state.storage.seed_database(&[]).await?;
    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/change-password",
    request_body = crate::models::ChangePasswordRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Auth",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn change_password(
    State(state): State<AppState>,
    Extension(auth): Extension<super::super::AuthUser>,
    jar: CookieJar,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, AppError> {
    if state.public_demo.is_shared(&auth.username, &auth.role) {
        return Err(AppError::forbidden(
            crate::api::shared_accounts::SHARED_ACCOUNT_REFUSAL,
        ));
    }
    if body.new_password.len() < 8 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Password must be at least 8 characters"})),
        )
            .into_response());
    }
    let user = state
        .storage
        .get_user_by_username(&auth.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("User not found"))?;

    if !verify_password(&body.old_password, &user.password_hash) {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Current password is incorrect"})),
        )
            .into_response());
    }
    if let Err(message) = check_password_policy(
        &state,
        &user.username,
        &user.role,
        &body.new_password,
        crate::edition::PasswordChangeKind::SelfService,
    )
    .await
    {
        return Ok((StatusCode::BAD_REQUEST, Json(json!({"error": message}))).into_response());
    }

    let hash = hash_password(&body.new_password);
    state.storage.update_user_password(user.id, &hash).await?;
    // Sign out every other session of the user (a stolen session must not
    // outlive the password it was opened with); this one keeps its refresh token.
    let own_refresh = jar
        .get("voltrus_refresh")
        .map(|c| hash_refresh_token(c.value()));
    state
        .storage
        .revoke_other_user_refresh_tokens(&user.username, own_refresh.as_deref())
        .await?;
    if user.username == "admin" {
        crate::storage::remove_initial_admin_password(state.storage.db_path());
    }
    state
        .ext
        .password_changed(
            &user.username,
            &hash,
            crate::edition::PasswordChangeKind::SelfService,
        )
        .await;
    Ok(Json(json!({"ok": true})).into_response())
}

/// The add-on edition's password policy ([`crate::edition::Extension::validate_new_password`]);
/// the core's own minimum length is checked by each caller first. `Err` is
/// the message for the user.
pub(crate) async fn check_password_policy(
    state: &AppState,
    username: &str,
    role: &str,
    new_password: &str,
    kind: crate::edition::PasswordChangeKind,
) -> Result<(), String> {
    let check = crate::edition::PasswordCheck {
        username: username.to_string(),
        role: role.to_string(),
        new_password: new_password.to_string(),
        kind,
    };
    state.ext.validate_new_password(&check).await
}
