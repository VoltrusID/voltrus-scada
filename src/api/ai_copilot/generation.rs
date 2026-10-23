use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;

use super::super::{AppError, AppState, AuthUser};
use crate::ai_copilot::{self, AiRequest, resolve_provider};
use crate::models;

// ── Generation endpoints ──

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/register-map",
    responses((status = 200, description = "Register map generated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_register_map(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::RegisterMapRequest>,
) -> Result<impl IntoResponse, AppError> {
    let settings = super::helpers::load_ai_settings(&state).await?;
    let provider = resolve_provider(&settings);

    let user_prompt = ai_copilot::prompts::register_map_user_prompt(
        &ai_copilot::prompts::scrub_prompt(&body.file_content),
        &body.device_name,
    );

    let response = provider
        .complete(AiRequest {
            system_prompt: ai_copilot::prompts::register_map_system_prompt().into(),
            user_prompt,
            max_tokens: Some(4096),
            temperature: Some(0.2),
        })
        .await
        .map_err(|e| AppError(e.into()))?;

    let json_str = ai_copilot::parser::extract_json(&response.content)?;
    let registers = ai_copilot::parser::parse_register_map(&json_str)?;

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "register_map",
            &format!("Register map for {}", body.device_name),
            &serde_json::json!({"device_name": body.device_name, "file_type": body.file_type})
                .to_string(),
            &serde_json::to_string(&registers)?,
            body.device_id.as_deref(),
            &response.model,
            response.usage.total_tokens as i64,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/dashboard-layout",
    responses((status = 200, description = "Dashboard layout generated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_dashboard_layout(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::DashboardLayoutRequest>,
) -> Result<impl IntoResponse, AppError> {
    let settings = super::helpers::load_ai_settings(&state).await?;
    let provider = resolve_provider(&settings);

    // Fetch available data sources for context
    let data_sources = state.storage.list_data_sources(None).await?;
    let tags_str = data_sources
        .iter()
        .map(|ds| format!("{}: {} [{}]", ds.key, ds.name, ds.unit))
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt = ai_copilot::prompts::dashboard_layout_user_prompt(
        &ai_copilot::prompts::scrub_prompt(&body.description),
        &ai_copilot::prompts::scrub_prompt(&tags_str),
    );

    let response = provider
        .complete(AiRequest {
            system_prompt: ai_copilot::prompts::dashboard_layout_system_prompt().into(),
            user_prompt,
            max_tokens: Some(4096),
            temperature: Some(0.3),
        })
        .await
        .map_err(|e| AppError(e.into()))?;

    let json_str = ai_copilot::parser::extract_json(&response.content)?;
    let layout = ai_copilot::parser::parse_dashboard_layout(&json_str)?;

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "dashboard_layout",
            &format!(
                "Dashboard layout: {}",
                body.description.chars().take(60).collect::<String>()
            ),
            &serde_json::json!({"description": body.description}).to_string(),
            &serde_json::to_string(&layout)?,
            body.screen_id.as_deref(),
            &response.model,
            response.usage.total_tokens as i64,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/alarm-thresholds",
    responses((status = 200, description = "Alarm thresholds generated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_alarm_thresholds(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::AlarmThresholdRequest>,
) -> Result<impl IntoResponse, AppError> {
    let settings = super::helpers::load_ai_settings(&state).await?;
    let provider = resolve_provider(&settings);

    let tags_str = body
        .tags
        .iter()
        .map(|t| {
            format!(
                "{}: {} [{}] range {}-{}",
                t.source_key,
                t.name,
                t.unit.as_deref().unwrap_or(""),
                t.min_val.map(|v| v.to_string()).unwrap_or_default(),
                t.max_val.map(|v| v.to_string()).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt =
        ai_copilot::prompts::alarm_threshold_user_prompt(&body.device_type, &tags_str);

    let response = provider
        .complete(AiRequest {
            system_prompt: ai_copilot::prompts::alarm_threshold_system_prompt().into(),
            user_prompt,
            max_tokens: Some(2048),
            temperature: Some(0.2),
        })
        .await
        .map_err(|e| AppError(e.into()))?;

    let json_str = ai_copilot::parser::extract_json(&response.content)?;
    let thresholds = ai_copilot::parser::parse_alarm_thresholds(&json_str)?;

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "alarm_thresholds",
            &format!("Alarm thresholds for {}", body.device_type),
            &serde_json::json!({"device_type": body.device_type, "tag_count": body.tags.len()})
                .to_string(),
            &serde_json::to_string(&thresholds)?,
            None,
            &response.model,
            response.usage.total_tokens as i64,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/tag-rename",
    responses((status = 200, description = "Tag rename mapping generated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_tag_rename(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::TagRenameRequest>,
) -> Result<impl IntoResponse, AppError> {
    let settings = super::helpers::load_ai_settings(&state).await?;
    let provider = resolve_provider(&settings);

    let tags_str = body
        .tags
        .iter()
        .map(|t| t.current.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let user_prompt = ai_copilot::prompts::tag_rename_user_prompt(
        &tags_str,
        &body.convention,
        body.custom_pattern.as_deref(),
    );

    let response = provider
        .complete(AiRequest {
            system_prompt: ai_copilot::prompts::tag_rename_system_prompt().into(),
            user_prompt,
            max_tokens: Some(4096),
            temperature: Some(0.1),
        })
        .await
        .map_err(|e| AppError(e.into()))?;

    let json_str = ai_copilot::parser::extract_json(&response.content)?;
    let renames = ai_copilot::parser::parse_tag_rename(&json_str)?;

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "tag_rename",
            &format!("Tag rename to {} convention", body.convention),
            &serde_json::json!({"convention": body.convention, "tag_count": body.tags.len()})
                .to_string(),
            &serde_json::to_string(&renames)?,
            None,
            &response.model,
            response.usage.total_tokens as i64,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/flow-rule",
    responses((status = 200, description = "Flow rule generated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_flow_rule(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::FlowRuleRequest>,
) -> Result<impl IntoResponse, AppError> {
    let settings = super::helpers::load_ai_settings(&state).await?;
    let provider = resolve_provider(&settings);

    let tags_str = body.available_tags.join("\n");

    let user_prompt = ai_copilot::prompts::flow_rule_user_prompt(&body.description, &tags_str);

    let response = provider
        .complete(AiRequest {
            system_prompt: ai_copilot::prompts::flow_rule_system_prompt().into(),
            user_prompt,
            max_tokens: Some(2048),
            temperature: Some(0.2),
        })
        .await
        .map_err(|e| AppError(e.into()))?;

    let json_str = ai_copilot::parser::extract_json(&response.content)?;
    let flow = ai_copilot::parser::parse_flow_rule(&json_str)?;

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "flow_rule",
            &format!(
                "Flow rule: {}",
                body.description.chars().take(60).collect::<String>()
            ),
            &serde_json::json!({"description": body.description}).to_string(),
            &serde_json::to_string(&flow)?,
            None,
            &response.model,
            response.usage.total_tokens as i64,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}
