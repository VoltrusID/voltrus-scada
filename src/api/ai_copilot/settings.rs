use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::super::{AppError, AppState, AuthUser};
use crate::ai_copilot::{AiProviderType, AiRequest, resolve_provider};
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
    let api_key = state.storage.get_ai_setting("api_key").await?;
    let default_model = state.storage.get_ai_setting("default_model").await?;

    let api_key_preview = match &api_key {
        Some(key) if key.len() > 8 => {
            let last4 = &key[key.len() - 4..];
            format!("****{}", last4)
        }
        Some(_) => "****".into(),
        None => "".into(),
    };

    Ok(Json(models::AiSettingsResponse {
        provider,
        api_key_set: api_key.is_some(),
        api_key_preview,
        default_model,
    }))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct UpdateAiSettingsBody {
    pub provider: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
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
    if let Some(provider) = &body.provider {
        if AiProviderType::from_str(provider).is_none() {
            return Err(AppError(anyhow::anyhow!(
                "Invalid provider. Use: byok_openai, byok_anthropic, voltrus_ai"
            )));
        }
        state.storage.set_ai_setting("provider", provider).await?;
    }

    if let Some(api_key) = &body.api_key
        && !api_key.is_empty()
    {
        state.storage.set_ai_setting("api_key", api_key).await?;
    }

    if let Some(model) = &body.default_model {
        state.storage.set_ai_setting("default_model", model).await?;
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "ai_settings_updated".into(),
        username: auth.username.clone(),
        resource: "ai_copilot".into(),
        details: Some(json!({"provider": body.provider}).to_string()),
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
    let settings = super::helpers::load_ai_settings(&state).await?;
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
