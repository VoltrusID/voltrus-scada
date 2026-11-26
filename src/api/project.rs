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
    /// `true`/`1`: publish every screen the import touched, in the same
    /// transaction (docs/version-history.md). Default off, so an import never
    /// changes what operators see unasked.
    pub publish: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    /// `true`/`1`: write each screen's published rows (what operators see)
    /// instead of the draft.
    pub published: Option<String>,
}

fn truthy(v: &Option<String>) -> bool {
    matches!(
        v.as_deref().map(str::to_ascii_lowercase).as_deref(),
        Some("true" | "1" | "yes")
    )
}

/// `merge` when absent or empty, else the named mode.
pub fn truthy_flag(v: &Option<String>) -> bool {
    truthy(v)
}

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
    export_yaml_with(state, false).await
}

/// The project YAML; with `published`, each screen's 2D rows are the ones
/// operators see (docs/version-history.md).
pub async fn export_yaml_with(state: &AppState, published: bool) -> anyhow::Result<String> {
    let doc = project_yaml::export::export_project_yaml_with(&state.storage, published).await?;
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
pub async fn export_project(
    State(state): State<AppState>,
    Query(q): Query<ExportQuery>,
) -> Result<Response, AppError> {
    let yaml = export_yaml_with(&state, truthy(&q.published)).await?;
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
    /// Refused before writing (publish not allowed, signature needed).
    Refused(AppError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Invalid(e) => e.fmt(f),
            ImportError::Unreadable(m) => f.write_str(m),
            ImportError::Failed(e) => write!(f, "{e:#}"),
            ImportError::Refused(e) => write!(f, "{:#}", e.0),
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
            ImportError::Refused(e) => e.into_response(),
        }
    }
}

fn find_invalid(e: &anyhow::Error) -> Option<&InvalidProject> {
    e.chain().find_map(|c| c.downcast_ref::<InvalidProject>())
}

/// How an applied import is attributed and whether it publishes.
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub origin: crate::edition::ActionOrigin,
    pub publish: bool,
    /// The electronic signature for publishing, when the edition wants one
    /// (REST: the `X-Voltrus-Signature` header, JSON).
    pub signature: Option<crate::models::SignatureInput>,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            origin: crate::edition::ActionOrigin::Session,
            publish: false,
            signature: None,
        }
    }
}

/// The checks of "publish after import", before anything is written: the
/// rules of a publish from the Designer (docs/version-history.md). Publishing
/// off refuses; with approval required the import files requests, which an
/// API key or a demo account may not; the edition's signature for
/// `design_publish` covers the document's hash.
async fn authorize_import_publish(
    state: &AppState,
    auth: &AuthUser,
    opts: &ImportOptions,
    mode: Mode,
    body: &str,
) -> Result<project_yaml::ImportPublish, AppError> {
    use crate::storage::design_history::Approval;
    let settings = state.storage.design_settings().await?;
    if !settings.publishing {
        return Err(AppError(
            super::RejectedJson(
                StatusCode::CONFLICT,
                json!({"error": project_yaml::PublishingOff.to_string(), "code": "publishing_off"}),
            )
            .into(),
        ));
    }
    let request = settings.approval == Approval::Required;
    if request {
        super::design_history::check_requester(auth, opts.origin)?;
    }
    let sha = {
        use sha2::{Digest, Sha256};
        Sha256::digest(body.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let signed = super::signature::authorize_action(
        state,
        auth,
        opts.origin,
        crate::edition::SignedAction::DesignPublish,
        "project:import:publish".into(),
        json!({
            "stage": if request { "request" } else { "publish" },
            "source": "import",
            "mode": format!("{mode:?}").to_lowercase(),
            "document_sha256": sha,
        }),
        opts.signature.as_ref(),
    )
    .await
    .map_err(|e| AppError(super::RejectedJson(e.status(), e.body()).into()))?;
    Ok(project_yaml::ImportPublish {
        signature: signed.and_then(|s| s.id),
    })
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
    opts: ImportOptions,
) -> Result<ImportPlan, ImportError> {
    let doc = project_yaml::parse_project_yaml(body).map_err(|e| match find_invalid(&e) {
        Some(invalid) => ImportError::Invalid(invalid.clone()),
        None => ImportError::Unreadable(format!("{e:#}")),
    })?;

    let publish = if opts.publish && !dry_run {
        Some(
            authorize_import_publish(state, auth, &opts, mode, body)
                .await
                .map_err(ImportError::Refused)?,
        )
    } else {
        None
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
        let actor = super::design_history::actor_for_origin(auth, opts.origin, "import")
            .with_note(format!("Project import ({mode:?})"));
        // Live sessions' edits are written before the import and its rows
        // merged into them after (docs/collaboration.md).
        state.collab.flush_all(state).await;
        let r = project_yaml::apply_import_as(&state.storage, &doc, mode, &actor, publish).await;
        state.collab.absorb_all(state).await;
        r
    };
    let plan = result.map_err(|e| match find_invalid(&e) {
        Some(invalid) => ImportError::Invalid(invalid.clone()),
        None if e.downcast_ref::<project_yaml::PublishingOff>().is_some() => {
            ImportError::Refused(AppError(
                super::RejectedJson(
                    StatusCode::CONFLICT,
                    json!({"error": e.to_string(), "code": "publishing_off"}),
                )
                .into(),
            ))
        }
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
                "mode={mode:?} create={} update={} delete={}{}{}{}",
                plan.totals.create,
                plan.totals.update,
                plan.totals.delete,
                plan.design_batch
                    .as_ref()
                    .map(|b| format!(" batch={b}"))
                    .unwrap_or_default(),
                if plan.published.is_empty() {
                    String::new()
                } else {
                    format!(" published={}", plan.published.join(","))
                },
                if plan.publish_requested.is_empty() {
                    String::new()
                } else {
                    format!(" publish_requested={}", plan.publish_requested.join(","))
                }
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
        ("publish" = Option<String>, Query, description = "true: publish the screens the import touched (or file publish requests when approval is required); refused while publishing is off. The signature an edition wants goes in the X-Voltrus-Signature header (JSON)"),
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
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Query(q): Query<ImportQuery>,
    headers: axum::http::HeaderMap,
    body: String,
) -> Result<Response, AppError> {
    let mode = parse_mode(q.mode.as_deref())?;
    let signature = match headers
        .get("x-voltrus-signature")
        .map(|v| v.to_str().map(str::to_string))
    {
        None => None,
        Some(Ok(text)) => Some(serde_json::from_str(&text).map_err(|e| {
            AppError::bad_request(format!("X-Voltrus-Signature is not a signature: {e}"))
        })?),
        Some(Err(_)) => return Err(AppError::bad_request("X-Voltrus-Signature is not text")),
    };
    let opts = ImportOptions {
        origin: super::signature::origin_of(&auth, api_key.is_some()),
        publish: truthy(&q.publish),
        signature,
    };
    Ok(
        match run_import(&state, &auth, &body, mode, truthy(&q.dry_run), opts).await {
            Ok(plan) => Json(plan).into_response(),
            Err(e) => e.into_response(),
        },
    )
}

/// Pollers run from a copy of the device configuration; bring them in line
/// with what the import wrote.
#[cfg(feature = "modbus")]
pub(crate) async fn sync_modbus_pollers(state: &AppState, plan: &project_yaml::ImportPlan) {
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
