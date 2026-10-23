use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum AiError {
    NoProviderConfigured,
    InvalidApiKey(String),
    RateLimited { retry_after_secs: Option<u64> },
    Timeout,
    MalformedResponse(String),
    ProviderError(String),
    NetworkError(String),
    ConfigError(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProviderConfigured => write!(f, "No AI provider configured"),
            Self::InvalidApiKey(msg) => write!(f, "Invalid API key: {msg}"),
            Self::RateLimited { .. } => write!(f, "AI provider rate limit exceeded"),
            Self::Timeout => write!(f, "AI request timed out"),
            Self::MalformedResponse(msg) => write!(f, "AI returned unexpected format: {msg}"),
            Self::ProviderError(msg) => write!(f, "AI provider error: {msg}"),
            Self::NetworkError(msg) => write!(f, "Network error: {msg}"),
            Self::ConfigError(msg) => write!(f, "Configuration error: {msg}"),
        }
    }
}

impl std::error::Error for AiError {}

impl IntoResponse for AiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::NoProviderConfigured => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::InvalidApiKey(_) => (StatusCode::UNAUTHORIZED, self.to_string()),
            Self::RateLimited { retry_after_secs } => {
                let resp = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({
                        "error": self.to_string(),
                        "retry_after_secs": retry_after_secs,
                    })),
                );
                return resp.into_response();
            }
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, self.to_string()),
            Self::MalformedResponse(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Self::ProviderError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Self::NetworkError(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            Self::ConfigError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
        };
        (status, Json(json!({"error": message}))).into_response()
    }
}

impl From<reqwest::Error> for AiError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else if err.is_status() {
            match err.status() {
                Some(StatusCode::TOO_MANY_REQUESTS) => Self::RateLimited {
                    retry_after_secs: None,
                },
                Some(StatusCode::UNAUTHORIZED) => {
                    Self::InvalidApiKey("Authentication failed".into())
                }
                Some(s) => Self::ProviderError(format!("HTTP {s}: {err}")),
                None => Self::ProviderError(err.to_string()),
            }
        } else if err.is_connect() {
            Self::NetworkError(format!("Connection failed: {err}"))
        } else {
            Self::NetworkError(err.to_string())
        }
    }
}
