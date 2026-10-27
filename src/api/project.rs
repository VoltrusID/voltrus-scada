//! Declarative project export/import over HTTP (admin only).
//!
//! The format, validation and secret handling live in `crate::project_yaml`;
//! these handlers add the HTTP framing, a pre-import configuration snapshot and
//! the poller restarts a live server needs after devices changed. The MCP
//! project tools call [`export_yaml`] and [`run_import`], so both entry points
//! snapshot, import, resync pollers and audit the same way.

use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::project_yaml::{self, ImportPlan, InvalidProject, Mode};

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

/// `merge` when absent or empty, else the named mode.
pub fn parse_mode(mode: Option<&str>) -> Result<Mode, AppError> {
    match mode {
        None | Some("") => Ok(Mode::Merge),
        Some(m) => m
            .parse()
            .map_err(|e: anyhow::Error| AppError::bad_request(e.to_string())),
    }
}

/// The whole configuration as project YAML.
pub async fn export_yaml(state: &AppState) -> anyhow::Result<String> {
    let doc = project_yaml::export_project_yaml(&state.storage).await?;
    Ok(project_yaml::to_yaml(&doc))
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
    let yaml = export_yaml(&state).await?;
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

/// Why an import did not run.
pub enum ImportError {
    /// The YAML does not parse as a project, or the project fails validation.
    Invalid(InvalidProject),
    /// The text is not a project at all (YAML syntax, wrong format).
    Unreadable(String),
    /// The server failed while planning or writing.
    Failed(anyhow::Error),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Invalid(e) => e.fmt(f),
            ImportError::Unreadable(m) => f.write_str(m),
            ImportError::Failed(e) => write!(f, "{e:#}"),
        }
    }
}

impl IntoResponse for ImportError {
    fn into_response(self) -> Response {
        match self {
            ImportError::Invalid(e) => (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": e.to_string(),
                    "issues": e.0,
                })),
            )
                .into_response(),
            ImportError::Unreadable(m) => AppError::bad_request(m).into_response(),
            ImportError::Failed(e) => AppError(e).into_response(),
        }
    }
}

fn find_invalid(e: &anyhow::Error) -> Option<&InvalidProject> {
    e.chain().find_map(|c| c.downcast_ref::<InvalidProject>())
}

/// Parse `body` and plan (`dry_run`) or apply it. An applied import takes a
/// rollback snapshot first, restarts the Modbus pollers it touched and is
/// audit-logged; a dry run writes nothing.
pub async fn run_import(
    state: &AppState,
    auth: &AuthUser,
    body: &str,
    mode: Mode,
    dry_run: bool,
) -> Result<ImportPlan, ImportError> {
    let doc = project_yaml::parse_project_yaml(body).map_err(|e| match find_invalid(&e) {
        Some(invalid) => ImportError::Invalid(invalid.clone()),
        None => ImportError::Unreadable(format!("{e:#}")),
    })?;

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
    let plan = result.map_err(|e| match find_invalid(&e) {
        Some(invalid) => ImportError::Invalid(invalid.clone()),
        None => ImportError::Failed(e),
    })?;

    if !dry_run {
        #[cfg(feature = "modbus")]
        sync_modbus_pollers(state, &plan).await;

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
    Ok(plan)
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
    let mode = parse_mode(q.mode.as_deref())?;
    Ok(
        match run_import(&state, &auth, &body, mode, truthy(&q.dry_run)).await {
            Ok(plan) => Json(plan).into_response(),
            Err(e) => e.into_response(),
        },
    )
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
