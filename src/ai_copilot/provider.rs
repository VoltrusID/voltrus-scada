use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::error::AiError;

#[derive(Debug, Clone)]
pub struct AiRequest {
    pub system_prompt: String,
    pub user_prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct AiResponse {
    pub content: String,
    pub model: String,
    pub usage: TokenUsage,
    pub provider: String,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProviderType {
    ByokOpenai,
    ByokAnthropic,
    /// Any server with an OpenAI-style `/chat/completions` endpoint at a
    /// configured base URL (Ollama, LM Studio, OpenRouter, ...).
    ByokOpenaiCompatible,
    VoltrusAi,
}

impl AiProviderType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "byok_openai" => Some(Self::ByokOpenai),
            "byok_anthropic" => Some(Self::ByokAnthropic),
            "byok_openai_compatible" => Some(Self::ByokOpenaiCompatible),
            "voltrus_ai" => Some(Self::VoltrusAi),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ByokOpenai => "byok_openai",
            Self::ByokAnthropic => "byok_anthropic",
            Self::ByokOpenaiCompatible => "byok_openai_compatible",
            Self::VoltrusAi => "voltrus_ai",
        }
    }
}

pub struct AiSettings {
    pub provider: AiProviderType,
    /// May be empty for `byok_openai_compatible` (local servers need no key).
    pub api_key: String,
    pub default_model: Option<String>,
    /// Only used by `byok_openai_compatible`.
    pub base_url: Option<String>,
}

impl AiSettings {
    /// Checks that the provider has what it needs before a request is made.
    pub fn validate(&self) -> Result<(), String> {
        match self.provider {
            AiProviderType::ByokOpenaiCompatible => {
                let base_url = self.base_url.as_deref().unwrap_or("").trim();
                if base_url.is_empty() {
                    return Err("The OpenAI-compatible provider needs a base URL".into());
                }
                validate_base_url(base_url)?;
                if self
                    .default_model
                    .as_deref()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
                {
                    return Err("The OpenAI-compatible provider needs a model name".into());
                }
                Ok(())
            }
            _ if self.api_key.is_empty() => Err("API key not configured".into()),
            _ => Ok(()),
        }
    }
}

/// Checks a base URL for the OpenAI-compatible provider and returns it trimmed.
/// Only http and https are accepted; http is fine for a model on the local network.
pub fn validate_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let url = reqwest::Url::parse(trimmed).map_err(|e| format!("Invalid base URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The base URL must start with http:// or https://".into());
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("The base URL has no host".into());
    }
    // The base URL is shown back to admins, so no credentials in it.
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Put the key in the API key field, not in the base URL".into());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("The base URL must not have a query string or fragment".into());
    }
    Ok(trimmed.to_string())
}

/// The chat completions endpoint for a base URL such as `http://localhost:11434/v1`.
/// A trailing slash, or a URL that already ends in `/chat/completions`, is accepted.
pub fn chat_completions_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    }
}

/// Enum-based dispatch to avoid dyn trait issues with async fn.
pub enum AiProviderImpl {
    OpenAi(OpenAiProvider),
    Anthropic(AnthropicProvider),
    /// Hosted service supplied by an add-on edition (see [`crate::edition::HostedAi`]).
    Hosted(HostedProvider),
}

/// A call to the hosted AI service registered by an add-on edition.
pub struct HostedProvider {
    pub api_key: String,
    pub model: String,
}

impl HostedProvider {
    pub async fn complete(&self, request: AiRequest) -> Result<AiResponse, AiError> {
        match crate::edition::hosted_ai() {
            Some(service) => service.complete(&self.api_key, &self.model, request).await,
            None => Err(AiError::ProviderError(
                "The hosted AI provider is not available in this edition. Use byok_openai, byok_anthropic or byok_openai_compatible."
                    .into(),
            )),
        }
    }

    pub fn provider_name(&self) -> &str {
        "voltrus_ai"
    }
}

