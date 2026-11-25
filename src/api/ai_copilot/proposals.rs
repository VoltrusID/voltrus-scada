use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::super::{AppError, AppState, AuthUser};
use crate::security::AuditEvent;

/// Approving, rejecting and applying a proposal run one at a time, each
/// reading the proposal's status under this lock: two clicks (or two
/// reviewers) cannot apply one proposal twice, or reject it while it is being
/// applied. Applies are rare and short, so one lock for all proposals is fine.
static REVIEW: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(Default::default);

// ── Proposals ──

#[derive(Deserialize)]
pub struct ListProposalsQuery {
    pub proposal_type: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/ai/proposals",
    responses((status = 200, description = "List AI proposals")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_proposals(
    State(state): State<AppState>,
    Query(query): Query<ListProposalsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let proposals = state
        .storage
        .list_ai_proposals(
            query.proposal_type.as_deref(),
            query.status.as_deref(),
            query.limit.unwrap_or(50),
        )
        .await?;

    Ok(Json(proposals))
}

#[utoipa::path(
    get,
    path = "/api/v1/ai/proposals/{id}",
    responses((status = 200, description = "Get AI proposal")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_proposal(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let proposal = state
        .storage
        .get_ai_proposal(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Proposal not found")))?;

    Ok(Json(proposal))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct ReviewProposalBody {
    pub applied_config: Option<serde_json::Value>,
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/proposals/{id}/approve",
    responses((status = 200, description = "Proposal approved")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn approve_proposal(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(body): Json<ReviewProposalBody>,
) -> Result<impl IntoResponse, AppError> {
    let _review = REVIEW.lock().await;
    let proposal = state
        .storage
        .get_ai_proposal(id)
        .await?
        .ok_or_else(|| AppError::not_found("Proposal not found"))?;
    if proposal.status != "pending" {
        return Err(AppError::bad_request(format!(
            "Only a pending proposal can be approved; this one is {}",
            proposal.status
        )));
    }
    // An edited configuration must be valid: approving the original instead
    // would apply something the reviewer did not mean.
    if let Some(edited) = &body.applied_config {
        crate::ai_copilot::parser::check_proposal_config(&proposal.proposal_type, edited).map_err(
            |e| AppError::bad_request(format!("The edited configuration is not valid: {e}")),
        )?;
    }
    let applied_config = body.applied_config.map(|v| v.to_string());
    state
        .storage
        .update_ai_proposal_status(
            id,
            "approved",
            Some(&auth.username),
            applied_config.as_deref(),
        )
        .await?;

    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/proposals/{id}/reject",
    responses((status = 200, description = "Proposal rejected")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn reject_proposal(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let _review = REVIEW.lock().await;
    let proposal = state
        .storage
        .get_ai_proposal(id)
        .await?
        .ok_or_else(|| AppError::not_found("Proposal not found"))?;
    // An applied proposal is in the configuration: rejecting it would not
    // undo it, only make the record say it never happened.
    if !matches!(proposal.status.as_str(), "pending" | "approved") {
        return Err(AppError::bad_request(format!(
            "Only a pending or approved proposal can be rejected; this one is {}",
            proposal.status
        )));
    }
    state
        .storage
        .update_ai_proposal_status(id, "rejected", Some(&auth.username), None)
        .await?;

    Ok(Json(json!({"ok": true})))
}

#[utoipa::path(
    post,
    path = "/api/v1/ai/proposals/{id}/apply",
    responses((status = 200, description = "Proposal applied to production")),
    tag = "AI Copilot",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn apply_proposal(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let _review = REVIEW.lock().await;
    let proposal = state
        .storage
        .get_ai_proposal(id)
        .await?
        .ok_or_else(|| AppError::not_found("Proposal not found"))?;

    if proposal.status != "approved" {
        return Err(AppError::bad_request(format!(
            "Only an approved proposal can be applied; this one is {}",
            proposal.status
        )));
    }

    let config_to_apply = proposal
        .applied_config
        .as_ref()
        .unwrap_or(&proposal.proposed_config);
    crate::ai_copilot::parser::check_proposal_config(&proposal.proposal_type, config_to_apply)
        .map_err(AppError::bad_request)?;

    // Each apply goes through the REST handlers and is all or nothing.
    let target = &proposal.target_entity_id;
    let result = match proposal.proposal_type.as_str() {
        "register_map" => {
            super::helpers::apply_register_map(&state, config_to_apply, target).await?
        }
        "alarm_thresholds" => {
            super::helpers::apply_alarm_thresholds(&state, config_to_apply).await?
        }
        "tag_rename" => super::helpers::apply_tag_rename(&state, config_to_apply).await?,
        "dashboard_layout" => {
            super::helpers::apply_dashboard_layout(&state, config_to_apply, target).await?
        }
        "flow_rule" => super::helpers::apply_flow_rule(&state, &auth, config_to_apply).await?,
        "equipment_template" => {
            super::helpers::apply_equipment_template(&state, config_to_apply, target).await?
        }
        other => {
            return Err(AppError::bad_request(format!(
                "Unknown proposal type: {other}"
            )));
        }
    };

    state
        .storage
        .mark_ai_proposal_applied(id, Some(&config_to_apply.to_string()))
        .await?;

    state.audit_logger.log(AuditEvent {
        action: "ai_proposal_applied".into(),
        username: auth.username.clone(),
        resource: "ai_copilot".into(),
        details: Some(
            json!({
                "proposal_id": id,
                "proposal_type": proposal.proposal_type,
                "source": "ai-assist",
                "title": proposal.title,
                "result": result,
            })
            .to_string(),
        ),
    });

    Ok(Json(json!({"ok": true, "result": result})))
}
