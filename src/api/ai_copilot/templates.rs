use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::super::{AppError, AppState, AuthUser};
use crate::models;

// ── Templates ──

#[derive(Deserialize)]
pub struct ListTemplatesQuery {
    pub industry: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/ai/templates",
    responses((status = 200, description = "List equipment templates")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<ListTemplatesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let templates = state
        .storage
        .list_equipment_templates(query.industry.as_deref())
        .await?;
    Ok(Json(templates))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/generate/from-template",
    responses((status = 200, description = "Template instantiated")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn generate_from_template(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<models::TemplateInstantiateRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Look up template by ID via list + filter (no dedicated get method)
    let templates = state.storage.list_equipment_templates(None).await?;
    let template = templates
        .into_iter()
        .find(|t| t.id == body.template_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Template not found")))?;

    let prefix = body.prefix.as_deref().unwrap_or(&template.equipment_type);

    let config = json!({
        "template_id": template.id,
        "template_name": template.name,
        "prefix": prefix,
        "device_id": body.device_id,
        "tags": template.tag_template,
        "alarms": template.alarm_template,
        "dashboard": template.dashboard_template,
        "flows": template.flow_template,
    });

    let proposal_id = state
        .storage
        .create_ai_proposal(
            "equipment_template",
            &format!("Equipment template: {}", template.name),
            &json!({"template_id": template.id, "prefix": prefix}).to_string(),
            &config.to_string(),
            body.device_id.as_deref(),
            "template",
            0,
        )
        .await?;

    let proposal = state
        .storage
        .get_ai_proposal(proposal_id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Failed to create proposal")))?;

    Ok(Json(proposal))
}