impl AiProviderImpl {
    pub async fn complete(&self, request: AiRequest) -> Result<AiResponse, AiError> {
        match self {
            Self::OpenAi(p) => p.complete(request).await,
            Self::Anthropic(p) => p.complete(request).await,
            Self::Hosted(p) => p.complete(request).await,
        }
    }

    pub fn provider_name(&self) -> &str {
        match self {
            Self::OpenAi(p) => p.provider_name(),
            Self::Anthropic(p) => p.provider_name(),
            Self::Hosted(p) => p.provider_name(),
        }
    }
}

pub fn resolve_provider(settings: &AiSettings) -> AiProviderImpl {
    match settings.provider {
        AiProviderType::ByokOpenai => AiProviderImpl::OpenAi(OpenAiProvider {
            endpoint: OPENAI_CHAT_COMPLETIONS_URL.into(),
            api_key: settings.api_key.clone(),
            model: settings
                .default_model
                .clone()
                .unwrap_or_else(|| "gpt-4o-mini".into()),
            name: "openai",
        }),
        AiProviderType::ByokOpenaiCompatible => AiProviderImpl::OpenAi(OpenAiProvider {
            endpoint: chat_completions_url(settings.base_url.as_deref().unwrap_or("")),
            api_key: settings.api_key.clone(),
            model: settings.default_model.clone().unwrap_or_default(),
            name: "openai_compatible",
        }),
        AiProviderType::ByokAnthropic => AiProviderImpl::Anthropic(AnthropicProvider {
            api_key: settings.api_key.clone(),
            model: settings
                .default_model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".into()),
        }),
        AiProviderType::VoltrusAi => AiProviderImpl::Hosted(HostedProvider {
            api_key: settings.api_key.clone(),
            model: settings
                .default_model
                .clone()
                .unwrap_or_else(|| "voltrus-default".into()),
        }),
    }
}

// ── OpenAI and OpenAI-compatible ────────────────────────────────────────

const OPENAI_CHAT_COMPLETIONS_URL: &str = "https://api.openai.com/v1/chat/completions";

pub struct OpenAiProvider {
    /// Full chat completions URL.
    pub endpoint: String,
    /// Sent as a bearer token; no `Authorization` header when empty.
    pub api_key: String,
    pub model: String,
    /// `openai` or `openai_compatible`.
    pub name: &'static str,
}

// Compatible servers do not all send `model` or `usage`.
#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    /// `length` when the reply hit `max_tokens`.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize, Default)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

impl OpenAiProvider {
    pub async fn complete(&self, request: AiRequest) -> Result<AiResponse, AiError> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": request.system_prompt},
                {"role": "user", "content": request.user_prompt}
            ],
            "max_tokens": request.max_tokens.unwrap_or(2048),
            "temperature": request.temperature.unwrap_or(0.3),
        });

        let mut req = client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(60));
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let resp = req.json(&body).send().await?;

        let label = self.label();
        let status = resp.status();
        if status.is_client_error() || status.is_server_error() {
            let retry_after = resp.headers().get(reqwest::header::RETRY_AFTER).cloned();
            let text = resp.text().await.unwrap_or_default();
            return Err(AiError::from_provider_status(
                label,
                status,
                retry_after.as_ref(),
                &text,
            ));
        }

        let data: OpenAiResponse = resp.json().await.map_err(|e| {
            AiError::MalformedResponse(format!("Failed to parse {label} response: {e}"))
        })?;

        let choice = data.choices.into_iter().next();
        if choice
            .as_ref()
            .and_then(|c| c.finish_reason.as_deref())
            .is_some_and(|r| r == "length")
        {
            return Err(AiError::Truncated {
                max_tokens: request.max_tokens,
            });
        }
        let content = choice.and_then(|c| c.message.content).unwrap_or_default();
        let usage = data.usage.unwrap_or_default();

        Ok(AiResponse {
            content,
            model: data.model.unwrap_or_else(|| self.model.clone()),
            usage: TokenUsage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
            },
            provider: self.name.into(),
        })
    }

    pub fn provider_name(&self) -> &str {
        self.name
    }

    fn label(&self) -> &'static str {
        if self.name == "openai" {
            "OpenAI"
        } else {
            "OpenAI-compatible"
        }
    }
}

