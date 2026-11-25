use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub enum AiError {
    NoProviderConfigured,
    InvalidApiKey(String),
    RateLimited {
        retry_after_secs: Option<u64>,
    },
    Timeout,
    MalformedResponse(String),
    /// The reply stopped at the output token limit, so its JSON is incomplete.
    Truncated {
        max_tokens: Option<u32>,
    },
    ProviderError(String),
    NetworkError(String),
    ConfigError(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProviderConfigured => write!(f, "No AI provider configured"),
            Self::InvalidApiKey(msg) => write!(
                f,
                "The AI provider rejected the API key ({msg}). Check the key in the AI settings."
            ),
            Self::RateLimited {
                retry_after_secs: Some(s),
            } => write!(
                f,
                "The AI provider's rate limit or quota was reached; try again in {s} s"
            ),
            Self::RateLimited { .. } => write!(
                f,
                "The AI provider's rate limit or quota was reached; try again later"
            ),
            Self::Timeout => write!(f, "AI request timed out"),
            Self::MalformedResponse(msg) => write!(f, "AI returned unexpected format: {msg}"),
            Self::Truncated { max_tokens } => {
                write!(f, "The AI reply was cut off at the output limit")?;
                if let Some(n) = max_tokens {
                    write!(f, " of {n} tokens")?;
                }
                write!(
                    f,
                    ", so it is incomplete and was not used. Send less at a time (part of the datasheet, or fewer tags)."
                )
            }
            Self::ProviderError(msg) => write!(f, "AI provider error: {msg}"),
            Self::NetworkError(msg) => write!(f, "Network error: {msg}"),
            Self::ConfigError(msg) => write!(f, "Configuration error: {msg}"),
        }
    }
}

impl std::error::Error for AiError {}

impl AiError {
    /// Status and machine-readable code of the answer to the browser. A key the
    /// provider rejects is not answered with 401: the web client reads 401 as
    /// its own session having expired.
    pub fn status_and_code(&self) -> (StatusCode, &'static str) {
        match self {
            Self::NoProviderConfigured => (StatusCode::BAD_REQUEST, "ai_not_configured"),
            Self::InvalidApiKey(_) => (StatusCode::BAD_REQUEST, "ai_invalid_api_key"),
            Self::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "ai_rate_limited"),
            Self::Timeout => (StatusCode::GATEWAY_TIMEOUT, "ai_timeout"),
            Self::MalformedResponse(_) => (StatusCode::BAD_GATEWAY, "ai_malformed_response"),
            Self::Truncated { .. } => (StatusCode::BAD_GATEWAY, "ai_truncated"),
            Self::ProviderError(_) => (StatusCode::BAD_GATEWAY, "ai_provider_error"),
            Self::NetworkError(_) => (StatusCode::BAD_GATEWAY, "ai_network_error"),
            Self::ConfigError(_) => (StatusCode::BAD_REQUEST, "ai_config_error"),
        }
    }

    /// The error for a provider's non-success HTTP answer; `body` is the
    /// provider's text, shortened in the message.
    pub fn from_provider_status(
        label: &str,
        status: StatusCode,
        retry_after: Option<&HeaderValue>,
        body: &str,
    ) -> Self {
        let detail = provider_message(body);
        match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                Self::InvalidApiKey(format!("{label} answered {status}: {detail}"))
            }
            StatusCode::TOO_MANY_REQUESTS => Self::RateLimited {
                retry_after_secs: retry_after
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .filter(|s| s.is_finite() && *s >= 0.0)
                    .map(|s| s.ceil() as u64),
            },
            StatusCode::NOT_FOUND => Self::ConfigError(format!(
                "{label} answered {status} (unknown model or wrong base URL?): {detail}"
            )),
            _ => Self::ProviderError(format!("{label} API error ({status}): {detail}")),
        }
    }
}

/// The `error.message` of an OpenAI or Anthropic error body, else the body
/// itself, cut to a length that fits in a banner.
fn provider_message(body: &str) -> String {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let msg = parsed
        .as_ref()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("error"))
        })
        .and_then(|m| m.as_str())
        .unwrap_or(body)
        .trim();
    const MAX: usize = 300;
    if msg.chars().count() > MAX {
        format!("{}…", msg.chars().take(MAX).collect::<String>())
    } else {
        msg.to_string()
    }
}

impl IntoResponse for AiError {
    fn into_response(self) -> Response {
        let (status, code) = self.status_and_code();
        let mut body = json!({"error": self.to_string(), "code": code});
        let mut retry = None;
        if let Self::RateLimited { retry_after_secs } = &self {
            body["retry_after_secs"] = json!(retry_after_secs);
            retry = *retry_after_secs;
        }
        let mut resp = (status, Json(body)).into_response();
        if let Some(secs) = retry
            && let Ok(v) = HeaderValue::from_str(&secs.to_string())
        {
            resp.headers_mut().insert(header::RETRY_AFTER, v);
        }
        resp
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_401_and_403_are_a_bad_key_answered_with_400() {
        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let e = AiError::from_provider_status(
                "OpenAI",
                status,
                None,
                r#"{"error":{"message":"Incorrect API key provided"}}"#,
            );
            assert!(matches!(e, AiError::InvalidApiKey(_)), "{e:?}");
            assert_eq!(
                e.status_and_code(),
                (StatusCode::BAD_REQUEST, "ai_invalid_api_key")
            );
            assert!(e.to_string().contains("Incorrect API key provided"), "{e}");
        }
    }

    #[test]
    fn provider_429_is_rate_limited_with_retry_after() {
        let v = HeaderValue::from_static("20");
        let e =
            AiError::from_provider_status("Anthropic", StatusCode::TOO_MANY_REQUESTS, Some(&v), "");
        assert!(matches!(
            e,
            AiError::RateLimited {
                retry_after_secs: Some(20)
            }
        ));
        let resp = e.into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers()[header::RETRY_AFTER], "20");

        let e = AiError::from_provider_status("x", StatusCode::TOO_MANY_REQUESTS, None, "");
        assert_eq!(e.status_and_code().0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn other_provider_failures_stay_502_with_a_short_message() {
        let long = "x".repeat(5000);
        let e = AiError::from_provider_status("OpenAI", StatusCode::BAD_GATEWAY, None, &long);
        assert_eq!(e.status_and_code().0, StatusCode::BAD_GATEWAY);
        assert!(e.to_string().len() < 600);
    }

    #[test]
    fn truncation_is_a_clear_502() {
        let e = AiError::Truncated {
            max_tokens: Some(4096),
        };
        assert_eq!(
            e.status_and_code(),
            (StatusCode::BAD_GATEWAY, "ai_truncated")
        );
        assert!(e.to_string().contains("cut off"), "{e}");
        assert!(e.to_string().contains("4096"), "{e}");
    }
}
