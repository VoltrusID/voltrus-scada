use super::AppState;
use axum::Json;
use axum::extract::Request;
use axum::extract::{ConnectInfo, Extension, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum_extra::extract::CookieJar;
use serde_json::json;

pub async fn admin_only_middleware(
    Extension(auth): Extension<AuthUser>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    if auth.role != "admin" {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin access required"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// Allows admin, demo, and integration roles. Used for routes that demo users
/// can access (device config, screens, layouts, commands, flows, etc.) but not
/// truly admin-only routes (user management, roles, API keys, security,
/// compliance). "integration" is the write-capable API key role; plain "api"
/// keys stay read-only and are rejected here.
pub async fn demo_allowed_middleware(
    Extension(auth): Extension<AuthUser>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    if auth.role != "admin" && auth.role != "demo" && auth.role != "integration" {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin access required"})),
        )
            .into_response();
    }
    next.run(req).await
}

#[derive(Clone)]
pub struct AuthUser {
    pub username: String,
    pub role: String,
}

pub async fn cookie_auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    // Extract client IP if available (may not be present in test oneshot)
    let client_ip = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut req = req;

    // 1. Try API key authentication (X-API-Key header or ?api_key= query param)
    if let Some(api_key) = extract_api_key(&req) {
        match state.storage.validate_api_key(&api_key).await {
            Ok(Some((id, permissions, key_role))) => {
                // Fail closed: only "integration" is privileged. Unknown or
                // legacy role values behave exactly like the read-only "api"
                // role and can never escalate to admin/demo.
                let role = if key_role == "integration" {
                    "integration"
                } else {
                    "api"
                };
                req.extensions_mut().insert(AuthUser {
                    username: format!("api_key:{}", id),
                    role: role.to_string(),
                });
                req.extensions_mut().insert(ApiKeyAuth {
                    id,
                    permissions,
                    role: role.to_string(),
                });
                return next.run(req).await;
            }
            _ => {
                // Failed API key auth — record intrusion and reject immediately.
                // If an API key was explicitly provided, it must be the sole auth method;
                // do not fall through to cookie/JWT auth.
                state
                    .intrusion_detector
                    .record_401_with_storage(&client_ip, &state.storage)
                    .await;
                return (StatusCode::UNAUTHORIZED, "Invalid or expired API key").into_response();
            }
        }
    }

    // 2. Try a Bearer token from an external identity provider (add-on editions only;
    //    the open core has none and skips straight to local cookie auth).
    if let Some(auth_header) = req.headers().get("authorization")
        && let Some(token) = auth_header
            .to_str()
            .ok()
            .and_then(|v| v.strip_prefix("Bearer "))
        && let Some(identity) = state.ext.authenticate_bearer(token).await
    {
        req.extensions_mut().insert(AuthUser {
            username: identity.username,
            role: identity.role,
        });
        return next.run(req).await;
    }

    // 3. Fall back to local cookie auth
    let jar = CookieJar::from_headers(req.headers());
    let token = jar.get("voltrus_token").map(|c| c.value()).unwrap_or("");
    if token.is_empty() {
        state
            .intrusion_detector
            .record_401_with_storage(&client_ip, &state.storage)
            .await;
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }
    match verify_session(&state, token).await {
        Some(claims) => {
            req.extensions_mut().insert(AuthUser {
                username: claims.username,
                role: claims.role,
            });
            next.run(req).await
        }
        None => {
            state
                .intrusion_detector
                .record_401_with_storage(&client_ip, &state.storage)
                .await;
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            )
                .into_response()
        }
    }
}

/// Verify an access token and re-read the user from the database: a deleted
/// user is refused, and the role is the stored one, not the one signed into
/// the token when it was issued (a demoted admin loses admin at once).
/// Fails closed on a storage error.
pub(crate) async fn verify_session(
    state: &AppState,
    token: &str,
) -> Option<crate::models::TokenClaims> {
    let mut claims = super::auth::verify_token(token, &state.auth_secret)?;
    let user = state
        .storage
        .get_user_by_username(&claims.username)
        .await
        .ok()??;
    claims.role = user.role;
    Some(claims)
}

/// API key authentication info, inserted for requests authenticated via
/// X-API-Key / ?api_key=.
/// Note: permissions field is stored for future use but not yet enforced.
/// `role` is the key's effective role ("api" = read-only, "integration" =
/// also allowed on demo-allowed operational write routes).
#[derive(Clone, Debug)]
pub struct ApiKeyAuth {
    pub id: i64,
    pub permissions: String,
    pub role: String,
}

/// Extract API key from X-API-Key header or ?api_key= query parameter.
fn extract_api_key(req: &Request) -> Option<String> {
    // Check X-API-Key header first
    if let Some(header) = req.headers().get("X-API-Key")
        && let Ok(key) = header.to_str()
        && !key.is_empty()
    {
        return Some(key.to_string());
    }
    // Fall back to query parameter
    if let Some(query) = req.uri().query() {
        for pair in query.split('&') {
            if let Some(key) = pair.strip_prefix("api_key=")
                && !key.is_empty()
            {
                return Some(key.to_string());
            }
        }
    }
    None
}
