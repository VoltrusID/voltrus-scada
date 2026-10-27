//! Model Context Protocol server built into the Voltrus server.
//!
//! One endpoint, `POST /mcp`, speaks MCP's Streamable HTTP transport in its
//! stateless form: every JSON-RPC request is answered in the HTTP response as
//! `application/json`, there are no sessions and no server-initiated stream
//! (`GET`/`DELETE` answer 405, which the transport allows).
//!
//! Authentication is the REST API's: an `X-API-Key` header, an
//! `Authorization: Bearer <api key>` header (what MCP clients send), or a
//! session cookie. Tools run in-process against the same handlers the REST
//! routes use, so validation, write policy and audit behaviour cannot drift.
//! Authorisation is per tool: read tools need any authenticated principal,
//! write tools need the role the REST operational write routes need
//! ([`crate::api::role_allows_operational_writes`]). Admin-only operations
//! (users, roles, API keys, licence, security) have no tool at all.
//!
//! Add a tool by creating a module under `tools/` that returns a `Vec<Tool>`
//! and listing it in [`tools::registry`].

mod protocol;
pub mod tools;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Extension, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use serde_json::json;

use crate::api::{AppState, AuthUser, cookie_auth_middleware};

/// Route path of the MCP endpoint.
pub const MCP_PATH: &str = "/mcp";

/// The `/mcp` route with its authentication and request guards. Merged into the
/// API router, so the global rate limiter, body limit and security headers apply.
pub fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route(
            MCP_PATH,
            post(handle_post)
                .get(method_not_allowed)
                .delete(method_not_allowed),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            cookie_auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bearer_api_key_middleware,
        ))
        .layer(middleware::from_fn_with_state(state, guard_middleware))
}

async fn handle_post(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: Bytes,
) -> Response {
    let message: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(protocol::error_response(
                    serde_json::Value::Null,
                    protocol::PARSE_ERROR,
                    format!("parse error: {e}"),
                )),
            )
                .into_response();
        }
    };
    match protocol::handle_message(&state, &user, message).await {
        Some(reply) => Json(reply).into_response(),
        // Notifications and client responses need no body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// The server offers no SSE stream and keeps no sessions.
async fn method_not_allowed() -> Response {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [(header::ALLOW, "POST")],
        Json(json!({"error": "this MCP endpoint is stateless: use POST with a JSON-RPC message"})),
    )
        .into_response()
}

/// Rejects cross-origin browser requests (DNS-rebinding and cookie-riding
/// protection the MCP transport requires), insists on a JSON body, and tells a
/// client with no credentials which scheme to use. Non-browser MCP clients send
/// no `Origin` header and are unaffected.
async fn guard_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if let Some(origin) = req.headers().get(header::ORIGIN) {
        let host = req.headers().get(header::HOST);
        if !origin_allowed(origin, host, state.cors_origin.as_deref()) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "origin not allowed"})),
            )
                .into_response();
        }
    }
    if req.method() == axum::http::Method::POST {
        let is_json = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase())
            .is_some_and(|v| v == "application/json" || v.starts_with("application/json;"));
        if !is_json {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(json!({"error": "Content-Type must be application/json"})),
            )
                .into_response();
        }
    }
    let mut resp = next.run(req).await;
    if resp.status() == StatusCode::UNAUTHORIZED {
        resp.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Bearer realm=\"voltrus\""),
        );
    }
    resp
}

/// An `Origin` is allowed when it names the host the request was sent to, or is
/// one of the configured `cors_origin` entries.
fn origin_allowed(
    origin: &HeaderValue,
    host: Option<&HeaderValue>,
    cors_origin: Option<&str>,
) -> bool {
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let origin = origin.trim_end_matches('/');
    if let Some(configured) = cors_origin
        && configured
            .split(',')
            .any(|o| o.trim().trim_end_matches('/').eq_ignore_ascii_case(origin))
    {
        return true;
    }
    let authority = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    host.and_then(|h| h.to_str().ok())
        .is_some_and(|h| h.eq_ignore_ascii_case(authority))
}

/// MCP clients send credentials as `Authorization: Bearer <token>`. When the
/// token is a valid API key and no `X-API-Key` header is present, hand it to
/// the shared authentication middleware as one, so a key authenticates the same
/// way here as on the REST routes. Any other bearer token is left alone for an
/// add-on edition's identity provider.
async fn bearer_api_key_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if !req.headers().contains_key("x-api-key")
        && let Some(token) = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|t| t.trim().to_string())
        && !token.is_empty()
        && matches!(state.storage.validate_api_key(&token).await, Ok(Some(_)))
        && let Ok(value) = HeaderValue::from_str(&token)
    {
        req.headers_mut().insert("x-api-key", value);
    }
    next.run(req).await
}
