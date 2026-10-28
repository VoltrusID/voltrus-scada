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
pub const LOGIN_MAX_IP_ATTEMPTS: u32 = 10;

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

/// Client IP resolved by [`client_ip`], attached to login requests by
/// [`login_ip_rate_limit_middleware`] so the handler audits the same address
/// that was rate limited.
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

/// Per-IP brute-force limit for the login route.
///
/// Rejects with 429 once an IP has [`LOGIN_MAX_IP_ATTEMPTS`] failed logins in the
/// window; only failed responses (401/403) count, so legitimate users behind the
/// same address are not penalised for successful logins. The IP is
/// [`client_ip`], so proxy headers are honoured only from loopback peers.
pub async fn login_ip_rate_limit_middleware(
    State(state): State<crate::api::AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&req);
    if !state.login_rate_limiter.is_ip_allowed(&ip) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"error": "Too many login attempts. Please try again later."})),
        )
            .into_response();
    }
    req.extensions_mut().insert(ClientIp(ip.clone()));
    let resp = next.run(req).await;
    if matches!(
        resp.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        state.login_rate_limiter.record_ip_attempt(&ip);
    }
    resp
}

/// Client IP for rate limiting and auditing.
///
/// Uses the TCP peer address. Proxy headers (`CF-Connecting-IP`, then the first
/// `X-Forwarded-For` hop, then `X-Real-IP`) are only trusted when the peer is
/// loopback, i.e. a local reverse proxy or tunnel; a remote client cannot spoof
/// them. Returns `"unknown"` when no peer address was recorded (test oneshots).
pub fn client_ip(req: &Request) -> String {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer);
    client_ip_from_parts(peer, req.headers())
}

/// [`client_ip`] for handlers that extract `ConnectInfo` and the headers
/// instead of the whole request.
pub fn client_ip_from_parts(peer: Option<SocketAddr>, headers: &axum::http::HeaderMap) -> String {
    let Some(peer) = peer else {
        return "unknown".to_string();
    };
    if peer.ip().is_loopback() {
        let forwarded = headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get("x-forwarded-for")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.split(',').next())
            })
            .or_else(|| headers.get("x-real-ip").and_then(|v| v.to_str().ok()))
            .and_then(|s| s.trim().parse::<std::net::IpAddr>().ok());
        if let Some(ip) = forwarded {
            return ip.to_string();
        }
    }
    peer.ip().to_string()
}

/// Whether the request reached us over HTTPS: the server does not terminate TLS,
/// so only a loopback peer (a local reverse proxy or tunnel) can say so, with
/// `X-Forwarded-Proto: https` (first hop) or Cloudflare's `CF-Visitor`
/// (`{"scheme":"https"}`). From any other peer the headers are ignored, as in
/// [`client_ip`].
pub fn request_is_https(peer: Option<&SocketAddr>, headers: &axum::http::HeaderMap) -> bool {
    if !peer.is_some_and(|p| p.ip().is_loopback()) {
        return false;
    }
    let forwarded_https = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("https"));
    forwarded_https
        || headers
            .get("cf-visitor")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .is_some_and(|v| v["scheme"].as_str() == Some("https"))
}

/// Middleware that applies a global per-IP rate limit to all requests.
/// This protects public endpoints like /api/health and /api/settings/branding
/// from being used as amplification or enumeration vectors.
pub async fn global_rate_limit_middleware(
    State(state): State<crate::api::AppState>,
    req: Request,
    next: Next,
) -> Response {
    let ip = client_ip(&req);
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
