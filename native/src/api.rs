//! REST client for the voltrus server API (`/api/v1`).
//!
//! Auth: login sets httpOnly `voltrus_token` and `voltrus_refresh` cookies,
//! not returned in the body, so we parse them from `Set-Cookie` and replay
//! the token as a Cookie header (the refresh cookie only to renew the
//! session and to sign out, so the server can revoke it).
//!
//! Every call goes through three generic helpers: [`VoltrusApi::get_json`],
//! [`VoltrusApi::send_json`] and [`VoltrusApi::send_empty`]. A new endpoint
//! is a typed mirror of its answer plus one line calling a helper; the
//! console reaches it through the runtime's generic `Command::Call` (see
//! `runtime::Call`) without a new command or event.

pub mod shell;

use std::fmt;

use anyhow::{Result, anyhow};
pub use reqwest::Method;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};

use crate::ws_types::{Signature, SignatureRequired};

pub mod admin;
pub mod eng_edr;
pub mod eng_flows;
pub mod eng_imports;
pub mod eng_mqtt;
pub mod eng_scripts;
pub mod eng_snapshots;
pub mod eng_system;
pub mod insights;
pub mod ops;
pub mod protocols;

/// A request the server answered with an error status.
#[derive(Debug, Clone)]
pub struct ServerError {
    pub status: reqwest::StatusCode,
    /// The server's `{"error": …}` text, or the status and body.
    pub message: String,
    /// The server's machine-readable `code`, when it gave one
    /// (`signature_required`, `signature_invalid`, `account_locked`, …).
    pub code: Option<String>,
    /// What an electronic signature would have to state, when the server
    /// asks for one (see [`crate::signature`]).
    pub signature_required: Option<SignatureRequired>,
    /// The node a refused flow's error is about (`{"error", "node_id"}`).
    pub node_id: Option<String>,
}

impl ServerError {
    pub fn new(status: reqwest::StatusCode, body: &str) -> Self {
        // Server errors are `{"error": "...", "code": "..."}` JSON.
        let value = serde_json::from_str::<serde_json::Value>(body).ok();
        let field = |name: &str| {
            value
                .as_ref()
                .and_then(|value| value.get(name)?.as_str().map(str::to_owned))
        };
        let message = field("error").unwrap_or_else(|| format!("HTTP {status}: {body}"));
        let code = field("code");
        let node_id = field("node_id");
        let signature_required = value
            .as_ref()
            .and_then(|value| value.get("signature_required"))
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        Self {
            status,
            message,
            code,
            signature_required,
            node_id,
        }
    }

    /// The server refused the credentials: a wrong password (401) or a
    /// locked or disabled account (403). Signing in again with them only
    /// counts towards locking the account.
    pub fn refuses_credentials(&self) -> bool {
        matches!(
            self.status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        )
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ServerError {}

/// Whether a sign-in failed because the server refused the credentials, as
/// opposed to being unreachable or busy.
pub fn refused(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<ServerError>()
        .is_some_and(ServerError::refuses_credentials)
}

pub const TOKEN_COOKIE: &str = "voltrus_token";
pub const REFRESH_COOKIE: &str = "voltrus_refresh";

#[derive(Debug, Clone, Serialize)]
struct LoginBody<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginOk {
    pub ok: bool,
    pub username: String,
    pub role: String,
    /// Set for a generated or reset password: the operator must choose a new
    /// one before working.
    #[serde(default)]
    pub must_change_password: bool,
    /// The password is past the edition's maximum age (the change is then
    /// for that reason, not a generated password).
    #[serde(default)]
    pub password_expired: bool,
    /// The edition's session limits; older servers and Core send none.
    #[serde(default)]
    pub session: SessionPolicy,
}

/// The session limits the server enforces (`session` of the sign-in
/// answer): signed out after `idle_timeout_secs` without user input (see
/// `POST /auth/activity`), and after `absolute_timeout_secs` in any case.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct SessionPolicy {
    pub idle_timeout_secs: Option<u64>,
    pub absolute_timeout_secs: Option<u64>,
}

impl SessionPolicy {
    /// How often user input may be reported: at most once per
    /// min(60 s, idle timeout / 3), as the web client does, so the server
    /// hears of an active user well before the limit; `None` without an
    /// idle timeout (nothing to report).
    pub fn activity_interval(&self) -> Option<std::time::Duration> {
        let idle = self.idle_timeout_secs.filter(|secs| *secs > 0)?;
        let millis = (idle.saturating_mul(1000) / 3).clamp(1000, 60_000);
        Some(std::time::Duration::from_millis(millis))
    }
}

/// Why the server ended a session (`code` of a refused renewal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEnd {
    /// No user input for longer than the idle timeout.
    Idle,
    /// The session reached its maximum length.
    Expired,
}

impl SessionEnd {
    pub fn from_code(code: Option<&str>) -> Option<Self> {
        match code? {
            "session_idle" => Some(Self::Idle),
            "session_expired" => Some(Self::Expired),
            _ => None,
        }
    }

