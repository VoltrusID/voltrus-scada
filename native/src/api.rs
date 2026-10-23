//! REST client for the voltrus server API (`/api/v1`).
//!
//! Auth: login sets an httpOnly `voltrus_token` cookie — not returned in the
//! body — so we parse it from `Set-Cookie` and replay it as a Cookie header.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

pub const TOKEN_COOKIE: &str = "voltrus_token";

#[derive(Debug, Clone, Serialize)]
struct LoginBody {
    username: String,
    password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginOk {
    pub ok: bool,
    pub username: String,
    pub role: String,
}

/// Mirrors `voltrus::models::DataSourceSummary` (server serializes exactly these fields).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DataSourceSummary {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub unit: String,
    pub color: String,
    #[serde(default)]
    pub warn_val: f64,
    #[serde(default)]
    pub crit_val: f64,
    #[serde(default)]
    pub decimals: i64,
    pub source_type: String,
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
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Extract the token value from raw `Set-Cookie` header values.
/// `voltrus_token=<jwt>; HttpOnly; SameSite=Strict; ...` → `<jwt>`
pub fn parse_token_cookie(set_cookie_values: &[String]) -> Option<String> {
    let prefix = format!("{TOKEN_COOKIE}=");
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

pub struct VoltrusApi {
    http: reqwest::Client,
    /// Base URL without trailing slash, e.g. `http://localhost:3000`.
    pub base: String,
    pub token: Option<String>,
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
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let token = self
            .token
            .as_deref()
            .ok_or_else(|| anyhow!("not logged in"))?;
        Ok(self
            .http
            .request(method, format!("{}{path}", self.base))
            .header("Cookie", format!("{TOKEN_COOKIE}={token}")))
    }

    async fn decode<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            // Server errors are `{"error": "..."}` JSON.
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(msg) = v.get("error").and_then(|e| e.as_str()) {
                    return Err(anyhow!("{msg}"));
                }
            }
            return Err(anyhow!("HTTP {status}: {body}"));
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// POST /api/v1/auth/login — captures the `voltrus_token` cookie.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<LoginOk> {
        let resp = self
            .http
            .post(format!("{}/api/v1/auth/login", self.base))
            .json(&LoginBody {
                username: username.to_string(),
                password: password.to_string(),
            })
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
            parse_token_cookie(&set_cookies)
                .ok_or_else(|| anyhow!("login ok but no {TOKEN_COOKIE} cookie set"))?,
        );
        Ok(login)
    }

    /// GET /api/v1/data-sources
    pub async fn data_sources(&self) -> Result<Vec<DataSourceSummary>> {
        let resp = self
            .req(reqwest::Method::GET, "/api/v1/data-sources")?
            .send()
            .await?;
        Self::decode(resp).await
    }

    /// GET /api/v1/alarms?status=..&limit=..
    pub async fn alarms(&self, status: &str, limit: i64) -> Result<Vec<Alarm>> {
        let resp = self
            .req(
                reqwest::Method::GET,
                &format!("/api/v1/alarms?status={status}&limit={limit}"),
            )?
            .send()
            .await?;
        Self::decode(resp).await
    }

    /// POST /api/v1/alarms/{id}/acknowledge
    pub async fn ack_alarm(&self, id: i64) -> Result<()> {
        let resp = self
            .req(
                reqwest::Method::POST,
                &format!("/api/v1/alarms/{id}/acknowledge"),
            )?
            .send()
            .await?;
        Self::decode::<serde_json::Value>(resp).await?;
        Ok(())
    }

    /// POST /api/v1/alarms/acknowledge-all
    pub async fn ack_all(&self) -> Result<()> {
        let resp = self
            .req(reqwest::Method::POST, "/api/v1/alarms/acknowledge-all")?
            .send()
            .await?;
        Self::decode::<serde_json::Value>(resp).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_token_from_set_cookie() {
        let raw = vec![
            "voltrus_refresh=abc123; HttpOnly; Path=/".to_string(),
            "voltrus_token=eyJhbGciOiJIUzI1NiJ9.payload.sig; HttpOnly; Secure; SameSite=Strict; Max-Age=900; Path=/".to_string(),
        ];
        assert_eq!(
            parse_token_cookie(&raw).as_deref(),
            Some("eyJhbGciOiJIUzI1NiJ9.payload.sig")
        );
    }

    #[test]
    fn decodes_percent_encoded_padding() {
        // The server percent-encodes '=' padding as %3D in Set-Cookie values.
        let raw = vec![
            "voltrus_token=eyJ1c2VybmFtZSI6ImFkbWluIn0%3D%3D.deadbeef; HttpOnly; Path=/"
                .to_string(),
        ];
        assert_eq!(
            parse_token_cookie(&raw).as_deref(),
            Some("eyJ1c2VybmFtZSI6ImFkbWluIn0==.deadbeef")
        );
    }

    #[test]
    fn returns_none_without_token_cookie() {
        let raw = vec!["session=xyz; Path=/".to_string()];
        assert!(parse_token_cookie(&raw).is_none());
    }

    #[test]
    fn rejects_empty_token_value() {
        let raw = vec!["voltrus_token=; Path=/".to_string()];
        assert!(parse_token_cookie(&raw).is_none());
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
}
