use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

pub const LOGIN_MAX_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW_SECS: u64 = 60;
const LOGIN_MAX_IP_ATTEMPTS: u32 = 10;

const GLOBAL_MAX_REQUESTS: u32 = 2000;
const GLOBAL_WINDOW_SECS: u64 = 60;

#[derive(Clone)]
pub struct LoginRateLimiter {
    attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl LoginRateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_allowed(&self, key: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(LOGIN_WINDOW_SECS);

        let entries = attempts.entry(key.to_string()).or_default();
        entries.retain(|t| now.duration_since(*t) < window);

        entries.len() < LOGIN_MAX_ATTEMPTS as usize
    }

    pub fn is_ip_allowed(&self, ip: &str) -> bool {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(LOGIN_WINDOW_SECS);

        let key = format!("ip:{}", ip);
        let entries = attempts.entry(key).or_default();
        entries.retain(|t| now.duration_since(*t) < window);

        entries.len() < LOGIN_MAX_IP_ATTEMPTS as usize
    }

    pub fn record_attempt(&self, key: &str) {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        let entries = attempts.entry(key.to_string()).or_default();
        entries.retain(|t| now.duration_since(*t) < Duration::from_secs(LOGIN_WINDOW_SECS));
        entries.push(now);
    }

    pub fn record_ip_attempt(&self, ip: &str) {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        let key = format!("ip:{}", ip);
        let entries = attempts.entry(key).or_default();
        entries.retain(|t| now.duration_since(*t) < Duration::from_secs(LOGIN_WINDOW_SECS));
        entries.push(now);
    }

    pub fn cleanup(&self) {
        let mut attempts = self.attempts.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(LOGIN_WINDOW_SECS * 2);
        attempts.retain(|_, entries| {
            entries.retain(|t| now.duration_since(*t) < window);
            !entries.is_empty()
        });
    }
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

// ── Global Rate Limiter ──

#[derive(Clone)]
pub struct GlobalRateLimiter {
    requests: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

impl GlobalRateLimiter {
    pub fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn is_allowed(&self, ip: &str) -> bool {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(GLOBAL_WINDOW_SECS);

        let key = ip.to_string();
        let entries = requests.entry(key).or_default();
        entries.retain(|t| now.duration_since(*t) < window);

        entries.len() < GLOBAL_MAX_REQUESTS as usize
    }

    pub fn record_request(&self, ip: &str) {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let key = ip.to_string();
        let entries = requests.entry(key).or_default();
        entries.retain(|t| now.duration_since(*t) < Duration::from_secs(GLOBAL_WINDOW_SECS));
        entries.push(now);
    }

    pub fn cleanup(&self) {
        let mut requests = self.requests.lock().unwrap();
        let now = Instant::now();
        let window = Duration::from_secs(GLOBAL_WINDOW_SECS * 2);
        requests.retain(|_, entries| {
            entries.retain(|t| now.duration_since(*t) < window);
            !entries.is_empty()
        });
    }
}

impl Default for GlobalRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Middleware that rate-limits login attempts by IP address.
pub async fn login_rate_limit_middleware(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> impl IntoResponse {
    let limiter = req
        .extensions()
        .get::<LoginRateLimiter>()
        .cloned()
        .unwrap_or_default();

    let key = addr.ip().to_string();

    if !limiter.is_allowed(&key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"error": "Too many login attempts. Please try again later."})),
        )
            .into_response();
    }

    limiter.record_attempt(&key);
    next.run(req).await
}

/// Middleware that applies a global per-IP rate limit to all requests.
/// This protects public endpoints like /api/health and /api/settings/branding
/// from being used as amplification or enumeration vectors.
pub async fn global_rate_limit_middleware(
    State(state): State<crate::api::AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Extract client IP if available (may not be present in test oneshot)
    let ip = req
        .extensions()
        .get::<ConnectInfo<std::net::SocketAddr>>()
        .map(|ci| ci.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if !state.global_rate_limiter.is_allowed(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"error": "Rate limit exceeded. Please try again later."})),
        )
            .into_response();
    }

    state.global_rate_limiter.record_request(&ip);
    next.run(req).await
}