    /// What the sign-in card says, in the web client's words.
    pub fn message(self, policy: &SessionPolicy) -> String {
        match (self, policy.idle_timeout_secs.filter(|secs| *secs > 0)) {
            (Self::Expired, _) => {
                "Your session reached its maximum length. Please sign in again.".to_owned()
            }
            (Self::Idle, Some(secs)) => {
                let minutes = ((secs + 30) / 60).max(1);
                let plural = if minutes == 1 { "" } else { "s" };
                format!("You were signed out after {minutes} minute{plural} of inactivity.")
            }
            (Self::Idle, None) => "You were signed out after a period of inactivity.".to_owned(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ChangePasswordBody<'a> {
    old_password: &'a str,
    new_password: &'a str,
}

/// One tag's history in the server's compact form: evenly spaced values from
/// `start` (Unix ms), `interval` ms apart.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct HistorySeries {
    #[serde(default)]
    pub values: Vec<f64>,
    #[serde(default)]
    pub start: i64,
    #[serde(default)]
    pub interval: u64,
}

impl HistorySeries {
    /// `(Unix ms, value)` of every sample.
    pub fn samples(&self) -> impl Iterator<Item = (i64, f64)> + '_ {
        let interval = i64::try_from(self.interval).unwrap_or(i64::MAX);
        self.values.iter().enumerate().map(move |(index, value)| {
            let offset = i64::try_from(index)
                .unwrap_or(i64::MAX)
                .saturating_mul(interval);
            (self.start.saturating_add(offset), *value)
        })
    }
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    /// By data source id (`keyed_by=id`).
    #[serde(default)]
    data: std::collections::HashMap<String, HistorySeries>,
}

#[derive(Debug, Serialize)]
struct ShelveBody<'a> {
    duration_secs: u64,
    reason: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<&'a Signature>,
}

/// The body of an alarm acknowledge that carries an electronic signature.
#[derive(Debug, Serialize)]
struct SignedBody<'a> {
    signature: &'a Signature,
}

/// Mirrors `voltrus::models::DataSource`, the full row `GET
/// /api/v1/data-sources` answers with. Missing fields default, so an older
/// or newer server still decodes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DataSource {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub unit: String,
    pub color: String,
    pub min_val: f64,
    pub max_val: f64,
    pub warn_val: f64,
    pub crit_val: f64,
    pub base_val: f64,
    pub sort_order: i64,
    pub decimals: i64,
    pub source_type: String,
    /// JSON text: per-type settings, the low limits (`low_warn`,
    /// `low_crit`) and an optional `expression`.
    pub source_config: String,
    pub device_id: String,
    pub equipment_id: Option<String>,
    pub chart_type: String,
    /// The tag tree path, e.g. `plant/inverters/inv1`.
    pub path: Option<String>,
    /// ISA-18.2 alarm on/off delays in seconds; 0 is immediate.
    pub on_delay_secs: i64,
    pub off_delay_secs: i64,
}

/// `GET /api/v1/version`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Version {
    pub version: String,
}

/// Mirrors `voltrus::models::Alarm`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Alarm {
    pub id: i64,
    pub source_key: String,
    #[serde(default)]
    pub source_name: String,
    pub severity: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub threshold: f64,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub triggered_at: i64,
    pub resolved_at: Option<i64>,
    pub acknowledged_at: Option<i64>,
    pub acknowledged_by: Option<String>,
    pub cooldown_until: Option<i64>,
    pub shelved_until: Option<i64>,
    pub shelved_by: Option<String>,
    pub shelved_reason: Option<String>,
    #[serde(default)]
    pub deadband: f64,
    #[serde(default)]
    pub state: String,
    #[serde(default = "default_alarm_type")]
    pub alarm_type: String,
}

fn default_alarm_type() -> String {
    "high".to_string()
}

/// `GET /api/v1/alarms/summary` (operator): open alarms nobody has
/// acknowledged by priority, shelved alarms, alarms raised in the last 24 h
/// and the mean time to acknowledge them. Mirrors `models::AlarmSummary`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AlarmSummary {
    pub active_critical: i64,
    pub active_warning: i64,
    pub active_info: i64,
    pub shelved: i64,
    pub recent_24h: i64,
    pub avg_ack_time_secs: f64,
}

/// A row of `GET /api/v1/alarms/analytics?from=&to=` (operator): how often
/// a source alarmed in the window, most often first. Mirrors
/// `models::AlarmAnalyticsEntry`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AlarmAnalyticsEntry {
    pub source_key: String,
    pub source_name: String,
    pub trigger_count: i64,
    pub avg_time_to_ack_secs: f64,
    pub last_triggered_at: i64,
}

/// A row of `GET /api/v1/alarms/audit` (operator), newest first. `details`
/// is free text, or JSON (`{"reason":…,"until":…}`) for a shelve. Mirrors
/// `models::AlarmAuditEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AlarmAuditEntry {
    pub id: i64,
    pub alarm_id: i64,
    pub action: String,
    pub performed_by: String,
    pub details: String,
    pub created_at: i64,
}

/// A row of `GET /api/v1/alarms/{id}/journal` (operator): one ISA-18.2
/// lifecycle event (triggered, acknowledged, shelved, unshelved, resolved),
/// oldest first. Mirrors `models::AlarmJournalEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AlarmJournalEntry {
    pub id: i64,
    pub alarm_id: i64,
    pub event: String,
    pub at: i64,
    pub actor: Option<String>,
}

/// Percent-decode a cookie value. Since SCADA-17 the server's access token is
/// base64url without padding, so its `Set-Cookie` value has nothing to
/// decode; servers before that percent-encoded the JWT payload's base64 `=`
/// padding as `%3D`, which the WebSocket auth path refuses. Browsers' cookie
/// jars decode this transparently; we do the same so both work.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = |b: u8| match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => 0xff,
            };
            let h = hex(bytes[i + 1]);
            let l = hex(bytes[i + 2]);
            if h != 0xff && l != 0xff {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value of cookie `name` in raw `Set-Cookie` header values:
/// `voltrus_token=<jwt>; HttpOnly; SameSite=Strict; ...` → `<jwt>`. An
/// empty value (the server clearing the cookie) counts as none.
pub fn parse_cookie(set_cookie_values: &[String], name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    for raw in set_cookie_values {
        if let Some(rest) = raw.strip_prefix(&prefix) {
            let value = rest.split(';').next().unwrap_or("").trim();
            if !value.is_empty() {
                return Some(percent_decode(value));
            }
        }
    }
    None
}

/// Parse a response body. An empty body reads as JSON `null`, so callers
/// that do not care about the answer can ask for [`IgnoredAny`], `()` or an
/// `Option`.
fn parse_body<T: DeserializeOwned>(body: &str) -> Result<T> {
    let body = if body.trim().is_empty() { "null" } else { body };
    Ok(serde_json::from_str(body)?)
}

/// The `Max-Age` of cookie `name` in raw `Set-Cookie` values: how long the
/// server lets the token live.
pub fn cookie_max_age(set_cookie_values: &[String], name: &str) -> Option<std::time::Duration> {
    let prefix = format!("{name}=");
    let raw = set_cookie_values
        .iter()
        .find(|raw| raw.starts_with(&prefix))?;
    raw.split(';').skip(1).find_map(|attribute| {
        let (key, value) = attribute.trim().split_once('=')?;
        key.eq_ignore_ascii_case("max-age")
            .then(|| value.trim().parse::<u64>().ok())
            .flatten()
            .map(std::time::Duration::from_secs)
    })
}

/// The `Set-Cookie` values of a response.
fn set_cookies(resp: &reqwest::Response) -> Vec<String> {
    resp.headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect()
}

/// The user closed the signature prompt: the request was not sent again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignatureDeclined;

impl fmt::Display for SignatureDeclined {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Not signed, not sent")
    }
}

