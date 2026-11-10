use super::AppState;
use axum::Json;
use axum::extract::Request;
use axum::extract::{Extension, State};
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

/// Roles that may use the operational write routes: admin, demo, and the
/// write-capable "integration" API key role. Plain "api" keys stay read-only.
/// REST, MCP and WebSocket tag writes all ask this one question, so the three
/// cannot drift apart.
pub fn role_allows_operational_writes(role: &str) -> bool {
    matches!(role, "admin" | "demo" | "integration")
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
    if !role_allows_operational_writes(&auth.role) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "admin access required"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// For the few protected routes a signed-in read-only person may use but a
/// read-only API key may not (acknowledging an alarm: an operator at the screen
/// acknowledges, ISA-18.2 style; an unattended read-only integration must not).
pub async fn deny_read_only_keys_middleware(req: Request, next: Next) -> axum::response::Response {
    if let Some(key) = req.extensions().get::<ApiKeyAuth>()
        && !role_allows_operational_writes(&key.role)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "read-only API keys cannot do this; use an integration key"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// Scopes an API key may list in `permissions`. `read` and `data:read` add
/// nothing (every key reads); `write`/`data:write` are needed, on an
/// integration key, for the operational write routes. There is no admin scope:
/// a key never reaches the admin routes.
pub const KEY_SCOPES: &[&str] = &["read", "write", "data:read", "data:write"];

/// The scopes a key's `permissions` text declares, or `None` when it declares
/// none (empty list, or legacy free text that is not a JSON list), in which
/// case the key role alone decides.
pub fn parse_key_scopes(permissions: &str) -> Option<Vec<String>> {
    let scopes: Vec<String> = serde_json::from_str(permissions.trim()).ok()?;
    (!scopes.is_empty()).then_some(scopes)
}

/// Check the `permissions` of a key being created: a JSON list of strings.
/// Other scope names than [`KEY_SCOPES`] are kept (older clients send e.g.
/// `data:export`) but grant nothing. A write scope needs the integration role,
/// otherwise the key would look write-capable and be refused everywhere.
pub fn validate_key_scopes(permissions: &str, role: &str) -> Result<(), String> {
    let trimmed = permissions.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let scopes: Vec<String> = serde_json::from_str(trimmed).map_err(|_| {
        format!(
            "permissions must be a JSON list of scopes ({})",
            KEY_SCOPES.join(", ")
        )
    })?;
    for scope in &scopes {
        if scope == "admin" {
            return Err("API keys cannot hold the admin scope".to_string());
        }
        if scope_allows_write(scope) && role != "integration" {
            return Err(format!(
                "scope {scope:?} needs the integration role; this key's role is {role:?}"
            ));
        }
    }
    Ok(())
}

fn scope_allows_write(scope: &str) -> bool {
    matches!(scope, "write" | "data:write")
}

/// The role a key acts with. Fails closed: only an "integration" key is
/// write-capable, and only when its scopes, if it lists any, include a write
/// scope. Unknown or legacy role values behave like the read-only "api" role.
pub fn effective_key_role(key_role: &str, permissions: &str) -> &'static str {
    if key_role != "integration" {
        return "api";
    }
    match parse_key_scopes(permissions) {
        None => "integration",
        Some(scopes) if scopes.iter().any(|s| scope_allows_write(s)) => "integration",
        Some(_) => "api",
    }
}

/// Look an API key up and build the principal it authenticates as.
pub(crate) async fn authenticate_api_key(
    state: &AppState,
    key: &str,
) -> Option<(AuthUser, ApiKeyAuth)> {
    let (id, permissions, key_role) = state.storage.validate_api_key(key).await.ok()??;
    let role = effective_key_role(&key_role, &permissions).to_string();
    Some((
        AuthUser {
            username: format!("api_key:{id}"),
            role: role.clone(),
        },
        ApiKeyAuth {
            id,
            permissions,
            role,
        },
    ))
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
    let client_ip = crate::security::client_ip(&req);

    let mut req = req;

    // 1. Try API key authentication (X-API-Key header, or ?api_key= where a
    //    client cannot set headers).
    let query_key = query_api_key(&req);
    if query_key.is_some() && !query_key_allowed(&req) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "API keys in the URL (?api_key=) are accepted only on GET /metrics and GET /api/v1/stream, because URLs end up in proxy and server logs. Send the key in the X-API-Key header."
            })),
        )
            .into_response();
    }
    if let Some(api_key) = header_api_key(&req).or(query_key) {
        match authenticate_api_key(&state, &api_key).await {
            Some((user, key)) => {
                req.extensions_mut().insert(user);
                req.extensions_mut().insert(key);
                return next.run(req).await;
            }
            None => {
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
/// or disabled user is refused, and the role is the stored one, not the one signed into
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
    if user.disabled {
        return None;
    }
    claims.role = user.role;
    Some(claims)
}

/// API key authentication info, inserted for requests authenticated via
/// X-API-Key (or `?api_key=` on the two routes that allow it).
/// `role` is the key's effective role ([`effective_key_role`]): "api" =
/// read-only, "integration" = also allowed on demo-allowed operational write
/// routes.
#[derive(Clone, Debug)]
pub struct ApiKeyAuth {
    pub id: i64,
    pub permissions: String,
    pub role: String,
}

fn header_api_key(req: &Request) -> Option<String> {
    req.headers()
        .get("X-API-Key")
        .and_then(|h| h.to_str().ok())
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

fn query_api_key(req: &Request) -> Option<String> {
    req.uri()
        .query()?
        .split('&')
        .find_map(|pair| pair.strip_prefix("api_key="))
        .filter(|k| !k.is_empty())
        .map(str::to_string)
}

/// Prometheus scrapers and browser `EventSource` cannot always send headers;
/// those two read-only endpoints keep the query parameter for compatibility.
fn query_key_allowed(req: &Request) -> bool {
    req.method() == axum::http::Method::GET
        && matches!(req.uri().path(), "/metrics" | "/api/v1/stream")
}
