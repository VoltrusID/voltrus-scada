//! REST client for the voltrus server API (`/api/v1`).
//!
//! Auth: login sets httpOnly `voltrus_token` and `voltrus_refresh` cookies,
//! not returned in the body, so we parse them from `Set-Cookie` and replay
//! the token as a Cookie header (the refresh cookie only on sign-out, so the
//! server can revoke it).
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

/// A request the server answered with an error status.
#[derive(Debug, Clone)]
pub struct ServerError {
    pub status: reqwest::StatusCode,
    /// The server's `{"error": …}` text, or the status and body.
    pub message: String,
}

impl ServerError {
    fn new(status: reqwest::StatusCode, body: &str) -> Self {
        // Server errors are `{"error": "..."}` JSON.
        let message = serde_json::from_str::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("HTTP {status}: {body}"));
        Self { status, message }
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
    #[serde(default)]
    data: std::collections::HashMap<String, HistorySeries>,
}

#[derive(Debug, Serialize)]
struct ShelveBody<'a> {
    duration_secs: u64,
    reason: &'a str,
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

/// Percent-decode a cookie value: the server's `Set-Cookie` percent-encodes
/// the base64 `=` padding of the JWT payload as `%3D`, but the WebSocket
/// auth path expects the raw token. Browsers' cookie jars decode this
/// transparently; we must do the same.
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

/// Clones share one connection pool, so background tasks can each hold one.
#[derive(Clone)]
pub struct VoltrusApi {
    http: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:3000`.
    pub base: String,
    pub token: Option<String>,
    /// The refresh token of the last sign-in, sent only to sign out so the
    /// server revokes it.
    pub refresh: Option<String>,
}

impl VoltrusApi {
    pub fn new(base: &str) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            base: base.trim_end_matches('/').to_string(),
            token: None,
            refresh: None,
        }
    }

    fn req(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("not logged in"))?;
        Ok(self
            .http
            .request(method, format!("{}{path}", self.base))
            .header("Cookie", format!("{TOKEN_COOKIE}={token}")))
    }

    async fn decode<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(ServerError::new(status, &body).into());
        }
        parse_body(&body)
    }

    /// `GET path` (e.g. `/api/v1/alarms/summary`), decoded as `T`.
    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self.req(Method::GET, path)?.send().await?;
        Self::decode(resp).await
    }

    /// `method path` with `body` as JSON, the answer decoded as `T`.
    pub async fn send_json<B: Serialize + ?Sized, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self.req(method, path)?.json(body).send().await?;
        Self::decode(resp).await
    }

    /// `method path` without a body (POST actions, DELETE), the answer
    /// decoded as `T`; ask for [`IgnoredAny`] when only success matters.
    pub async fn send_empty<T: DeserializeOwned>(&self, method: Method, path: &str) -> Result<T> {
        let resp = self.req(method, path)?.send().await?;
        Self::decode(resp).await
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

        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
            .collect();

        let login: LoginOk = Self::decode(resp).await?;

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

    /// GET /api/v1/data-sources
    pub async fn data_sources(&self) -> Result<Vec<DataSource>> {
        self.get_json("/api/v1/data-sources").await
    }

    /// GET /api/v1/alarms?status=..&limit=..
    pub async fn alarms(&self, status: &str, limit: i64) -> Result<Vec<Alarm>> {
        self.get_json(&format!("/api/v1/alarms?status={status}&limit={limit}"))
            .await
    }

    /// POST /api/v1/alarms/{id}/acknowledge
    pub async fn ack_alarm(&self, id: i64) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::POST, &format!("/api/v1/alarms/{id}/acknowledge"))
            .await?;
        Ok(())
    }

    /// POST /api/v1/alarms: the server's acknowledge-all (the same route the
    /// web client and the Python SDK use).
    pub async fn ack_all(&self) -> Result<()> {
        self.send_empty::<IgnoredAny>(Method::POST, "/api/v1/alarms")
            .await?;
        Ok(())
    }

    /// POST /api/v1/alarms/{id}/shelf — hide an alarm for `duration_secs`.
    pub async fn shelve_alarm(&self, id: i64, duration_secs: u64, reason: &str) -> Result<()> {
        self.send_json::<_, IgnoredAny>(
            Method::POST,
            &format!("/api/v1/alarms/{id}/shelf"),
            &ShelveBody {
                duration_secs,
                reason,
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

    /// GET /api/v1/history?range=.. — every tag's history, keyed by tag key.
    pub async fn history(
        &self,
        range: &str,
    ) -> Result<std::collections::HashMap<String, HistorySeries>> {
        Ok(self
            .get_json::<HistoryResponse>(&format!("/api/v1/history?range={range}"))
            .await?
            .data)
    }
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