impl std::error::Error for SignatureDeclined {}

/// Whether a request failed only because the user declined to sign it.
pub fn declined(error: &anyhow::Error) -> bool {
    error.downcast_ref::<SignatureDeclined>().is_some()
}

/// What a session's requests ask of the runtime that owns the session.
#[derive(Debug)]
pub enum HookRequest {
    /// The server refused `stale` (401): the session's current token,
    /// renewed first if it is still `stale`; `None` when renewing failed.
    Renew {
        stale: String,
        answer: tokio::sync::oneshot::Sender<Option<String>>,
    },
    /// The server wants request `id` signed: the user's signature, or
    /// `None` when they decline. Asked again (same `id`) after a wrong
    /// password, with `ask.error` saying so.
    Sign {
        id: u64,
        what: String,
        ask: crate::signature::Ask,
        answer: tokio::sync::oneshot::Sender<Option<Signature>>,
    },
    /// Request `id` was sent with a signature: how it ended.
    Signed {
        id: u64,
        what: String,
        result: std::result::Result<(), String>,
    },
}

/// The channel to the runtime behind a session's client: requests renew a
/// refused token and ask for signatures through it, as the web client's
/// shared fetch does.
#[derive(Clone, Debug)]
pub struct Hooks(tokio::sync::mpsc::UnboundedSender<HookRequest>);

impl Hooks {
    pub fn new(sender: tokio::sync::mpsc::UnboundedSender<HookRequest>) -> Self {
        Self(sender)
    }

    async fn renew(&self, stale: &str) -> Option<String> {
        let (answer, wait) = tokio::sync::oneshot::channel();
        self.0
            .send(HookRequest::Renew {
                stale: stale.to_owned(),
                answer,
            })
            .ok()?;
        wait.await.ok().flatten()
    }

    async fn sign(&self, id: u64, what: &str, ask: crate::signature::Ask) -> Option<Signature> {
        let (answer, wait) = tokio::sync::oneshot::channel();
        self.0
            .send(HookRequest::Sign {
                id,
                what: what.to_owned(),
                ask,
                answer,
            })
            .ok()?;
        wait.await.ok().flatten()
    }

    fn signed(&self, id: u64, what: &str, result: std::result::Result<(), String>) {
        let _ = self.0.send(HookRequest::Signed {
            id,
            what: what.to_owned(),
            result,
        });
    }
}

/// Ids for requests waiting for a signature, unique in the process.
fn next_sign_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The request in a few words for the signature prompt and its toasts.
pub fn describe_request(method: &Method, path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let parts: Vec<&str> = path
        .trim_start_matches("/api/v1/")
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let decode = |part: &str| percent_decode(part);
    match parts.as_slice() {
        ["flows", id, "start"] => format!("Start flow {}", decode(id)),
        ["flows", id, "stop"] => format!("Stop flow {}", decode(id)),
        ["recipes", id, "load"] => format!("Load recipe {}", decode(id)),
        ["alarms", id, "unshelf"] => format!("Unshelve alarm {}", decode(id)),
        ["data-sources", id, "write-policy"] => {
            format!("Change the write policy of tag {}", decode(id))
        }
        ["devices", id, "simulate"] => format!("Simulate device {}", decode(id)),
        ["devices", id, "go-live"] => format!("Put device {} back live", decode(id)),
        _ => format!("{method} {path}"),
    }
}

/// `body` with the signature added (`{"signature": …}` for a request
/// without one); `None` for a body that is not a JSON object.
pub fn with_signature(
    body: Option<serde_json::Value>,
    signature: &Signature,
) -> Option<serde_json::Value> {
    let mut body = body.unwrap_or_else(|| serde_json::json!({}));
    body.as_object_mut()?
        .insert("signature".into(), serde_json::to_value(signature).ok()?);
    Some(body)
}

