//! Declarative project export/import over HTTP (admin only).
//!
//! The format, validation and secret handling live in `crate::project_yaml`;
//! these handlers add the HTTP framing, a pre-import configuration snapshot and
//! the poller restarts a live server needs after devices changed.

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::project_yaml::{self, InvalidProject, Mode};

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    /// `merge` (default) or `replace`.
    pub mode: Option<String>,
    /// `true`/`1` prints the plan and writes nothing.
    pub dry_run: Option<String>,
}

fn truthy(v: &Option<String>) -> bool {
    matches!(
        v.as_deref().map(str::to_ascii_lowercase).as_deref(),
        Some("true" | "1" | "yes")
    )
}

/// GET /api/v1/project/export -- whole configuration as one YAML document
#[utoipa::path(
    get,
    path = "/api/v1/project/export",
    responses(
        (status = 200, description = "Project YAML (format voltrus-project/1)", content_type = "application/yaml"),
        (status = 403),
    ),
    tag = "Project",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn export_project(State(state): State<AppState>) -> Result<Response, AppError> {
    let doc = project_yaml::export_project_yaml(&state.storage).await?;
    let yaml = project_yaml::to_yaml(&doc);
    Ok((
        [
            (header::CONTENT_TYPE, "application/yaml; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"project.yaml\"",
            ),
        ],
        yaml,
    )
        .into_response())
}

fn invalid_response(e: &InvalidProject) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({
            "error": e.to_string(),
            "issues": e.0,
        })),
    )
        .into_response()
}

fn find_invalid(e: &anyhow::Error) -> Option<&InvalidProject> {
    e.chain().find_map(|c| c.downcast_ref::<InvalidProject>())
}

/// POST /api/v1/project/import?mode=merge|replace&dry_run=true -- YAML body
#[utoipa::path(
    post,
    path = "/api/v1/project/import",
    params(
        ("mode" = Option<String>, Query, description = "merge (default) or replace"),
        ("dry_run" = Option<String>, Query, description = "true: return the plan, write nothing"),
    ),
    request_body(content = String, content_type = "application/yaml", description = "Project YAML (format voltrus-project/1)"),
    responses(
        (status = 200, description = "Plan (dry run) or result of the import", body = serde_json::Value),
        (status = 400, description = "Invalid project or unknown mode"),
        (status = 403),
    ),
    tag = "Project",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn import_project(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<ImportQuery>,
    body: String,
) -> Result<Response, AppError> {
    let mode: Mode = match q.mode.as_deref() {
        None | Some("") => Mode::Merge,
        Some(m) => m
            .parse()
            .map_err(|e: anyhow::Error| AppError::bad_request(e.to_string()))?,
    };
    let dry_run = truthy(&q.dry_run);

    let doc = match project_yaml::parse_project_yaml(&body) {
        Ok(d) => d,
        Err(e) => {
            return Ok(match find_invalid(&e) {
                Some(invalid) => invalid_response(invalid),
                None => AppError::bad_request(format!("{e:#}")).into_response(),
            });
        }
    };

    let result = if dry_run {
        project_yaml::plan_import(&state.storage, &doc, mode).await
    } else {
        // Rollback point: the snapshot covers the tables snapshots know about.
        let _ = state
            .storage
            .create_config_snapshot(
                Some("pre-import"),
                Some("Automatic snapshot before a project import"),
                "import",
                Some(&auth.username),
            )
            .await;
        project_yaml::apply_import(&state.storage, &doc, mode).await
    };
    let plan = match result {
        Ok(p) => p,
        Err(e) => {
            return Ok(match find_invalid(&e) {
                Some(invalid) => invalid_response(invalid),
                None => AppError(e).into_response(),
            });
        }
    };

    if !dry_run {
        #[cfg(feature = "modbus")]
        sync_modbus_pollers(&state, &plan).await;

        state.audit_logger.log(crate::security::AuditEvent {
            action: "project_import".to_string(),
            username: auth.username.clone(),
            resource: "project".to_string(),
            details: Some(format!(
                "mode={mode:?} create={} update={} delete={}",
                plan.totals.create, plan.totals.update, plan.totals.delete
            )),
        });
    }
    Ok(Json(plan).into_response())
}

/// Pollers run from a copy of the device configuration; bring them in line
/// with what the import wrote.
#[cfg(feature = "modbus")]
async fn sync_modbus_pollers(state: &AppState, plan: &project_yaml::ImportPlan) {
    use project_yaml::Action;
    let configs = state
        .storage
        .get_all_device_configs()
        .await
        .unwrap_or_default();
    for change in plan.changes_in("modbus_devices") {
        let id = &change.key;
        match change.action {
            Action::Delete => {
                state.poller_manager.stop(id).await;
                state.cache.remove(id).await;
            }
            Action::Create | Action::Update => match configs.iter().find(|c| &c.id == id) {
                Some(dc) => state.poller_manager.restart(dc.clone()).await,
                None => state.poller_manager.stop(id).await,
            },
        }
    }
}
