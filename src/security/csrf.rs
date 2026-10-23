#![allow(dead_code)]
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rand::Rng as _;
use serde_json::json;

pub const CSRF_TOKEN_LENGTH: usize = 32;

pub fn generate_csrf_token() -> String {
    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(CSRF_TOKEN_LENGTH)
        .map(char::from)
        .collect();
    token
}

/// Validate that the X-CSRF-Token header matches the expected token.
pub fn validate_csrf_token(req: &Request, expected: &str) -> bool {
    req.headers()
        .get("X-CSRF-Token")
        .and_then(|v| v.to_str().ok())
        .map(|t| t == expected)
        .unwrap_or(false)
}

/// Middleware that validates CSRF tokens for state-changing operations.
/// The expected token should be stored in the request extensions.
pub async fn csrf_validation_middleware(req: Request, next: Next) -> Response {
    let expected = req
        .extensions()
        .get::<String>()
        .cloned()
        .unwrap_or_default();

    if !validate_csrf_token(&req, &expected) {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(json!({"error": "CSRF token missing or invalid"})),
        )
            .into_response();
    }

    next.run(req).await
}
