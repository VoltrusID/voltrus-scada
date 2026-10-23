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
    VoltrusAi,
}

impl AiProviderType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "byok_openai" => Some(Self::ByokOpenai),
            "byok_anthropic" => Some(Self::ByokAnthropic),
            "voltrus_ai" => Some(Self::VoltrusAi),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ByokOpenai => "byok_openai",
            Self::ByokAnthropic => "byok_anthropic",
            Self::VoltrusAi => "voltrus_ai",
        }
    }
}

pub struct AiSettings {
    pub provider: AiProviderType,
    pub api_key: String,
    pub default_model: Option<String>,
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
                "The hosted AI provider is not available in this edition. Use byok_openai or byok_anthropic."
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
            api_key: settings.api_key.clone(),
            model: settings
                .default_model
                .clone()
                .unwrap_or_else(|| "gpt-4o-mini".into()),
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

// ── OpenAI ──────────────────────────────────────────────────────────────

pub struct OpenAiProvider {
    pub api_key: String,
    pub model: String,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    model: String,
    usage: OpenAiUsage,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    content: String,
}

#[derive(Deserialize)]
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

        let resp = client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .timeout(std::time::Duration::from_secs(60))
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.is_client_error() || status.is_server_error() {
            let text = resp.text().await.unwrap_or_default();
            return Err(AiError::ProviderError(format!(
                "OpenAI API error ({status}): {text}"
            )));
        }

        let data: OpenAiResponse = resp.json().await.map_err(|e| {
            AiError::MalformedResponse(format!("Failed to parse OpenAI response: {e}"))
        })?;

        let content = data
            .choices
            .into_iter()
            .next()
            .map(|c| c.message.content)
            .unwrap_or_default();

        Ok(AiResponse {
            content,
            model: data.model,
            usage: TokenUsage {
                prompt_tokens: data.usage.prompt_tokens,
                completion_tokens: data.usage.completion_tokens,
                total_tokens: data.usage.total_tokens,
            },
            provider: "openai".into(),
        })
    }

    pub fn provider_name(&self) -> &str {
        "openai"
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
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
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
            let text = resp.text().await.unwrap_or_default();
            return Err(AiError::ProviderError(format!(
                "Anthropic API error ({status}): {text}"
            )));
        }

        let data: AnthropicResponse = resp.json().await.map_err(|e| {
            AiError::MalformedResponse(format!("Failed to parse Anthropic response: {e}"))
        })?;

        let content = data
            .content
            .into_iter()
            .next()
            .map(|c| c.text)
            .unwrap_or_default();

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
