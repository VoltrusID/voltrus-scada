use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::api::{AppError, AppState, AuthUser};
use crate::vertical_templates::definitions;
use crate::vertical_templates::instantiator::Instantiator;

// ── Query / Request types ──

#[derive(Deserialize)]
pub struct ListTemplatesQuery {
    pub industry: Option<String>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct InstantiateRequest {
    /// Template ID (e.g. "wastewater_treatment")
    pub template_id: String,
    /// Human-readable name for this instance
    pub name: String,
    /// Optional instance counts for multi-instance patterns (e.g. {"n": 3} for 3 pumps)
    pub instance_counts: Option<HashMap<String, u32>>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct PreviewRequest {
    pub template_id: String,
    pub instance_counts: Option<HashMap<String, u32>>,
}

// ── Response types ──

#[derive(Serialize)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub industry: String,
    pub description: String,
    pub equipment_type: String,
    pub tag_count: usize,
    pub alarm_count: usize,
    pub flow_count: usize,
    pub has_oee: bool,
}

#[derive(Serialize)]
pub struct TemplateListResponse {
    pub templates: Vec<TemplateSummary>,
    pub industries: Vec<definitions::IndustryInfo>,
}

// ── Handlers ──

#[utoipa::path(
    get,
    path = "/api/v1/templates",
    responses((status = 200, description = "List vertical-specific templates")),
    tag = "Vertical Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_templates(
    State(_state): State<AppState>,
    Query(query): Query<ListTemplatesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let templates = match query.industry {
        Some(ref industry) => definitions::templates_by_industry(industry),
        None => definitions::all_templates(),
    };

    let summaries: Vec<TemplateSummary> = templates
        .iter()
        .map(|t| TemplateSummary {
            id: t.id.clone(),
            name: t.name.clone(),
            industry: t.industry.clone(),
            description: t.description.clone(),
            equipment_type: t.equipment_type.clone(),
            tag_count: t.tags.len(),
            alarm_count: t.alarms.len(),
            flow_count: t.flows.len(),
            has_oee: t.oee_config.is_some(),
        })
        .collect();

    Ok(Json(TemplateListResponse {
        templates: summaries,
        industries: definitions::industries(),
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/templates/{id}",
    responses((status = 200, description = "Get template details")),
    tag = "Vertical Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_template(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let template = definitions::get_template(&id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Template not found: {}", id)))?;

    Ok(Json(template))
}

#[utoipa::path(
    post,
    path = "/api/v1/templates/{id}/instantiate",
    responses((status = 200, description = "Template instantiated")),
    tag = "Vertical Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn instantiate_template(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(body): Json<InstantiateRequest>,
) -> Result<impl IntoResponse, AppError> {
    let template = definitions::get_template(&id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Template not found: {}", id)))?;

    let instance_counts = body
        .instance_counts
        .unwrap_or_else(Instantiator::default_instance_counts);

    let instantiator =
        Instantiator::new(&state.storage, template, body.name.clone(), instance_counts);

    let result = instantiator.instantiate().await.map_err(AppError)?;

    // Log to audit trail
    state.audit_logger.log(crate::security::AuditEvent {
        action: "template_instantiate".to_string(),
        username: _auth.username.clone(),
        resource: format!("vertical_template:{}", id),
        details: Some(format!("Instantiated {} as '{}'", id, body.name)),
    });

    Ok(Json(result))
}

#[utoipa::path(
    post,
    path = "/api/v1/templates/preview",
    responses((status = 200, description = "Preview expanded template")),
    tag = "Vertical Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn preview_template(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(body): Json<PreviewRequest>,
) -> Result<impl IntoResponse, AppError> {
    let template = definitions::get_template(&body.template_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Template not found: {}", body.template_id)))?;

    let instance_counts = body
        .instance_counts
        .unwrap_or_else(Instantiator::default_instance_counts);

    // Build a preview without actually creating anything
    let instantiator =
        Instantiator::new(&state.storage, template, "preview".into(), instance_counts);
    let expanded_tags = instantiator.expand_tag_patterns();
    let expanded_alarms = instantiator.expand_alarm_patterns(&expanded_tags);

    let flow_previews: Vec<serde_json::Value> = definitions::get_template(&body.template_id)
        .map(|t| {
            t.flows
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "name": f.name,
                        "description": f.description,
                        "trigger_type": f.trigger_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Json(serde_json::json!({
        "template_id": body.template_id,
        "expanded_tags": expanded_tags.iter().map(|t| serde_json::json!({
            "name": t.name_pattern,
            "description": t.description,
            "unit": t.unit,
            "data_type": t.data_type,
            "group": t.group,
            "widget": t.widget,
        })).collect::<Vec<_>>(),
        "expanded_alarms": expanded_alarms.iter().map(|a| serde_json::json!({
            "tag_pattern": a.tag_pattern,
            "severity": a.severity,
            "comparison": a.comparison,
            "threshold": a.threshold,
            "message_template": a.message_template,
        })).collect::<Vec<_>>(),
        "flows": flow_previews,
    })))
}
