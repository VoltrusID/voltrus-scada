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
    responses(
        (status = 200, description = "Get AI proposal"),
        (status = 404, description = "No such proposal"),
    ),
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
        .ok_or_else(|| AppError::not_found("Proposal not found"))?;

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

    if proposal.status == "applying" {
        return Err(conflict(
            "This proposal's last apply did not finish, so part of it may be in the configuration. Check the configuration; it will not be applied again.",
        ));
    }
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

    // Marked `applying` before the first write and settled after the last
    // one: a crash in between leaves it `applying`, which is never applied
    // again and is flagged at the next start.
    if !state.storage.begin_ai_proposal_apply(id).await? {
        return Err(conflict("The proposal is no longer approved"));
    }

    // Each apply goes through the REST handlers and is all or nothing.
    let target = &proposal.target_entity_id;
    let applied = match proposal.proposal_type.as_str() {
        "register_map" => super::helpers::apply_register_map(&state, config_to_apply, target).await,
        "alarm_thresholds" => super::helpers::apply_alarm_thresholds(&state, config_to_apply).await,
        "tag_rename" => super::helpers::apply_tag_rename(&state, config_to_apply).await,
        "dashboard_layout" => {
            super::helpers::apply_dashboard_layout(&state, config_to_apply, target).await
        }
        "flow_rule" => super::helpers::apply_flow_rule(&state, &auth, config_to_apply).await,
        "equipment_template" => {
            super::helpers::apply_equipment_template(&state, config_to_apply, target).await
        }
        other => Err(AppError::bad_request(format!(
            "Unknown proposal type: {other}"
        ))),
    };
    let audit = |action: &str, details: serde_json::Value| {
        let mut d = json!({
            "proposal_id": id,
            "proposal_type": proposal.proposal_type,
            "source": "ai-assist",
            "title": proposal.title,
        });
        if let (Some(d), Some(extra)) = (d.as_object_mut(), details.as_object()) {
            d.extend(extra.clone());
        }
        state.audit_logger.log(AuditEvent {
            action: action.into(),
            username: auth.username.clone(),
            resource: "ai_copilot".into(),
            details: Some(d.to_string()),
        });
    };

    let result = match applied {
        Ok(result) => result,
        Err(err) => {
            let message = format!("{:#}", err.0);
            return Err(match super::helpers::undo_failures(&err) {
                // Nothing was changed: the proposal can be fixed and applied again.
                None => {
                    audit(
                        "ai_proposal_apply_failed",
                        json!({"error": message, "undo": "nothing to undo or fully undone"}),
                    );
                    if let Err(e) = state
                        .storage
                        .end_ai_proposal_apply(id, "approved", None)
                        .await
                    {
                        tracing::error!(
                            "AI proposal {id}: could not set it back to approved: {e:#}"
                        );
                        AppError(anyhow::anyhow!(
                            "{message}. Nothing was changed, but the proposal could not be set back to approved ({e:#}); it stays 'applying' and is marked failed at the next start"
                        ))
                    } else {
                        err
                    }
                }
                // Part of it may be in the configuration: a person has to look.
                Some(failures) => {
                    audit(
                        "ai_proposal_apply_failed",
                        json!({"error": message, "undo": "failed", "undo_failures": failures}),
                    );
                    if let Err(e) = state
                        .storage
                        .end_ai_proposal_apply(id, "failed", Some(&message))
                        .await
                    {
                        tracing::error!("AI proposal {id}: could not mark it failed: {e:#}");
                    }
                    AppError(anyhow::anyhow!("{message}"))
                }
            });
        }
    };

    if let Err(e) = state
        .storage
        .mark_ai_proposal_applied(id, Some(&config_to_apply.to_string()))
        .await
    {
        // The changes are in; only the record of it failed. It must not be
        // applied a second time.
        let message = format!(
            "The proposal was applied, but its status could not be saved ({e:#}); it will not be applied again. Check the configuration."
        );
        tracing::error!("AI proposal {id}: {message}");
        audit(
            "ai_proposal_applied",
            json!({"result": result, "status_saved": false, "error": message}),
        );
        if let Err(e2) = state
            .storage
            .end_ai_proposal_apply(id, "failed", Some(&message))
            .await
        {
            tracing::error!("AI proposal {id}: could not mark it failed either: {e2:#}");
        }
        return Err(AppError(anyhow::anyhow!("{message}")));
    }

    audit("ai_proposal_applied", json!({"result": result}));

    Ok(Json(json!({"ok": true, "result": result})))
}

/// An error answered with 409 and `{"error": message}`.
fn conflict(message: &str) -> AppError {
    AppError(anyhow::Error::new(super::super::Rejected(
        axum::http::StatusCode::CONFLICT,
        message.to_string(),
    )))
}