/// Clones share one connection pool, so background tasks can each hold one.
#[derive(Clone)]
pub struct VoltrusApi {
    http: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:3000`.
    pub base: String,
    pub token: Option<String>,
    /// The refresh token of the last sign-in or renewal, sent only to renew
    /// the session and to sign out (the server then revokes it).
    pub refresh: Option<String>,
    /// How long the server lets the current token live (its cookie's
    /// `Max-Age`), when it said.
    pub token_ttl: Option<std::time::Duration>,
    /// The session's runtime, for renewals after a 401 and signatures.
    hooks: Option<Hooks>,
    /// Whether a request the server wants signed asks the user here
    /// (actions that ask through their own path turn it off).
    signs: bool,
}

impl VoltrusApi {
    pub fn new(base: &str) -> Self {
        Self {
            http: crate::tls::http_client(std::time::Duration::from_secs(10)),
            base: base.trim_end_matches('/').to_string(),
            token: None,
            refresh: None,
            token_ttl: None,
            hooks: None,
            signs: true,
        }
    }

    /// This client with the session runtime's `hooks`.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Hooks) -> Self {
        self.hooks = Some(hooks);
        self
    }

    /// A copy whose requests do not ask for a signature themselves: a
    /// refusal that wants one comes back as the error.
    #[must_use]
    pub fn unsigned(&self) -> Self {
        Self {
            signs: false,
            ..self.clone()
        }
    }

    fn req(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("not logged in"))?;
        Ok(self.req_with(method, path, token))
    }

    fn req_with(&self, method: Method, path: &str, token: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{path}", self.base))
            .header("Cookie", format!("{TOKEN_COOKIE}={token}"))
    }

    async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(ServerError::new(status, &body).into());
        }
        parse_body(&body)
    }

    /// Every JSON request of the session: a 401 renews the token once
    /// (through the runtime, which owns the refresh token) and sends again;
    /// a refusal that wants an electronic signature asks the user and sends
    /// again with it, until it is answered otherwise or declined.
    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        mut body: Option<serde_json::Value>,
    ) -> Result<T> {
        let mut token = self.token.clone().ok_or_else(|| anyhow!("not logged in"))?;
        let mut renewed = false;
        let mut signing: Option<(u64, String)> = None;
        loop {
            let mut request = self.req_with(method.clone(), path, &token);
            if let Some(body) = &body {
                request = request.json(body);
            }
            let resp = request.send().await?;
            let status = resp.status();
            let text = resp.text().await?;
            if status.is_success() {
                if let (Some(hooks), Some((id, what))) = (&self.hooks, &signing) {
                    hooks.signed(*id, what, Ok(()));
                }
                return parse_body(&text);
            }
            let error = ServerError::new(status, &text);
            if let Some(hooks) = &self.hooks {
                if status == reqwest::StatusCode::UNAUTHORIZED && !renewed {
                    renewed = true;
                    if let Some(fresh) = hooks.renew(&token).await {
                        token = fresh;
                        continue;
                    }
                }
                if self.signs
                    && let Some(ask) = crate::signature::ask_of(&error)
                {
                    let (id, what) = signing
                        .get_or_insert_with(|| (next_sign_id(), describe_request(&method, path)))
                        .clone();
                    let Some(signature) = hooks.sign(id, &what, ask).await else {
                        return Err(SignatureDeclined.into());
                    };
                    match with_signature(body.take(), &signature) {
                        Some(signed) => {
                            body = Some(signed);
                            continue;
                        }
                        None => {
                            hooks.signed(id, &what, Err(error.message.clone()));
                            return Err(error.into());
                        }
                    }
                }
                if let Some((id, what)) = &signing {
                    hooks.signed(*id, what, Err(error.message.clone()));
                }
            }
            return Err(error.into());
        }
    }

    /// `GET path` (e.g. `/api/v1/alarms/summary`), decoded as `T`.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.request(Method::GET, path, None).await
    }

    /// `method path` with `body` as JSON, the answer decoded as `T`.
    pub async fn send_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let body = serde_json::to_value(body)?;
        self.request(method, path, Some(body)).await
    }

    /// `method path` without a body (POST actions, DELETE), the answer
    /// decoded as `T`; ask for [`IgnoredAny`] when only success matters.
    pub async fn send_empty<T: DeserializeOwned>(&self, method: Method, path: &str) -> Result<T> {
        self.request(method, path, None).await
    }

    /// POST /api/v1/auth/login — captures the `voltrus_token` and
    /// `voltrus_refresh` cookies.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<LoginOk> {
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/login", self.base))
            .json(&LoginBody { username, password })
            .send()
            .await
            .map_err(|e| anyhow!("connect failed: {e}"))?;

        let set_cookies = set_cookies(&resp);
        let login: LoginOk = Self::decode(resp).await?;
        self.token_ttl = cookie_max_age(&set_cookies, TOKEN_COOKIE);

        self.token = Some(
            parse_cookie(&set_cookies, TOKEN_COOKIE)
                .ok_or_else(|| anyhow!("login ok but no {TOKEN_COOKIE} cookie set"))?,
        );
        self.refresh = parse_cookie(&set_cookies, REFRESH_COOKIE);
        Ok(login)
    }

    /// POST /api/v1/auth/logout with both cookies: the server revokes the
    /// refresh token, so it cannot mint new sessions after sign-out.
    pub async fn logout(&self) -> Result<()> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("not logged in"))?;
        let mut cookie = format!("{TOKEN_COOKIE}={token}");
        if let Some(refresh) = &self.refresh {
            cookie.push_str(&format!("; {REFRESH_COOKIE}={refresh}"));
        }
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/logout", self.base))
            .header("Cookie", cookie)
            .send()
            .await?;
        Self::decode::<IgnoredAny>(resp).await?;
        Ok(())
    }

    /// POST /api/v1/auth/refresh with the refresh cookie: the server
    /// revokes that refresh token and sets a new token and refresh token,
    /// which replace these. Renews a session without its password and
    /// without signing the user's other sessions out, as a new sign-in does.
    pub async fn refresh_session(&mut self) -> Result<()> {
        let Some(refresh) = self.refresh.as_deref() else {
            return Err(ServerError::new(
                reqwest::StatusCode::UNAUTHORIZED,
                "The session has no refresh token",
            )
            .into());
        };
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/refresh", self.base))
            .header("Cookie", format!("{REFRESH_COOKIE}={refresh}"))
            .send()
            .await
            .map_err(|e| anyhow!("connect failed: {e}"))?;
        let set_cookies = set_cookies(&resp);
        Self::decode::<IgnoredAny>(resp).await?;
        let token = parse_cookie(&set_cookies, TOKEN_COOKIE)
            .ok_or_else(|| anyhow!("refresh ok but no {TOKEN_COOKIE} cookie set"))?;
        let refresh = parse_cookie(&set_cookies, REFRESH_COOKIE)
            .ok_or_else(|| anyhow!("refresh ok but no {REFRESH_COOKIE} cookie set"))?;
        self.token_ttl = cookie_max_age(&set_cookies, TOKEN_COOKIE);
        self.token = Some(token);
        self.refresh = Some(refresh);
        Ok(())
    }

    /// POST /api/v1/auth/activity with the refresh cookie: the user did
    /// something, so a session with an idle timeout stays open. Sent only
    /// for real input, never on a timer.
    pub async fn record_activity(&self) -> Result<()> {
        let refresh = self
            .refresh
            .as_deref()
            .ok_or_else(|| anyhow!("not logged in"))?;
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/activity", self.base))
            .header("Cookie", format!("{REFRESH_COOKIE}={refresh}"))
            .send()
            .await?;
        Self::decode::<IgnoredAny>(resp).await?;
        Ok(())
    }

    /// GET /api/v1/data-sources
    pub async fn data_sources(&self) -> Result<Vec<DataSource>> {
        self.get_json("/api/v1/data-sources").await
    }

    /// GET /api/v1/alarms?status=..&limit=..
    pub async fn alarms(&self, status: &str, limit: i64) -> Result<Vec<Alarm>> {
        self.get_json(&format!("/api/v1/alarms?status={status}&limit={limit}"))
            .await
    }

    /// `POST path` without a body, or with `{"signature": …}` when there
    /// is one.
    async fn post_signed(&self, path: &str, signature: Option<&Signature>) -> Result<()> {
        match signature {
            Some(signature) => {
                self.send_json::<_, IgnoredAny>(Method::POST, path, &SignedBody { signature })
                    .await?;
            }
            None => {
                self.send_empty::<IgnoredAny>(Method::POST, path).await?;
            }
        }
        Ok(())
    }

    /// POST /api/v1/alarms/{id}/acknowledge, signed when the server asked.
    pub async fn ack_alarm(&self, id: i64, signature: Option<&Signature>) -> Result<()> {
        self.post_signed(&format!("/api/v1/alarms/{id}/acknowledge"), signature)
            .await
    }

    /// POST /api/v1/alarms: the server's acknowledge-all (the same route the
    /// web client and the Python SDK use), signed when the server asked.
    pub async fn ack_all(&self, signature: Option<&Signature>) -> Result<()> {
        self.post_signed("/api/v1/alarms", signature).await
    }

    /// POST /api/v1/alarms/{id}/shelf — hide an alarm for `duration_secs`,
    /// signed when the server asked.
    pub async fn shelve_alarm(
        &self,
        id: i64,
        duration_secs: u64,
        reason: &str,
        signature: Option<&Signature>,
    ) -> Result<()> {
        self.send_json::<_, IgnoredAny>(
            Method::POST,
            &format!("/api/v1/alarms/{id}/shelf"),
            &ShelveBody {
                duration_secs,
                reason,
                signature,
            },
        )
        .await?;
        Ok(())
    }

    /// POST /api/v1/alarms/{id}/unshelf
    pub async fn unshelve_alarm(&self, id: i64) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::POST, &format!("/api/v1/alarms/{id}/unshelf"))
            .await?;
        Ok(())
    }

    /// POST /api/v1/auth/change-password
    pub async fn change_password(&self, old_password: &str, new_password: &str) -> Result<()> {
        self.send_json::<_, IgnoredAny>(
            Method::POST,
            "/api/v1/auth/change-password",
            &ChangePasswordBody {
                old_password,
                new_password,
            },
        )
        .await?;
        Ok(())
    }

    /// GET /api/v1/history?range=..&keyed_by=id — the history of the tags
    /// with these `keys` (every tag when empty), keyed by data source id
    /// (tag keys are not unique).
    pub async fn history(
        &self,
        range: &str,
        keys: &[String],
    ) -> Result<std::collections::HashMap<String, HistorySeries>> {
        let mut path = format!("/api/v1/history?range={range}&keyed_by=id");
        if !keys.is_empty() {
            path.push_str("&sensor=");
            path.push_str(&crate::tag_api::query_value(&keys.join(",")));
        }
        Ok(self.get_json::<HistoryResponse>(&path).await?.data)
    }
}

/// `POST /api/v1/data-sources` answers `201 {"id": 12}`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct Created {
    pub id: i64,
}

/// `GET /api/v1/shortcuts`: the signed-in user's per-tag shortcuts,
/// `{"<data source id>": "Mod+Shift+K"}` in the web client's combo format.
/// A cleared shortcut has no entry (servers before SCADA-17 returned it as
/// an empty combo, which [`TagShortcuts::by_id`] still skips).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct TagShortcuts(pub std::collections::HashMap<String, Option<String>>);

impl TagShortcuts {
    /// The set shortcuts by data source id; empty and malformed entries
    /// are skipped.
    pub fn by_id(&self) -> std::collections::HashMap<i64, String> {
        self.0
            .iter()
            .filter_map(|(id, combo)| {
                let combo = combo.as_deref().filter(|combo| !combo.is_empty())?;
                Some((id.parse().ok()?, combo.to_owned()))
            })
            .collect()
    }
}

/// One row of `GET /api/v1/modbus-devices` (`api/modbus_devices.rs`), and
/// the `device` of `GET /api/v1/modbus-devices/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModbusDevice {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub unit_id: u8,
    pub interval_ms: u64,
    pub endianness: String,
    pub enabled: bool,
    /// In the list only.
    pub register_count: usize,
}

/// Mirrors `voltrus::models::ModbusRegister`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModbusRegister {
    pub id: i64,
    pub device_id: String,
    pub name: String,
    pub address: u16,
    /// `int16`, `uint16`, `int32`, `float32` or `float64`.
    pub register_type: String,
    /// `holding` or `input`.
    pub register_area: String,
    pub scale: f64,
    pub sort_order: i64,
    pub zero_based: bool,
}

