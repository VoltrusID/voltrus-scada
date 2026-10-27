use axum::Json;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde_json::json;

use super::*;

fn extract_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        })
}

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
    jar: CookieJar,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let source_ip = extract_ip(&headers);
    let user_agent = extract_user_agent(&headers);
    let rate_key_user = format!("login:{}", body.username);

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
            // Check account lockout
            let now_secs = crate::timeutil::now_secs();
            if let Some(locked_until) = user.locked_until {
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
) -> (StatusCode, serde_json::Value, CookieJar) {
    if !verify_password(&body.password, &user.password_hash) {
        state.login_rate_limiter.record_attempt(rate_key_user);
        let _ = state
            .storage
            .increment_failed_attempts(&body.username)
            .await;

        // Lock account after 10 failed attempts
        if user.failed_attempts + 1 >= 10 {
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
                details: Some(format!(
                    "Invalid password (attempt {})",
                    user.failed_attempts + 1
                )),
            })
            .await;

        return (
            StatusCode::UNAUTHORIZED,
            json!({"error": "Invalid credentials"}),
            jar,
        );
    }

    // Successful login — reset failed attempts and revoke old refresh tokens
    let _ = state.storage.reset_failed_attempts(&body.username).await;
    let _ = state
        .storage
        .revoke_all_user_refresh_tokens(&body.username)
        .await;
    state.login_rate_limiter.cleanup();

    let secret = &state.auth_secret;
    let access_token = create_token(&body.username, &user.role, secret);

    // Generate refresh token
    let refresh_token_raw = generate_refresh_token();
    let refresh_hash = hash_refresh_token(&refresh_token_raw);
    let refresh_expires = crate::timeutil::now_secs() + REFRESH_TOKEN_EXPIRY_SECS;
    let _ = state
        .storage
        .create_refresh_token(
            &body.username,
            &refresh_hash,
            refresh_expires,
            crate::timeutil::now_secs(),
        )
        .await;

    let access_cookie =
        axum_extra::extract::cookie::Cookie::build(("voltrus_token", access_token.clone()))
            .http_only(true)
            .secure(state.cookie_secure)
            .same_site(axum_extra::extract::cookie::SameSite::Strict)
            .max_age(time::Duration::seconds(900)) // 15 minutes
            .path("/")
            .build();

    let refresh_cookie =
        axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", refresh_token_raw))
            .http_only(true)
            .secure(state.cookie_secure)
            .same_site(axum_extra::extract::cookie::SameSite::Strict)
            .max_age(time::Duration::seconds(REFRESH_TOKEN_EXPIRY_SECS))
            .path("/")
            .build();

    let jar = jar.add(access_cookie).add(refresh_cookie);
    (
        StatusCode::OK,
        json!({
            "ok": true,
            "username": body.username,
            "role": user.role,
            "must_change_password": user.must_change_password,
        }),
        jar,
    )
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
    match crate::api::auth_middleware::verify_session(&state, token).await {
        Some(claims) => {
            Json(json!({"authenticated": true, "username": claims.username, "role": claims.role}))
        }
        None => Json(json!({"authenticated": false})),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/auth/csrf-token",
    responses(
        (status = 200, body = serde_json::Value),
        (status = 401),
    ),
    tag = "Auth",
)]
pub async fn get_csrf_token(State(state): State<AppState>, jar: CookieJar) -> impl IntoResponse {
    let token = jar.get("voltrus_token").map(|c| c.value()).unwrap_or("");
    if token.is_empty() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    match verify_token(token, &state.auth_secret) {
        Some(_claims) => {
            let csrf = crate::security::generate_csrf_token();
            Json(json!({"csrf_token": csrf})).into_response()
        }
        None => (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response(),
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
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    let refresh_token = jar.get("voltrus_refresh").map(|c| c.value()).unwrap_or("");
    if refresh_token.is_empty() {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response());
    }

    let refresh_hash = hash_refresh_token(refresh_token);
    match state
        .storage
        .get_refresh_token_by_hash(&refresh_hash)
        .await?
    {
        Some(rt) if rt.revoked_at.is_none() && rt.expires_at > crate::timeutil::now_secs() => {
            // Revoke old token (rotation)
            let _ = state.storage.revoke_refresh_token(&rt.token_hash).await;

            // Get user
            let user = state
                .storage
                .get_user_by_username(&rt.username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("User not found"))?;

            // Create new tokens
            let new_access = create_token(&user.username, &user.role, &state.auth_secret);
            let new_refresh_raw = generate_refresh_token();
            let new_refresh_hash = hash_refresh_token(&new_refresh_raw);
            let refresh_expires = crate::timeutil::now_secs() + REFRESH_TOKEN_EXPIRY_SECS;
            let _ = state
                .storage
                .create_refresh_token(
                    &user.username,
                    &new_refresh_hash,
                    refresh_expires,
                    crate::timeutil::now_secs(),
                )
                .await;

            let access_cookie =
                axum_extra::extract::cookie::Cookie::build(("voltrus_token", new_access))
                    .http_only(true)
                    .secure(state.cookie_secure)
                    .same_site(axum_extra::extract::cookie::SameSite::Strict)
                    .max_age(time::Duration::seconds(900))
                    .path("/")
                    .build();

            let refresh_cookie =
                axum_extra::extract::cookie::Cookie::build(("voltrus_refresh", new_refresh_raw))
                    .http_only(true)
                    .secure(state.cookie_secure)
                    .same_site(axum_extra::extract::cookie::SameSite::Strict)
                    .max_age(time::Duration::seconds(REFRESH_TOKEN_EXPIRY_SECS))
                    .path("/")
                    .build();

            let jar = jar.add(access_cookie).add(refresh_cookie);
            Ok((jar, Json(json!({"ok": true}))).into_response())
        }
        _ => {
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
    Json(body): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, AppError> {
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

    let hash = hash_password(&body.new_password);
    state.storage.update_user_password(user.id, &hash).await?;
    Ok(Json(json!({"ok": true})).into_response())
}
