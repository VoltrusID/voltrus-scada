use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::super::{AppError, AppState, AuthUser};
use crate::ai_copilot::provider::validate_base_url;
use crate::ai_copilot::{AiProviderType, AiRequest, key_store, resolve_provider};
use crate::models;

// ── Settings ──

#[utoipa::path(
    get,
    path = "/api/v1/ai/settings",
    responses((status = 200, description = "AI copilot settings")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_ai_settings(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let provider = state
        .storage
        .get_ai_setting("provider")
        .await?
        .unwrap_or_else(|| "byok_openai".into());
    let default_model = state.storage.get_ai_setting("default_model").await?;
    let base_url = state.storage.get_ai_setting("base_url").await?;

    // Never the key itself: a masked hint, so the admin can tell which key is set.
    let (api_key_set, api_key_preview) =
        match key_store::load(&state.storage, &state.auth_secret).await? {
            key_store::StoredKey::None => (false, String::new()),
            key_store::StoredKey::Readable(key) => (true, key_store::masked_hint(&key)),
            key_store::StoredKey::Unreadable => {
                (false, "stored key unreadable: enter it again".to_string())
            }
        };

    Ok(Json(models::AiSettingsResponse {
        provider,
        api_key_set,
        api_key_preview,
        default_model,
        base_url,
        hosted_ai_available: crate::edition::hosted_ai().is_some(),
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateAiSettingsBody {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    /// For `byok_openai_compatible`, e.g. `http://localhost:11434/v1`. An
    /// empty string removes it.
    pub base_url: Option<String>,
    /// Remove the stored API key.
    #[serde(default)]
    pub clear_api_key: bool,
}

#[utoipa::path(
    put,
    path = "/api/v1/ai/settings",
    responses((status = 200, description = "Settings updated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_ai_settings(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<UpdateAiSettingsBody>,
) -> Result<impl IntoResponse, AppError> {
    let stored_provider = state.storage.get_ai_setting("provider").await?;
    let stored_model = state.storage.get_ai_setting("default_model").await?;
    let stored_base_url = state.storage.get_ai_setting("base_url").await?;

    let provider = match &body.provider {
        Some(p) => Some(AiProviderType::from_str(p).ok_or_else(|| {
            AppError::bad_request(
                "Invalid provider. Use: byok_openai, byok_anthropic, byok_openai_compatible, voltrus_ai",
            )
        })?),
        None => None,
    };
    // The hosted provider exists only when an add-on edition registers it; a
    // stored `voltrus_ai` setting keeps parsing, but it cannot be chosen anew.
    if matches!(provider, Some(AiProviderType::VoltrusAi))
        && crate::edition::hosted_ai().is_none()
        && stored_provider.as_deref() != Some(AiProviderType::VoltrusAi.as_str())
    {
        return Err(AppError::bad_request(
            "The hosted Voltrus AI provider is not available in this edition. Use byok_openai, byok_anthropic or byok_openai_compatible.",
        ));
    }
    let base_url = match body.base_url.as_deref().map(str::trim) {
        Some("") => Some(String::new()),
        Some(url) => Some(validate_base_url(url).map_err(AppError::bad_request)?),
        None => None,
    };
    let new_key = body.api_key.as_deref().filter(|k| !k.is_empty());

    // Check the result before writing anything: an OpenAI-compatible
    // provider is unusable without a base URL and a model.
    let effective_provider = provider
        .as_ref()
        .map(AiProviderType::as_str)
        .or(stored_provider.as_deref());
    if effective_provider == Some(AiProviderType::ByokOpenaiCompatible.as_str()) {
        let effective_url = base_url.clone().or(stored_base_url);
        if effective_url.as_deref().is_none_or(str::is_empty) {
            return Err(AppError::bad_request(
                "The OpenAI-compatible provider needs a base URL, e.g. http://localhost:11434/v1",
            ));
        }
        let effective_model = body.default_model.clone().or(stored_model);
        if effective_model
            .as_deref()
            .is_none_or(|m| m.trim().is_empty())
        {
            return Err(AppError::bad_request(
                "The OpenAI-compatible provider needs a model name",
            ));
        }
    }

    // A key entered for one provider is not sent to another one (for example
    // an OpenAI key to a custom base URL): switching provider without
    // entering a new key removes the stored one.
    let provider_changed = match (&provider, stored_provider.as_deref()) {
        (Some(new), Some(old)) => new.as_str() != old,
        _ => false,
    };
    let key_cleared = new_key.is_none() && (body.clear_api_key || provider_changed);

    if let Some(p) = &provider {
        state.storage.set_ai_setting("provider", p.as_str()).await?;
    }
    if let Some(api_key) = new_key {
        key_store::save(&state.storage, &state.auth_secret, api_key).await?;
    } else if key_cleared {
        key_store::clear(&state.storage).await?;
    }
    if let Some(model) = &body.default_model {
        state
            .storage
            .set_ai_setting("default_model", model.trim())
            .await?;
    }
    match base_url.as_deref() {
        Some("") => state.storage.delete_ai_setting("base_url").await?,
        Some(url) => state.storage.set_ai_setting("base_url", url).await?,
        None => {}
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "ai_settings_updated".into(),
        username: auth.username.clone(),
        resource: "ai_copilot".into(),
        details: Some(
            json!({
                "provider": body.provider,
                "base_url": base_url,
                "api_key_cleared": key_cleared,
            })
            .to_string(),
        ),
    });

    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/settings/test",
    responses((status = 200, description = "Test result")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn test_ai_settings(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    // A missing base URL, model or key is reported like a failed connection.
    let settings = match super::helpers::load_ai_settings(&state).await {
        Ok(s) => s,
        Err(e) => return Ok(Json(json!({"ok": false, "error": e.0.to_string()}))),
    };
    let provider = resolve_provider(&settings);

    let request = AiRequest {
        system_prompt: "You are a test assistant. Reply with exactly: OK".into(),
        user_prompt: "Test connection".into(),
        max_tokens: Some(10),
        temperature: Some(0.0),
    };

    match provider.complete(request).await {
        Ok(resp) => Ok(Json(json!({
            "ok": true,
            "model": resp.model,
            "provider": resp.provider,
        }))),
        Err(e) => Ok(Json(json!({
            "ok": false,
            "error": e.to_string(),
        }))),
    }
}