// ── Anthropic ───────────────────────────────────────────────────────────

pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: AnthropicUsage,
    /// `max_tokens` when the reply hit the output limit.
    #[serde(default)]
    stop_reason: Option<String>,
}

/// A content block; only `text` blocks carry the answer (others, such as
/// `thinking`, have no `text`).
#[derive(Deserialize)]
struct AnthropicContent {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

/// The answer text of a Messages API reply: its text blocks joined, or
/// [`AiError::Truncated`] when it stopped at the output limit.
fn anthropic_text(data: &AnthropicResponse, max_tokens: Option<u32>) -> Result<String, AiError> {
    if data.stop_reason.as_deref() == Some("max_tokens") {
        return Err(AiError::Truncated { max_tokens });
    }
    Ok(data
        .content
        .iter()
        .filter(|c| c.kind.as_deref().is_none_or(|k| k == "text"))
        .filter_map(|c| c.text.as_deref())
        .collect::<Vec<_>>()
        .join(""))
}

impl AnthropicProvider {
    pub async fn complete(&self, request: AiRequest) -> Result<AiResponse, AiError> {
        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": request.max_tokens.unwrap_or(2048),
            "system": request.system_prompt,
            "messages": [
                {"role": "user", "content": request.user_prompt}
            ],
        });

        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(60))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.is_client_error() || status.is_server_error() {
            let retry_after = resp.headers().get(reqwest::header::RETRY_AFTER).cloned();
            let text = resp.text().await.unwrap_or_default();
            return Err(AiError::from_provider_status(
                "Anthropic",
                status,
                retry_after.as_ref(),
                &text,
            ));
        }

        let data: AnthropicResponse = resp.json().await.map_err(|e| {
            AiError::MalformedResponse(format!("Failed to parse Anthropic response: {e}"))
        })?;

        let content = anthropic_text(&data, request.max_tokens)?;

        Ok(AiResponse {
            content,
            model: data.model,
            usage: TokenUsage {
                prompt_tokens: data.usage.input_tokens,
                completion_tokens: data.usage.output_tokens,
                total_tokens: data.usage.input_tokens + data.usage.output_tokens,
            },
            provider: "anthropic".into(),
        })
    }

    pub fn provider_name(&self) -> &str {
        "anthropic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use axum::http::HeaderMap;
    use std::sync::{Arc, Mutex};

    type Seen = Arc<Mutex<Vec<(Option<String>, serde_json::Value)>>>;

    fn compatible(base_url: Option<&str>, model: Option<&str>, key: &str) -> AiSettings {
        AiSettings {
            provider: AiProviderType::ByokOpenaiCompatible,
            api_key: key.into(),
            default_model: model.map(Into::into),
            base_url: base_url.map(Into::into),
        }
    }

    #[test]
    fn provider_strings_round_trip() {
        for p in [
            AiProviderType::ByokOpenai,
            AiProviderType::ByokAnthropic,
            AiProviderType::ByokOpenaiCompatible,
            AiProviderType::VoltrusAi,
        ] {
            assert_eq!(
                AiProviderType::from_str(p.as_str()).unwrap().as_str(),
                p.as_str()
            );
        }
        assert_eq!(
            AiProviderType::ByokOpenaiCompatible.as_str(),
            "byok_openai_compatible"
        );
        assert!(AiProviderType::from_str("openai_compatible").is_none());
    }

    #[test]
    fn chat_completions_url_joins_base_urls() {
        let want = "http://localhost:11434/v1/chat/completions";
        assert_eq!(chat_completions_url("http://localhost:11434/v1"), want);
        assert_eq!(chat_completions_url("http://localhost:11434/v1/"), want);
        assert_eq!(chat_completions_url(" http://localhost:11434/v1// "), want);
        assert_eq!(chat_completions_url(want), want);
        assert_eq!(chat_completions_url(&format!("{want}/")), want);
        assert_eq!(
            chat_completions_url("https://openrouter.ai/api/v1"),
            "https://openrouter.ai/api/v1/chat/completions"
        );
    }

    #[test]
    fn base_url_validation() {
        assert_eq!(
            validate_base_url(" http://localhost:11434/v1 ").unwrap(),
            "http://localhost:11434/v1"
        );
        assert!(validate_base_url("https://api.groq.com/openai/v1").is_ok());
        assert!(validate_base_url("http://192.168.1.20:1234/v1").is_ok());
        assert!(validate_base_url("ftp://example.com/v1").is_err());
        assert!(validate_base_url("file:///etc/passwd").is_err());
        assert!(validate_base_url("localhost:11434/v1").is_err());
        assert!(validate_base_url("not a url").is_err());
        assert!(validate_base_url("").is_err());
        assert!(validate_base_url("https://example.com/v1?key=x").is_err());
        assert!(validate_base_url("https://user:secret@example.com/v1").is_err());
    }

    #[test]
    fn settings_validation() {
        let ok = compatible(Some("http://localhost:11434/v1"), Some("llama3.2"), "");
        assert!(ok.validate().is_ok());
        assert!(compatible(None, Some("llama3.2"), "").validate().is_err());
        assert!(
            compatible(Some("http://localhost:11434/v1"), None, "")
                .validate()
                .is_err()
        );
        assert!(
            compatible(Some("http://localhost:11434/v1"), Some("  "), "")
                .validate()
                .is_err()
        );
        assert!(
            compatible(Some("gopher://x"), Some("m"), "")
                .validate()
                .is_err()
        );
        // The other BYOK providers still need a key.
        let openai = AiSettings {
            provider: AiProviderType::ByokOpenai,
            api_key: String::new(),
            default_model: None,
            base_url: None,
        };
        assert!(openai.validate().is_err());
    }

    #[test]
    fn resolve_uses_base_url_and_model() {
        let p = resolve_provider(&compatible(
            Some("http://localhost:11434/v1/"),
            Some("qwen2.5"),
            "",
        ));
        match p {
            AiProviderImpl::OpenAi(p) => {
                assert_eq!(p.endpoint, "http://localhost:11434/v1/chat/completions");
                assert_eq!(p.model, "qwen2.5");
                assert_eq!(p.provider_name(), "openai_compatible");
            }
            _ => panic!("expected the OpenAI request code"),
        }
        let openai = resolve_provider(&AiSettings {
            provider: AiProviderType::ByokOpenai,
            api_key: "sk-x".into(),
            default_model: None,
            base_url: Some("http://ignored".into()),
        });
        match openai {
            AiProviderImpl::OpenAi(p) => assert_eq!(p.endpoint, OPENAI_CHAT_COMPLETIONS_URL),
            _ => panic!("expected OpenAI"),
        }
    }

    /// A local server that answers like Ollama (no `usage`, no `model`) and
    /// records the Authorization header and body of each request.
    async fn mock_server() -> (String, Seen) {
        let seen: Seen = Arc::default();
        let recorder = seen.clone();
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let recorder = recorder.clone();
                    async move {
                        let auth = headers
                            .get("authorization")
                            .map(|v| v.to_str().unwrap().to_string());
                        recorder.lock().unwrap().push((auth, body));
                        Json(serde_json::json!({
                            "choices": [{"message": {"role": "assistant", "content": "OK"}}]
                        }))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/v1"), seen)
    }

    fn ping() -> AiRequest {
        AiRequest {
            system_prompt: "Reply OK".into(),
            user_prompt: "ping".into(),
            max_tokens: Some(1),
            temperature: Some(0.0),
        }
    }

    #[tokio::test]
    async fn compatible_request_without_key_sends_no_authorization() {
        let (base, seen) = mock_server().await;
        let provider = resolve_provider(&compatible(Some(&base), Some("llama3.2"), ""));
        let resp = provider.complete(ping()).await.unwrap();
        assert_eq!(resp.content, "OK");
        assert_eq!(resp.model, "llama3.2");
        assert_eq!(resp.provider, "openai_compatible");
        assert_eq!(resp.usage.total_tokens, 0);

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, None);
        assert_eq!(seen[0].1["model"], "llama3.2");
        assert_eq!(seen[0].1["max_tokens"], 1);
    }

    /// A server that answers every chat completion with `status`, `headers` and `body`.
    async fn canned_server(
        status: u16,
        headers: &'static [(&'static str, &'static str)],
        body: serde_json::Value,
    ) -> String {
        let app = axum::Router::new().route(
            "/v1/chat/completions",
            axum::routing::post(move || {
                let body = body.clone();
                async move {
                    let mut resp = axum::response::IntoResponse::into_response((
                        axum::http::StatusCode::from_u16(status).unwrap(),
                        Json(body),
                    ));
                    for (k, v) in headers {
                        resp.headers_mut().insert(*k, v.parse().unwrap());
                    }
                    resp
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}/v1")
    }

    #[tokio::test]
    async fn openai_reply_cut_at_max_tokens_is_truncated() {
        let cut = serde_json::json!({
            "choices": [{"message": {"content": "[{\"name\": \"v"}, "finish_reason": "length"}]
        });
        let base = canned_server(200, &[], cut).await;
        let provider = resolve_provider(&compatible(Some(&base), Some("m"), ""));
        let err = provider.complete(ping()).await.unwrap_err();
        assert!(
            matches!(
                err,
                AiError::Truncated {
                    max_tokens: Some(1)
                }
            ),
            "{err:?}"
        );

        let whole = serde_json::json!({
            "choices": [{"message": {"content": "[]"}, "finish_reason": "stop"}]
        });
        let base = canned_server(200, &[], whole).await;
        let provider = resolve_provider(&compatible(Some(&base), Some("m"), ""));
        assert_eq!(provider.complete(ping()).await.unwrap().content, "[]");
    }

    #[tokio::test]
    async fn provider_401_and_429_map_to_bad_key_and_rate_limit() {
        let denied = serde_json::json!({"error": {"message": "Incorrect API key provided"}});
        let base = canned_server(401, &[], denied).await;
        let provider = resolve_provider(&compatible(Some(&base), Some("m"), "sk-x"));
        let err = provider.complete(ping()).await.unwrap_err();
        assert!(matches!(err, AiError::InvalidApiKey(_)), "{err:?}");

        let limited = serde_json::json!({"error": {"message": "Rate limit reached"}});
        let base = canned_server(429, &[("retry-after", "7")], limited).await;
        let provider = resolve_provider(&compatible(Some(&base), Some("m"), "sk-x"));
        let err = provider.complete(ping()).await.unwrap_err();
        assert!(
            matches!(
                err,
                AiError::RateLimited {
                    retry_after_secs: Some(7)
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn anthropic_reply_joins_text_blocks_and_detects_max_tokens() {
        let data: AnthropicResponse = serde_json::from_value(serde_json::json!({
            "model": "claude",
            "usage": {"input_tokens": 1, "output_tokens": 2},
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": "..."},
                {"type": "text", "text": "[1,"},
                {"type": "text", "text": "2]"}
            ]
        }))
        .unwrap();
        assert_eq!(anthropic_text(&data, Some(10)).unwrap(), "[1,2]");

        let cut: AnthropicResponse = serde_json::from_value(serde_json::json!({
            "model": "claude",
            "usage": {"input_tokens": 1, "output_tokens": 2},
            "stop_reason": "max_tokens",
            "content": [{"type": "text", "text": "[1,"}]
        }))
        .unwrap();
        assert!(matches!(
            anthropic_text(&cut, Some(4096)),
            Err(AiError::Truncated {
                max_tokens: Some(4096)
            })
        ));
    }

    #[tokio::test]
    async fn compatible_request_with_key_sends_bearer() {
        let (base, seen) = mock_server().await;
        let provider = resolve_provider(&compatible(
            Some(&format!("{base}/chat/completions")),
            Some("deepseek-chat"),
            "sk-or-test",
        ));
        provider.complete(ping()).await.unwrap();
        assert_eq!(
            seen.lock().unwrap()[0].0.as_deref(),
            Some("Bearer sk-or-test")
        );
    }
}