/// `GET /api/v1/modbus-devices/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ModbusDeviceDetail {
    pub device: ModbusDevice,
    pub registers: Vec<ModbusRegister>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_decodes_the_compact_server_form() {
        // The shape `GET /api/v1/history` answers with (see api/data/handlers.rs).
        let body = r#"{
            "sensors": [], "range": "30m", "interval": 5000,
            "data": {
                "PLC01.voltage": {"values": [230.1, 230.4, 229.9], "start": 1700000000000, "interval": 5000, "count": 3},
                "PLC01.empty": {"values": [], "start": 1700000000000, "interval": 5000, "count": 0}
            }
        }"#;
        let decoded: HistoryResponse = serde_json::from_str(body).unwrap();
        let samples: Vec<_> = decoded.data["PLC01.voltage"].samples().collect();
        assert_eq!(
            samples,
            [
                (1_700_000_000_000, 230.1),
                (1_700_000_005_000, 230.4),
                (1_700_000_010_000, 229.9)
            ]
        );
        assert_eq!(decoded.data["PLC01.empty"].samples().count(), 0);
    }

    #[test]
    fn a_data_source_keeps_every_field_the_server_sends() {
        // One row of `GET /api/v1/data-sources` (models::DataSource).
        let body = r##"{
            "id": 42, "key": "frequency", "name": "Grid frequency", "unit": "Hz",
            "color": "#f97316", "min_val": 49.0, "max_val": 51.0, "warn_val": 50.4,
            "crit_val": 50.8, "base_val": 50.0, "sort_order": 7, "decimals": 2,
            "source_type": "modbus", "source_config": "{\"low_warn\":49.6}",
            "device_id": "inverter-2", "equipment_id": null, "chart_type": "gauge",
            "path": "plant/grid", "on_delay_secs": 5, "off_delay_secs": 10
        }"##;
        let tag: DataSource = serde_json::from_str(body).unwrap();
        assert_eq!((tag.id, tag.device_id.as_str()), (42, "inverter-2"));
        assert_eq!((tag.min_val, tag.max_val, tag.base_val), (49.0, 51.0, 50.0));
        assert_eq!(tag.path.as_deref(), Some("plant/grid"));
        assert_eq!((tag.on_delay_secs, tag.off_delay_secs), (5, 10));
        assert_eq!(tag.chart_type, "gauge");
        // An older server without the delays (or the tree path) still decodes.
        let old: DataSource =
            serde_json::from_str(r#"{"id": 1, "key": "a", "name": "A"}"#).unwrap();
        assert_eq!((old.on_delay_secs, old.path), (0, None));
    }

    #[test]
    fn empty_bodies_read_as_null() {
        assert!(parse_body::<IgnoredAny>("").is_ok());
        assert_eq!(parse_body::<Option<i64>>("  ").unwrap(), None);
        assert!(parse_body::<()>("").is_ok());
        assert_eq!(
            parse_body::<serde_json::Value>(r#"{"ok":true}"#).unwrap()["ok"],
            true
        );
        assert!(parse_body::<Vec<i64>>("").is_err(), "a list needs a body");
    }

    /// Answers one HTTP request with `answer`; the request's head.
    fn answer_once(answer: &'static str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let base = format!("http://{}", listener.local_addr().expect("address"));
        let server = std::thread::spawn(move || {
            let (mut connection, _) = listener.accept().expect("a request");
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") && connection.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let _ = connection.write_all(answer.as_bytes());
            String::from_utf8_lossy(&head).into_owned()
        });
        (base, server)
    }

    #[test]
    fn a_refresh_sends_the_refresh_token_and_keeps_both_new_ones() {
        let (base, server) = answer_once(
            "HTTP/1.1 200 OK\r\n\
             Set-Cookie: voltrus_token=t2; HttpOnly; Path=/\r\n\
             Set-Cookie: voltrus_refresh=r2; HttpOnly; Path=/\r\n\
             Content-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n\
             {\"ok\":true}",
        );
        let mut api = VoltrusApi::new(&base);
        api.token = Some("t1".into());
        api.refresh = Some("r1".into());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(api.refresh_session()).expect("renewed");
        let head = server.join().expect("the server ran");
        assert!(head.starts_with("POST /api/v1/auth/refresh "), "{head}");
        assert!(head.contains("voltrus_refresh=r1"), "{head}");
        assert_eq!(api.token.as_deref(), Some("t2"));
        assert_eq!(
            api.refresh.as_deref(),
            Some("r2"),
            "the old refresh token is revoked by the renewal"
        );

        let (base, server) = answer_once(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 24\r\nConnection: close\r\n\r\n\
             {\"error\":\"unauthorized\"}",
        );
        let mut api = VoltrusApi::new(&base);
        api.refresh = Some("revoked".into());
        let error = runtime
            .block_on(api.refresh_session())
            .expect_err("refused");
        server.join().expect("the server ran");
        assert!(refused(&error), "a refused renewal ends the session");
    }

    #[test]
    fn parses_token_and_refresh_from_set_cookie() {
        let raw = [
            "voltrus_refresh=abc123; HttpOnly; Path=/".to_string(),
            "voltrus_token=eyJhbGciOiJIUzI1NiJ9.payload.sig; HttpOnly; Secure; SameSite=Strict; Max-Age=900; Path=/".to_string(),
        ];
        assert_eq!(
            parse_cookie(&raw, TOKEN_COOKIE).as_deref(),
            Some("eyJhbGciOiJIUzI1NiJ9.payload.sig")
        );
        assert_eq!(
            parse_cookie(&raw, REFRESH_COOKIE).as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn decodes_percent_encoded_padding() {
        // The server percent-encodes '=' padding as %3D in Set-Cookie values.
        let raw = [
            "voltrus_token=eyJ1c2VybmFtZSI6ImFkbWluIn0%3D%3D.deadbeef; HttpOnly; Path=/"
                .to_string(),
        ];
        assert_eq!(
            parse_cookie(&raw, TOKEN_COOKIE).as_deref(),
            Some("eyJ1c2VybmFtZSI6ImFkbWluIn0==.deadbeef")
        );
    }

    #[test]
    fn returns_none_without_the_cookie() {
        let raw = ["session=xyz; Path=/".to_string()];
        assert!(parse_cookie(&raw, TOKEN_COOKIE).is_none());
        // A cookie whose name only starts the same is another cookie.
        let raw = ["voltrus_token_old=xyz; Path=/".to_string()];
        assert!(parse_cookie(&raw, TOKEN_COOKIE).is_none());
    }

    #[test]
    fn rejects_empty_cookie_value() {
        let raw = ["voltrus_token=; Path=/".to_string()];
        assert!(parse_cookie(&raw, TOKEN_COOKIE).is_none());
    }

    #[test]
    fn alarm_deserializes_server_shape() {
        let json = r#"{
            "id": 7, "source_key": "PLC01.voltage", "source_name": "Voltage",
            "severity": "critical", "priority": "high", "value": 245.5,
            "threshold": 240.0, "message": "over limit", "triggered_at": 1700000000,
            "resolved_at": null, "acknowledged_at": null, "acknowledged_by": null,
            "cooldown_until": null, "shelved_until": null, "shelved_by": null,
            "shelved_reason": null, "deadband": 0.0, "state": "active", "alarm_type": "high"
        }"#;
        let alarm: Alarm = serde_json::from_str(json).unwrap();
        assert_eq!(alarm.id, 7);
        assert_eq!(alarm.severity, "critical");
        assert_eq!(alarm.alarm_type, "high");
    }

    #[test]
    fn refused_credentials_are_told_apart_from_other_failures() {
        use reqwest::StatusCode;
        let error = |status: StatusCode, body: &str| -> anyhow::Error {
            ServerError::new(status, body).into()
        };
        let wrong = error(
            StatusCode::UNAUTHORIZED,
            r#"{"error":"Invalid credentials"}"#,
        );
        assert_eq!(wrong.to_string(), "Invalid credentials");
        assert!(refused(&wrong));
        assert!(refused(&error(
            StatusCode::FORBIDDEN,
            r#"{"error":"Account locked. Try again later."}"#
        )));
        assert!(!refused(&error(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"error":"Too many login attempts. Please try again later."}"#
        )));
        let down = error(StatusCode::BAD_GATEWAY, "upstream gone");
        assert_eq!(down.to_string(), "HTTP 502 Bad Gateway: upstream gone");
        assert!(!refused(&down));
        assert!(!refused(&anyhow!("connect failed: refused")));
    }
}

#[cfg(test)]
mod request_tests {
    use super::*;
    use std::io::{Read, Write};

    /// Answers each request on 127.0.0.1 with the next of `answers`
    /// (status, body), one connection each, and reports every request's
    /// head and body.
    fn server(
        answers: Vec<(u16, &'static str)>,
    ) -> (String, std::sync::mpsc::Receiver<(String, String)>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let (report, requests) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let Ok((mut connection, _)) = listener.accept() else {
                    return;
                };
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if connection.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                let mut request_body = vec![0_u8; length];
                let _ = connection.read_exact(&mut request_body);
                let _ = report.send((head, String::from_utf8_lossy(&request_body).into_owned()));
                let _ = write!(
                    connection,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        (url, requests)
    }

    fn signature() -> Signature {
        Signature {
            password: "secret".into(),
            meaning: "I approve".into(),
            reason: "batch 7".into(),
        }
    }

    const ASK: &str = r#"{"error": "Signature required", "code": "signature_required",
        "signature_required": {"action": "flow_start", "meaning": "I approve"}}"#;

    #[test]
    fn a_request_the_server_wants_signed_asks_and_goes_again_with_the_signature() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (url, requests) = server(vec![(428, ASK), (200, r#"{"ok": true}"#)]);
        let (hooks, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let mut api = VoltrusApi::new(&url).with_hooks(Hooks::new(hooks));
        api.token = Some("t1".into());
        let answer = runtime.spawn(async move {
            api.send_empty::<IgnoredAny>(Method::POST, "/api/v1/flows/pump%20A/start")
                .await
                .map(|_| ())
        });
        runtime.block_on(async {
            let Some(HookRequest::Sign {
                what, ask, answer, ..
            }) = asked.recv().await
            else {
                panic!("expected a signature request");
            };
            assert_eq!(what, "Start flow pump A");
            assert_eq!(ask.request.meaning, "I approve");
            let _ = answer.send(Some(signature()));
            let Some(HookRequest::Signed { what, result, .. }) = asked.recv().await else {
                panic!("expected the signed request's outcome");
            };
            assert_eq!((what.as_str(), result), ("Start flow pump A", Ok(())));
        });
        assert!(runtime.block_on(answer).unwrap().is_ok());
        let (_, first) = requests.recv().unwrap();
        assert!(first.is_empty(), "sent unsigned first");
        let (_, second) = requests.recv().unwrap();
        let body: serde_json::Value = serde_json::from_str(&second).unwrap();
        assert_eq!(body["signature"]["reason"], "batch 7");
        assert_eq!(body["signature"]["meaning"], "I approve");
    }

    #[test]
    fn a_declined_signature_fails_the_request_without_sending_it_again() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (url, requests) = server(vec![(428, ASK)]);
        let (hooks, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let mut api = VoltrusApi::new(&url).with_hooks(Hooks::new(hooks));
        api.token = Some("t1".into());
        let body = serde_json::json!({"writable": true});
        let answer = runtime.spawn(async move {
            api.send_json::<_, IgnoredAny>(
                Method::PATCH,
                "/api/v1/data-sources/3/write-policy",
                &body,
            )
            .await
            .map(|_| ())
        });
        runtime.block_on(async {
            let Some(HookRequest::Sign { answer, .. }) = asked.recv().await else {
                panic!("expected a signature request");
            };
            let _ = answer.send(None);
        });
        let error = runtime.block_on(answer).unwrap().unwrap_err();
        assert!(declined(&error));
        assert_eq!(requests.iter().count(), 1);
    }

    #[test]
    fn without_hooks_or_signing_the_refusal_is_the_answer() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (url, _requests) = server(vec![(428, ASK)]);
        let (hooks, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let mut api = VoltrusApi::new(&url)
            .with_hooks(Hooks::new(hooks))
            .unsigned();
        api.token = Some("t1".into());
        let error = runtime
            .block_on(api.send_empty::<IgnoredAny>(Method::POST, "/api/v1/alarms/1/acknowledge"))
            .unwrap_err();
        assert!(crate::signature::ask(&error).is_some());
        assert!(asked.try_recv().is_err(), "nobody was asked");
    }

    #[test]
    fn a_401_renews_once_and_sends_again_with_the_new_token() {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let (url, requests) = server(vec![
            (401, r#"{"error": "unauthorized"}"#),
            (200, "[]"),
            (401, r#"{"error": "unauthorized"}"#),
            (401, r#"{"error": "unauthorized"}"#),
        ]);
        let (hooks, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let mut api = VoltrusApi::new(&url).with_hooks(Hooks::new(hooks));
        api.token = Some("old".into());
        let renewer = runtime.spawn(async move {
            while let Some(request) = asked.recv().await {
                if let HookRequest::Renew { stale, answer } = request {
                    assert_eq!(stale, "old");
                    let _ = answer.send(Some("new".into()));
                }
            }
        });
        let tags: Vec<DataSource> = runtime.block_on(api.data_sources()).unwrap();
        assert!(tags.is_empty());
        let heads: Vec<String> = requests.iter().take(2).map(|(head, _)| head).collect();
        assert!(heads[0].contains("voltrus_token=old"));
        assert!(heads[1].contains("voltrus_token=new"), "{}", heads[1]);
        // Refused again after the renewal: one retry only, then the error.
        let error = runtime.block_on(api.data_sources()).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<ServerError>()
                .map(|e| e.status.as_u16()),
            Some(401)
        );
        renewer.abort();
    }

    #[test]
    fn requests_are_described_for_the_prompt() {
        assert_eq!(
            describe_request(&Method::POST, "/api/v1/recipes/4/load"),
            "Load recipe 4"
        );
        assert_eq!(
            describe_request(&Method::POST, "/api/v1/alarms/9/unshelf"),
            "Unshelve alarm 9"
        );
        assert_eq!(
            describe_request(&Method::POST, "/api/v1/devices/plc%201/go-live"),
            "Put device plc 1 back live"
        );
        assert_eq!(
            describe_request(&Method::PUT, "/api/v1/other?x=1"),
            "PUT /api/v1/other"
        );
        let signed = with_signature(None, &signature()).unwrap();
        assert_eq!(signed["signature"]["password"], "secret");
        assert!(with_signature(Some(serde_json::json!([1])), &signature()).is_none());
    }

    #[test]
    fn the_session_policy_and_token_lifetime_are_read() {
        let login: LoginOk = serde_json::from_str(
            r#"{"ok": true, "username": "ana", "role": "operator",
                "must_change_password": true, "password_expired": true,
                "session": {"idle_timeout_secs": 900, "absolute_timeout_secs": null}}"#,
        )
        .unwrap();
        assert!(login.password_expired);
        assert_eq!(login.session.idle_timeout_secs, Some(900));
        let old: LoginOk =
            serde_json::from_str(r#"{"ok": true, "username": "ana", "role": "admin"}"#).unwrap();
        assert_eq!(old.session, SessionPolicy::default());
        assert_eq!(old.session.activity_interval(), None);

        let every = |idle| {
            SessionPolicy {
                idle_timeout_secs: Some(idle),
                absolute_timeout_secs: None,
            }
            .activity_interval()
            .unwrap()
            .as_secs()
        };
        assert_eq!(every(900), 60, "at most once a minute");
        assert_eq!(every(120), 40, "a third of a short timeout");
        assert_eq!(every(1), 1);

        let cookies = vec![
            "voltrus_refresh=r; HttpOnly; Max-Age=604800; Path=/".to_owned(),
            "voltrus_token=t; HttpOnly; SameSite=Strict; Max-Age=300; Path=/".to_owned(),
        ];
        assert_eq!(
            cookie_max_age(&cookies, TOKEN_COOKIE),
            Some(std::time::Duration::from_secs(300))
        );
        assert_eq!(
            cookie_max_age(&["voltrus_token=t".to_owned()], TOKEN_COOKIE),
            None
        );
    }

    #[test]
    fn session_ends_read_as_the_web_client_says_them() {
        let idle = SessionPolicy {
            idle_timeout_secs: Some(600),
            absolute_timeout_secs: None,
        };
        let end = SessionEnd::from_code(Some("session_idle")).unwrap();
        assert_eq!(
            end.message(&idle),
            "You were signed out after 10 minutes of inactivity."
        );
        assert_eq!(
            end.message(&SessionPolicy::default()),
            "You were signed out after a period of inactivity."
        );
        assert_eq!(
            SessionEnd::from_code(Some("session_expired"))
                .unwrap()
                .message(&idle),
            "Your session reached its maximum length. Please sign in again."
        );
        assert_eq!(SessionEnd::from_code(Some("account_locked")), None);
    }
}
