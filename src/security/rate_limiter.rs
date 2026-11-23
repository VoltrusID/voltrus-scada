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

/// The header a reverse proxy or tunnel in front of the server writes the
/// client's address into (`trusted_proxy.header` in config.yaml).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyHeader {
    /// No proxy header is trusted: the client is the TCP peer.
    None,
    /// `X-Forwarded-For`, read from the right: the first hop that is not a
    /// trusted proxy address is the client (default).
    #[default]
    XForwardedFor,
    /// `X-Real-IP` (nginx `real_ip`).
    XRealIp,
    /// `CF-Connecting-IP` (Cloudflare and its tunnel).
    CfConnectingIp,
}

/// One entry of `trusted_proxy.addresses`: an address (`10.0.0.2`) or a
/// network in CIDR form (`10.0.0.0/24`, `fd00::/8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProxyNet {
    addr: std::net::IpAddr,
    prefix: u8,
}

impl ProxyNet {
    pub fn parse(text: &str) -> Result<Self, String> {
        let (addr, prefix) = match text.trim().split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text.trim(), None),
        };
        let addr: std::net::IpAddr = addr
            .parse()
            .map_err(|_| format!("'{text}' is not an IP address or network"))?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("'{text}': prefix must be 0-{max}"))?,
            None => max,
        };
        Ok(Self { addr, prefix })
    }

    pub fn contains(&self, ip: std::net::IpAddr) -> bool {
        use std::net::IpAddr;
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - self.prefix as u32).unwrap_or(0);
                (u32::from(net) & mask) == (u32::from(ip) & mask)
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - self.prefix as u32).unwrap_or(0);
                (u128::from(net) & mask) == (u128::from(ip) & mask)
            }
            _ => false,
        }
    }
}

impl<'de> serde::Deserialize<'de> for ProxyNet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        ProxyNet::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// `trusted_proxy:` in config.yaml: which header carries the client address,
/// and which peers may send it. The default trusts `X-Forwarded-For` from
/// this machine (127.0.0.1 and ::1), where a local reverse proxy or tunnel
/// runs; that proxy must append the address it got the request from to the
/// header (nginx `$proxy_add_x_forwarded_for`, Caddy and cloudflared do).
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct TrustedProxy {
    #[serde(default)]
    pub header: ProxyHeader,
    /// Addresses or CIDR networks of the proxies (default 127.0.0.1 and
    /// ::1). The header is read only when the TCP peer is one of them.
    #[serde(default = "default_proxy_addresses")]
    pub addresses: Vec<ProxyNet>,
}

fn default_proxy_addresses() -> Vec<ProxyNet> {
    vec![
        ProxyNet {
            addr: std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            prefix: 32,
        },
        ProxyNet {
            addr: std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            prefix: 128,
        },
    ]
}

impl Default for TrustedProxy {
    fn default() -> Self {
        Self {
            header: ProxyHeader::default(),
            addresses: default_proxy_addresses(),
        }
    }
}

/// One `X-Forwarded-For` hop: an address, possibly with a port
/// (`203.0.113.7:51000`, `[2001:db8::1]:443`).
fn parse_hop(hop: &str) -> Option<std::net::IpAddr> {
    let hop = hop.trim();
    hop.parse::<std::net::IpAddr>()
        .ok()
        .or_else(|| hop.parse::<SocketAddr>().ok().map(|s| s.ip()))
        .or_else(|| {
            hop.strip_prefix('[')
                .and_then(|h| h.strip_suffix(']'))
                .and_then(|h| h.parse().ok())
        })
}

fn unmapped(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

static MISCONFIG_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl TrustedProxy {
    fn trusts(&self, ip: std::net::IpAddr) -> bool {
        let ip = unmapped(ip);
        self.addresses.iter().any(|n| n.contains(ip))
    }

    /// The client address of a request from `peer` with `headers`.
    pub fn client_ip(&self, peer: SocketAddr, headers: &axum::http::HeaderMap) -> String {
        let peer_ip = unmapped(peer.ip());
        if self.header == ProxyHeader::None || !self.trusts(peer_ip) {
            return peer_ip.to_string();
        }
        let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
        let forwarded = match self.header {
            ProxyHeader::None => None,
            ProxyHeader::CfConnectingIp => value("cf-connecting-ip").and_then(parse_hop),
            ProxyHeader::XRealIp => value("x-real-ip").and_then(parse_hop),
            // Every proxy appends the address it got the request from, so the
            // rightmost hop that is not one of ours is the client; anything to
            // its left was written by the client and can be anything.
            ProxyHeader::XForwardedFor => {
                let hops: Vec<&str> = headers
                    .get_all("x-forwarded-for")
                    .iter()
                    .filter_map(|v| v.to_str().ok())
                    .flat_map(|v| v.split(','))
                    .collect();
                if hops.is_empty()
                    && (value("cf-connecting-ip").is_some() || value("x-real-ip").is_some())
                {
                    warn_misconfigured(
                        "a request from a trusted proxy carries CF-Connecting-IP or X-Real-IP \
                         but no X-Forwarded-For; set trusted_proxy.header to the header the \
                         proxy writes, or every client counts as the proxy",
                    );
                }
                let mut client = None;
                for hop in hops.iter().rev() {
                    match parse_hop(hop).map(unmapped) {
                        Some(ip) if self.trusts(ip) => client = Some(ip),
                        Some(ip) => {
                            client = Some(ip);
                            break;
                        }
                        None => break,
                    }
                }
                client
            }
        };
        forwarded.unwrap_or(peer_ip).to_string()
    }
}

/// Log a proxy misconfiguration once per process.
fn warn_misconfigured(message: &str) {
    if !MISCONFIG_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!("trusted_proxy: {message} (see config.yaml)");
    }
}

static TRUSTED_PROXY: std::sync::RwLock<Option<TrustedProxy>> = std::sync::RwLock::new(None);
static DEFAULT_PROXY: std::sync::LazyLock<TrustedProxy> =
    std::sync::LazyLock::new(TrustedProxy::default);

/// Apply the config's `trusted_proxy` (called once at startup).
pub fn set_trusted_proxy(proxy: TrustedProxy) {
    *TRUSTED_PROXY.write().unwrap_or_else(|e| e.into_inner()) = Some(proxy);
}

/// Client IP for rate limiting and auditing.
///
/// The TCP peer address, unless the peer is a trusted proxy
/// (`trusted_proxy` in config.yaml; by default this machine, reading
/// `X-Forwarded-For` from the right): then the address the proxy names
/// ([`TrustedProxy::client_ip`]). A remote client cannot pick its own
/// address. Returns `"unknown"` when no peer address was recorded (test
/// oneshots).
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
    let guard = TRUSTED_PROXY.read().unwrap_or_else(|e| e.into_inner());
    guard
        .as_ref()
        .unwrap_or(&DEFAULT_PROXY)
        .client_ip(peer, headers)
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
