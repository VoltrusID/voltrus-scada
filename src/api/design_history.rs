//! Designer version history over REST (docs/version-history.md): the change
//! log, blame, versions, diff, revert, draft/publish with an optional
//! approval step, and git sync. MCP tools call the same functions.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppError, AppState, AuthUser, RejectedJson};
use crate::edition::{ActionOrigin, SignedAction};
use crate::models::SignatureInput;
use crate::storage::LayoutBase;
use crate::storage::design_history::{
    self as h, Approval, ChangeFilter, DesignActor, DesignSettings, RevertObject, RevertOutcome,
};

/// A response extension the audit middleware appends to the audit entry
/// (`batch=<id>`), so the one line per save points at its change-log batch.
#[derive(Clone, Debug)]
pub struct AuditBatch(pub String);

/// The design-history source of a request: `api_key` for a key, else a
/// person (`user`), or the editor the client names in `X-Voltrus-Client`
/// (`designer`, `native-designer`).
pub fn actor(user: &AuthUser, api_key: bool, hint: Option<&str>, operation: &str) -> DesignActor {
    let source = if api_key || user.username.starts_with("api_key:") {
        "api_key"
    } else {
        match hint {
            Some("designer") => "designer",
            Some("native-designer" | "native_designer") => "native_designer",
            Some("editor") => "editor",
            _ => "user",
        }
    };
    DesignActor::new(user.username.clone(), source, operation)
}

/// [`actor`] for a call whose origin is known (MCP tools, equipment writes).
pub fn actor_for_origin(user: &AuthUser, origin: ActionOrigin, operation: &str) -> DesignActor {
    let source = match origin {
        ActionOrigin::Mcp => "mcp",
        ActionOrigin::ApiKey => "api_key",
        ActionOrigin::Flow => "flow",
        ActionOrigin::Session | ActionOrigin::Recipe => {
            if user.username.starts_with("api_key:") {
                "api_key"
            } else {
                "user"
            }
        }
    };
    DesignActor::new(user.username.clone(), source, operation)
}

/// [`actor`] for an HTTP request (reads `X-Voltrus-Client`).
pub fn request_actor(
    user: &AuthUser,
    api_key: bool,
    headers: &HeaderMap,
    operation: &str,
) -> DesignActor {
    let hint = headers
        .get("x-voltrus-client")
        .and_then(|v| v.to_str().ok());
    actor(user, api_key, hint, operation)
}

/// A JSON body that may be absent (empty or whitespace): its default then.
fn optional_body<T: serde::de::DeserializeOwned + Default>(bytes: &[u8]) -> Result<T, AppError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(bytes).map_err(|e| AppError::bad_request(format!("invalid body: {e}")))
}

fn rejected(status: StatusCode, body: Value) -> AppError {
    AppError(RejectedJson(status, body).into())
}

fn conflict(message: impl Into<String>, code: &str) -> AppError {
    rejected(
        StatusCode::CONFLICT,
        json!({"error": message.into(), "code": code}),
    )
}

fn with_batch(body: Value, batch: Option<&String>) -> Response {
    let mut resp = Json(body).into_response();
    if let Some(b) = batch {
        resp.extensions_mut().insert(AuditBatch(b.clone()));
    }
    resp
}

// ── History ──

#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    /// Object id (widget or connection id, type id, instance name, faceplate id).
    pub object: Option<String>,
    pub object_type: Option<String>,
    pub user: Option<String>,
    pub batch: Option<String>,
    pub before_seq: Option<i64>,
    pub limit: Option<i64>,
    /// Project-wide route only: one screen, or `""` for project objects.
    pub screen: Option<String>,
}

impl HistoryQuery {
    fn filter(self, screen: Option<String>) -> ChangeFilter {
        ChangeFilter {
            screen_id: screen,
            object_type: self.object_type.filter(|s| !s.is_empty()),
            object_id: self.object.filter(|s| !s.is_empty()),
            username: self.user.filter(|s| !s.is_empty()),
            batch: self.batch.filter(|s| !s.is_empty()),
            before_seq: self.before_seq,
            limit: self.limit,
        }
    }
}

pub async fn list_changes(state: &AppState, f: ChangeFilter) -> Result<Value, AppError> {
    let limit = f.limit.unwrap_or(100).clamp(1, 500);
    let rows = state
        .storage
        .design_tx(move |c| h::list_changes(c, &f))
        .await?;
    let next = (rows.len() as i64 >= limit)
        .then(|| rows.last().map(|r| r.seq))
        .flatten();
    let users: std::collections::BTreeSet<&str> =
        rows.iter().map(|r| r.username.as_str()).collect();
    Ok(json!({"changes": rows, "next_before_seq": next, "users": users}))
}

/// GET /api/v1/screens/{id}/history
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/history",
    params(
        ("id" = String, Path, description = "Screen id"),
        ("object" = Option<String>, Query, description = "Only this widget or connection id"),
        ("object_type" = Option<String>, Query, description = "widget | connection"),
        ("user" = Option<String>, Query, description = "Only changes by this user"),
        ("batch" = Option<String>, Query, description = "Only this batch"),
        ("before_seq" = Option<i64>, Query, description = "Paging: rows older than this seq"),
        ("limit" = Option<i64>, Query, description = "Rows, 1-500 (default 100)"),
    ),
    responses((status = 200, description = "`{changes: [...], next_before_seq, users}`, newest first")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn screen_history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(list_changes(&state, q.filter(Some(id))).await?))
}

/// GET /api/v1/design/history — change log of every object, or of the
/// project objects (`screen=` empty): equipment types, instances, faceplates.
#[utoipa::path(
    get,
    path = "/api/v1/design/history",
    params(
        ("screen" = Option<String>, Query, description = "A screen id; empty for project objects; absent for all"),
        ("object" = Option<String>, Query), ("object_type" = Option<String>, Query),
        ("user" = Option<String>, Query), ("batch" = Option<String>, Query),
        ("before_seq" = Option<i64>, Query), ("limit" = Option<i64>, Query),
    ),
    responses((status = 200, description = "`{changes: [...], next_before_seq, users}`")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn project_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Result<impl IntoResponse, AppError> {
    let screen = q.screen.clone();
    Ok(Json(list_changes(&state, q.filter(screen)).await?))
}

/// GET /api/v1/screens/{id}/blame
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/blame",
    params(("id" = String, Path, description = "Screen id")),
    responses((status = 200, description = "`{objects: [{object_type, object_id, exists, username, at_ms, batch, kind, changes, created_by, ...}]}`")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn screen_blame(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let objects = state.storage.design_tx(move |c| h::blame(c, &id)).await?;
    Ok(Json(json!({"objects": objects})))
}

/// GET /api/v1/screens/{id}/design-status
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/design-status",
    params(("id" = String, Path, description = "Screen id")),
    responses((status = 200, description = "Head seq, publishing settings, published version, changes since, pending requests")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn screen_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let status = state
        .storage
        .design_tx(move |c| h::screen_status(c, &id))
        .await?;
    Ok(Json(status))
}

// ── Versions ──

/// GET /api/v1/screens/{id}/versions
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/versions",
    params(("id" = String, Path, description = "Screen id")),
    responses((status = 200, description = "`{versions: [...]}`, newest first")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let versions = state
        .storage
        .design_tx(move |c| h::list_versions(c, &id))
        .await?;
    Ok(Json(json!({"versions": versions})))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateVersion {
    pub name: Option<String>,
    #[serde(default)]
    pub message: String,
}

const MAX_NAME: usize = 100;
const MAX_MESSAGE: usize = 2000;

fn check_text(name: Option<&str>, message: &str) -> Result<(), AppError> {
    if name.is_some_and(|n| n.chars().count() > MAX_NAME) {
        return Err(AppError::bad_request(format!(
            "name is longer than {MAX_NAME} characters"
        )));
    }
    if message.chars().count() > MAX_MESSAGE {
        return Err(AppError::bad_request(format!(
            "message is longer than {MAX_MESSAGE} characters"
        )));
    }
    Ok(())
}

fn version_error(e: anyhow::Error) -> AppError {
    if let Some(t) = e.downcast_ref::<h::NameTaken>() {
        return conflict(t.to_string(), "name_taken");
    }
    AppError(e)
}

pub async fn create_version(
    state: &AppState,
    user: &AuthUser,
    screen: &str,
    body: CreateVersion,
) -> Result<h::Version, AppError> {
    check_text(body.name.as_deref(), &body.message)?;
    let screen = screen.to_string();
    let username = user.username.clone();
    let v = state
        .storage
        .design_tx(move |c| {
            h::create_version(
                c,
                &screen,
                body.name.as_deref(),
                body.message.trim(),
                &username,
                "saved",
            )
        })
        .await
        .map_err(version_error)?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "design_version_create".into(),
        username: user.username.clone(),
        resource: format!("screen:{}", v.screen_id),
        details: Some(format!(
            "version={} id={} upto_seq={}",
            v.name, v.id, v.upto_seq
        )),
    });
    Ok(v)
}

/// POST /api/v1/screens/{id}/versions — record the current draft as a version.
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/versions",
    params(("id" = String, Path, description = "Screen id")),
    request_body(description = "`{name?: string (default v<n>), message?: string}`"),
    responses((status = 201, description = "The version"), (status = 409, description = "The name is taken")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn post_version(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    let body: CreateVersion = optional_body(&body)?;
    let v = create_version(&state, &user, &id, body).await?;
    Ok((StatusCode::CREATED, Json(json!({"version": v}))))
}

/// GET /api/v1/screens/{id}/versions/{v} — a version with its rows.
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/versions/{v}",
    params(("id" = String, Path, description = "Screen id"), ("v" = String, Path, description = "Version id or name")),
    responses((status = 200, description = "`{version, equipment, connections}`"), (status = 404)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_version(
    State(state): State<AppState>,
    Path((id, v)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
    let found = state
        .storage
        .design_tx(move |c| h::get_version(c, &id, &v))
        .await?;
    let Some((version, snap)) = found else {
        return Err(AppError::not_found("No such version"));
    };
    Ok(Json(json!({
        "version": version,
        "equipment": snap.equipment,
        "connections": snap.connections,
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct StateQuery {
    /// `draft`, `published`, `seq:<n>`, or a version id or name.
    pub at: Option<String>,
}

/// GET /api/v1/screens/{id}/state?at= — the rows of a screen at a state
/// (the History pane's read-only preview).
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/state",
    params(("id" = String, Path, description = "Screen id"), ("at" = Option<String>, Query, description = "draft | published | seq:<n> | version id or name")),
    responses((status = 200, description = "`{state, equipment, connections}`"), (status = 404)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_state(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StateQuery>,
) -> Result<impl IntoResponse, AppError> {
    let at = q.at.unwrap_or_else(|| "draft".into());
    let found = state
        .storage
        .design_tx(move |c| h::resolve_state(c, &id, &at))
        .await?;
    let Some((label, snap)) = found else {
        return Err(AppError::not_found("No such state"));
    };
    Ok(Json(json!({
        "state": label,
        "equipment": snap.equipment,
        "connections": snap.connections,
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct DiffQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    /// `false`/`0` leaves out the YAML.
    pub yaml: Option<String>,
}

/// The diff of two states of a screen, with both as project YAML.
pub async fn diff(
    state: &AppState,
    screen: &str,
    from: &str,
    to: &str,
    with_yaml: bool,
) -> Result<Value, AppError> {
    let (screen, from, to) = (screen.to_string(), from.to_string(), to.to_string());
    let out = state
        .storage
        .design_tx(move |c| {
            let Some((fl, fs)) = h::resolve_state(c, &screen, &from)? else {
                return Ok(Err(format!("No state '{from}'")));
            };
            let Some((tl, ts)) = h::resolve_state(c, &screen, &to)? else {
                return Ok(Err(format!("No state '{to}'")));
            };
            let mut body = json!({
                "from": fl,
                "to": tl,
                "diff": h::diff_layouts(&fs, &ts)?,
            });
            if with_yaml {
                body["yaml"] = json!({
                    "from": h::layout_yaml(c, &fs)?,
                    "to": h::layout_yaml(c, &ts)?,
                });
                body["rows"] = json!({
                    "from": {"equipment": fs.equipment, "connections": fs.connections},
                    "to": {"equipment": ts.equipment, "connections": ts.connections},
                });
            }
            Ok(Ok(body))
        })
        .await?;
    out.map_err(AppError::not_found)
}

/// GET /api/v1/screens/{id}/diff?from=&to= — per object: added, removed,
/// changed fields; with both states as project YAML (tags by ref) and rows.
#[utoipa::path(
    get,
    path = "/api/v1/screens/{id}/diff",
    params(
        ("id" = String, Path, description = "Screen id"),
        ("from" = Option<String>, Query, description = "draft | published | seq:<n> | version id or name (default published)"),
        ("to" = Option<String>, Query, description = "Same (default draft)"),
        ("yaml" = Option<String>, Query, description = "false: leave out YAML and rows"),
    ),
    responses((status = 200, description = "`{from, to, diff: {widgets, connections, summary}, yaml: {from, to}, rows}`"), (status = 404)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_diff(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> Result<impl IntoResponse, AppError> {
    let with_yaml = !matches!(q.yaml.as_deref(), Some("0" | "false" | "no"));
    let from = q.from.unwrap_or_else(|| "published".into());
    let to = q.to.unwrap_or_else(|| "draft".into());
    Ok(Json(diff(&state, &id, &from, &to, with_yaml).await?))
}

// ── Revert ──

/// Body of a revert: a whole screen (`to` or `to_seq`), one object
/// (`object` with `to_seq` or `to`), or several (`objects`).
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevertBody {
    pub to: Option<String>,
    pub to_seq: Option<i64>,
    pub object: Option<String>,
    pub object_type: Option<String>,
    #[serde(default)]
    pub objects: Vec<RevertObject>,
    pub base_seq: Option<i64>,
    pub base_revision: Option<String>,
    #[serde(default)]
    pub message: String,
}

pub async fn revert(
    state: &AppState,
    actor: DesignActor,
    screen: &str,
    body: RevertBody,
) -> Result<Value, AppError> {
    check_text(None, &body.message)?;
    let base = LayoutBase {
        revision: body.base_revision.clone(),
        seq: body.base_seq,
    };
    let mut objects = body.objects.clone();
    if let Some(o) = &body.object {
        objects.push(RevertObject {
            object_type: body.object_type.clone(),
            object_id: o.clone(),
            to_seq: body.to_seq,
            to: body.to.clone(),
        });
    }
    let whole = objects.is_empty();
    let target = match (&body.to, body.to_seq) {
        (_, Some(s)) => format!("seq:{s}"),
        (Some(t), None) => t.clone(),
        (None, None) if whole => {
            return Err(AppError::bad_request(
                "give `to` (version, published, seq:<n>) or `to_seq`, or `object`/`objects`",
            ));
        }
        _ => String::new(),
    };
    let note = if whole {
        format!("Reverted the screen to {target}")
    } else {
        format!(
            "Reverted {} to an earlier state",
            objects
                .iter()
                .map(|o| o.object_id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let note = if body.message.trim().is_empty() {
        note
    } else {
        format!("{note}: {}", body.message.trim())
    };
    let actor = actor.with_operation("revert").with_note(note);
    let screen_id = screen.to_string();
    let outcome = state
        .storage
        .design_tx(move |c| {
            if whole {
                h::revert_screen(c, &screen_id, &target, &base, &actor)
            } else {
                h::revert_objects(c, &screen_id, &objects, &base, &actor)
            }
        })
        .await?;
    match outcome {
        RevertOutcome::Done {
            batch,
            seq,
            revision,
            removed_connections,
            ungrouped,
        } => Ok(json!({
            "ok": true,
            "batch": batch,
            "seq": seq,
            "revision": revision,
            "removed_connections": removed_connections,
            "ungrouped": ungrouped,
        })),
        RevertOutcome::Dangling {
            connection,
            missing,
        } => Err(rejected(
            StatusCode::CONFLICT,
            json!({
                "error": format!(
                    "Connection '{connection}' would end on {} that no longer exist; restore {} too",
                    missing.join(", "),
                    if missing.len() == 1 { "it" } else { "them" }
                ),
                "code": "dangling_connection",
                "connection": connection,
                "missing": missing,
            }),
        )),
        RevertOutcome::NotFound(m) => Err(AppError::not_found(m)),
        RevertOutcome::Conflict(crate::storage::LayoutSave::Conflict {
            revision,
            seq,
            since,
        }) => Err(super::layouts::conflict_error(revision, seq, since)),
        RevertOutcome::Conflict(_) => Err(AppError(anyhow::anyhow!("unexpected revert outcome"))),
    }
}

/// POST /api/v1/screens/{id}/revert
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/revert",
    params(("id" = String, Path, description = "Screen id")),
    request_body(description = "`{to: version|published|seq:<n>}` or `{to_seq}` (whole screen); `{object, object_type?, to_seq|to}` (one object); `{objects: [{object_id, object_type?, to_seq|to}]}` (several, one batch). Optional `base_seq`/`base_revision`, `message`."),
    responses(
        (status = 200, description = "`{ok, batch, seq, revision, removed_connections, ungrouped}`"),
        (status = 404, description = "No such version, seq or object state"),
        (status = 409, description = "`code: dangling_connection` (restore the named widgets too) or `conflict` (stale base)"),
    ),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn post_revert(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RevertBody>,
) -> Result<Response, AppError> {
    let actor = request_actor(&user, api_key.is_some(), &headers, "revert");
    let out = revert(&state, actor, &id, body).await?;
    let batch = out["batch"].as_str().map(str::to_string);
    Ok(with_batch(out, batch.as_ref()))
}

/// POST /api/v1/design/revert — put one project object (equipment type,
/// equipment instance, faceplate) back as it was after change `to_seq`,
/// through the object's own write path (types re-apply to every instance).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRevert {
    pub object_type: String,
    pub object_id: String,
    pub to_seq: i64,
    #[serde(default)]
    pub signature: Option<Value>,
}

#[utoipa::path(
    post,
    path = "/api/v1/design/revert",
    request_body(description = "`{object_type: equipment_type|equipment_instance|faceplate, object_id, to_seq}`"),
    responses((status = 200), (status = 404), (status = 409)),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn post_project_revert(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Json(body): Json<ProjectRevert>,
) -> Result<Response, AppError> {
    let origin = super::signature::origin_of(&user, api_key.is_some());
    revert_project_object(&state, &user, origin, body).await
}

/// The state of a project object right after `to_seq`: the `after` of its
/// last row at or before it, or the `before` of its first row after it.
fn project_object_at(
    c: &rusqlite::Connection,
    object_type: &str,
    object_id: &str,
    to_seq: i64,
) -> anyhow::Result<Option<Option<Value>>> {
    use rusqlite::OptionalExtension;
    let at: Option<Option<String>> = c
        .query_row(
            "SELECT after FROM design_changes WHERE screen_id = '' AND object_type = ?1 AND object_id = ?2 AND seq <= ?3 ORDER BY seq DESC LIMIT 1",
            rusqlite::params![object_type, object_id, to_seq],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(a) = at {
        return Ok(Some(a.and_then(|t| serde_json::from_str(&t).ok())));
    }
    let next: Option<Option<String>> = c
        .query_row(
            "SELECT before FROM design_changes WHERE screen_id = '' AND object_type = ?1 AND object_id = ?2 AND seq > ?3 ORDER BY seq LIMIT 1",
            rusqlite::params![object_type, object_id, to_seq],
            |r| r.get(0),
        )
        .optional()?;
    Ok(next.map(|b| b.and_then(|t| serde_json::from_str(&t).ok())))
}

pub async fn revert_project_object(
    state: &AppState,
    user: &AuthUser,
    origin: ActionOrigin,
    body: ProjectRevert,
) -> Result<Response, AppError> {
    let (ty, id, seq) = (
        body.object_type.clone(),
        body.object_id.clone(),
        body.to_seq,
    );
    let target = state
        .storage
        .design_tx(move |c| project_object_at(c, &ty, &id, seq))
        .await?;
    let Some(target) = target else {
        return Err(AppError::not_found(format!(
            "No history of {} '{}'",
            body.object_type, body.object_id
        )));
    };
    let mut target = target;
    if body.object_type != h::FACEPLATE
        && let (Some(Value::Object(m)), Some(sig)) = (&mut target, &body.signature)
    {
        m.insert("signature".into(), sig.clone());
    }
    let resp = match (body.object_type.as_str(), target) {
        (h::FACEPLATE, Some(v)) => {
            let f: crate::faceplate::Faceplate = serde_json::from_value(v)
                .map_err(|e| AppError::bad_request(format!("stored faceplate: {e}")))?;
            let exists = state.storage.get_faceplate(&f.id).await?.is_some();
            let a = actor_for_origin(user, origin, "revert").with_note(format!(
                "Reverted faceplate {} to seq {}",
                f.id, body.to_seq
            ));
            state.storage.save_faceplate_as(&f, !exists, &a).await?;
            Json(json!({"ok": true})).into_response()
        }
        (h::FACEPLATE, None) => {
            let a = actor_for_origin(user, origin, "revert").with_note(format!(
                "Reverted faceplate {} to before it existed",
                body.object_id
            ));
            state
                .storage
                .delete_faceplate_as(&body.object_id, &a)
                .await?;
            Json(json!({"ok": true})).into_response()
        }
        (h::EQUIPMENT_TYPE, Some(v)) => {
            if state
                .storage
                .get_equipment_type(&body.object_id)
                .await?
                .is_some()
            {
                super::equipment_types::update_type_as(
                    state,
                    user,
                    origin,
                    &body.object_id,
                    false,
                    v,
                )
                .await
            } else {
                super::equipment_types::create_type_as(state, user, origin, v).await
            }
        }
        (h::EQUIPMENT_INSTANCE, Some(v)) => {
            if state
                .storage
                .get_equipment_instance(&body.object_id)
                .await?
                .is_some()
            {
                super::equipment_types::update_instance_as(
                    state,
                    user,
                    origin,
                    &body.object_id,
                    false,
                    v,
                )
                .await
            } else {
                super::equipment_types::create_instance_as(state, user, origin, false, v).await
            }
        }
        (h::EQUIPMENT_TYPE | h::EQUIPMENT_INSTANCE, None) => {
            return Err(conflict(
                "Reverting an equipment type or instance to before it existed deletes it: use DELETE on it instead",
                "use_delete",
            ));
        }
        (other, _) => {
            return Err(AppError::bad_request(format!(
                "object_type '{other}' cannot be reverted here (screen objects: POST /api/v1/screens/{{id}}/revert)"
            )));
        }
    };
    Ok(resp)
}

// ── Publish ──

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishBody {
    /// Publish this version (id or name); without it the current draft is
    /// recorded as a new version first.
    pub version: Option<String>,
    pub name: Option<String>,
    #[serde(default)]
    pub message: String,
    /// The draft must still be at this seq (else 409).
    pub base_seq: Option<i64>,
    pub signature: Option<SignatureInput>,
}

enum Prepared {
    Version(Box<h::Version>),
    Conflict(crate::storage::LayoutSave),
    Missing,
}

/// Publish a screen, or file a publish request when approval is required.
pub async fn publish(
    state: &AppState,
    user: &AuthUser,
    origin: ActionOrigin,
    screen: &str,
    body: PublishBody,
) -> Result<Value, AppError> {
    check_text(body.name.as_deref(), &body.message)?;
    let settings = state.storage.design_settings().await?;
    if !settings.publishing {
        return Err(conflict(
            "Publishing is turned off for this site: operators see the draft",
            "publishing_off",
        ));
    }
    let needs_approval = settings.approval == Approval::Required;
    let stage = if needs_approval { "request" } else { "publish" };
    let summary = diff(state, screen, "published", "draft", false).await?;
    let signed = super::signature::authorize_action(
        state,
        user,
        origin,
        SignedAction::DesignPublish,
        format!("screen:{screen}:publish"),
        json!({
            "stage": stage,
            "screen_id": screen,
            "version": body.version,
            "message": body.message,
            "changes": summary["diff"]["summary"],
        }),
        body.signature.as_ref(),
    )
    .await
    .map_err(signature_error)?;
    let signature_id = signed.as_ref().and_then(|s| s.id.clone());

    let screen_id = screen.to_string();
    let username = user.username.clone();
    let version_ref = body.version.clone();
    let name = body.name.clone();
    let message = body.message.trim().to_string();
    let base_seq = body.base_seq;
    let sig = signature_id.clone();
    let prepared = state
        .storage
        .design_tx(move |c| {
            if let Some(seq) = base_seq {
                let base = LayoutBase { revision: None, seq: Some(seq) };
                if let Some(conflict) = h::check_base(c, &screen_id, &base)? {
                    return Ok(Prepared::Conflict(conflict));
                }
            }
            let version = match &version_ref {
                Some(r) => match h::get_version(c, &screen_id, r)? {
                    Some((v, _)) => v,
                    None => return Ok(Prepared::Missing),
                },
                None => h::create_version(
                    c,
                    &screen_id,
                    name.as_deref(),
                    &message,
                    &username,
                    if needs_approval { "pending" } else { "saved" },
                )?,
            };
            if needs_approval {
                c.execute(
                    "UPDATE design_versions SET status = 'pending', requested_by = ?2, signature = COALESCE(?3, signature) WHERE id = ?1",
                    rusqlite::params![version.id, username, sig],
                )?;
                return Ok(Prepared::Version(Box::new(
                    h::get_version(c, &screen_id, &version.id.to_string())?
                        .map(|x| x.0)
                        .unwrap_or(version),
                )));
            }
            Ok(Prepared::Version(Box::new(h::publish_version(
                c,
                &screen_id,
                version.id,
                &username,
                sig.as_deref(),
            )?)))
        })
        .await
        .map_err(version_error)?;
    let version = match prepared {
        Prepared::Version(v) => *v,
        Prepared::Missing => return Err(AppError::not_found("No such version")),
        Prepared::Conflict(crate::storage::LayoutSave::Conflict {
            revision,
            seq,
            since,
        }) => {
            return Err(super::layouts::conflict_error(revision, seq, since));
        }
        Prepared::Conflict(_) => return Err(AppError(anyhow::anyhow!("unexpected"))),
    };
    let mut details = format!(
        "version={} id={} upto_seq={} stage={stage}",
        version.name, version.id, version.upto_seq
    );
    if let Some(s) = &signed {
        details.push_str(&s.audit_suffix());
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: if needs_approval {
            "design_publish_request".into()
        } else {
            "design_publish".into()
        },
        username: user.username.clone(),
        resource: format!("screen:{screen}"),
        details: Some(details),
    });
    if !needs_approval {
        after_publish(state, user, screen, &version);
    }
    Ok(json!({
        "status": if needs_approval { "pending" } else { "published" },
        "version": version,
    }))
}

fn after_publish(state: &AppState, user: &AuthUser, screen: &str, v: &h::Version) {
    let message = if v.message.is_empty() {
        format!("Publish {screen} {}", v.name)
    } else {
        format!("Publish {screen} {}: {}", v.name, v.message)
    };
    crate::design_git::after_publish(
        state.storage.clone(),
        state.auth_secret.clone(),
        crate::design_git::SyncReason {
            message,
            author: user.username.clone(),
        },
    );
}

/// A signature refusal as an `AppError` with the signature answer's status
/// and body (428 prompts the client's signature dialog).
fn signature_error(e: super::signature::SignatureError) -> AppError {
    rejected(e.status(), e.body())
}

/// POST /api/v1/screens/{id}/publish
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/publish",
    params(("id" = String, Path, description = "Screen id")),
    request_body(description = "`{version?: id|name, name?, message?, base_seq?, signature?}`"),
    responses(
        (status = 200, description = "`{status: published|pending, version}`"),
        (status = 409, description = "Publishing is off, the name is taken, or the draft moved past `base_seq`"),
        (status = 428, description = "The edition wants an electronic signature"),
    ),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn post_publish(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let origin = super::signature::origin_of(&user, api_key.is_some());
    let body: PublishBody = optional_body(&body)?;
    let out = publish(&state, &user, origin, &id, body).await?;
    Ok(Json(out).into_response())
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBody {
    #[serde(default)]
    pub note: String,
    pub signature: Option<SignatureInput>,
}

/// Approve (`approve: true`) or reject a pending publish request.
pub async fn review(
    state: &AppState,
    user: &AuthUser,
    origin: ActionOrigin,
    screen: &str,
    version: &str,
    approve: bool,
    body: ReviewBody,
) -> Result<Value, AppError> {
    check_text(None, &body.note)?;
    if user.role != "admin" || origin != ActionOrigin::Session {
        return Err(AppError::forbidden(
            "Only an administrator signed in may approve or reject a publish request",
        ));
    }
    let (screen_id, vref) = (screen.to_string(), version.to_string());
    let found = state
        .storage
        .design_tx(move |c| h::get_version(c, &screen_id, &vref))
        .await?;
    let Some((v, _)) = found else {
        return Err(AppError::not_found("No such version"));
    };
    if v.status != "pending" {
        return Err(conflict(
            format!(
                "Version {} is not waiting for approval ({})",
                v.name, v.status
            ),
            "not_pending",
        ));
    }
    if v.requested_by.as_deref() == Some(user.username.as_str()) {
        return Err(AppError::forbidden(
            "A publish request must be approved or rejected by someone other than its requester",
        ));
    }
    let mut signed = None;
    if approve {
        signed = super::signature::authorize_action(
            state,
            user,
            origin,
            SignedAction::DesignPublish,
            format!("screen:{screen}:publish"),
            json!({
                "stage": "approve",
                "screen_id": screen,
                "version": v.name,
                "requested_by": v.requested_by,
                "message": v.message,
            }),
            body.signature.as_ref(),
        )
        .await
        .map_err(signature_error)?;
    }
    let sig = signed.as_ref().and_then(|s| s.id.clone());
    let (screen_id, username, note, vid) = (
        screen.to_string(),
        user.username.clone(),
        body.note.clone(),
        v.id,
    );
    let out = state
        .storage
        .design_tx(move |c| {
            h::review_version(
                c,
                vid,
                if approve { "approved" } else { "rejected" },
                &username,
                note.trim(),
            )?;
            if approve {
                h::publish_version(c, &screen_id, vid, &username, sig.as_deref())
            } else {
                Ok(h::get_version(c, &screen_id, &vid.to_string())?
                    .map(|x| x.0)
                    .expect("version exists"))
            }
        })
        .await?;
    let mut details = format!(
        "version={} id={} requested_by={}",
        out.name,
        out.id,
        out.requested_by.clone().unwrap_or_default()
    );
    if let Some(s) = &signed {
        details.push_str(&s.audit_suffix());
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: if approve {
            "design_publish_approve".into()
        } else {
            "design_publish_reject".into()
        },
        username: user.username.clone(),
        resource: format!("screen:{screen}"),
        details: Some(details),
    });
    if approve {
        after_publish(state, user, screen, &out);
    }
    Ok(json!({"status": if approve { "published" } else { "rejected" }, "version": out}))
}

/// POST /api/v1/screens/{id}/versions/{v}/approve
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/versions/{v}/approve",
    params(("id" = String, Path, description = "Screen id"), ("v" = String, Path, description = "Version id or name")),
    request_body(description = "`{note?, signature?}`"),
    responses((status = 200, description = "`{status: published, version}`"), (status = 403, description = "Not an admin session, or the requester"), (status = 409, description = "Not pending"), (status = 428)),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn post_approve(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path((id, v)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let origin = super::signature::origin_of(&user, api_key.is_some());
    let out = review(&state, &user, origin, &id, &v, true, optional_body(&body)?).await?;
    Ok(Json(out).into_response())
}

/// POST /api/v1/screens/{id}/versions/{v}/reject
#[utoipa::path(
    post,
    path = "/api/v1/screens/{id}/versions/{v}/reject",
    params(("id" = String, Path, description = "Screen id"), ("v" = String, Path, description = "Version id or name")),
    request_body(description = "`{note?}`"),
    responses((status = 200, description = "`{status: rejected, version}`"), (status = 403), (status = 409)),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn post_reject(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path((id, v)): Path<(String, String)>,
    body: axum::body::Bytes,
) -> Result<Response, AppError> {
    let origin = super::signature::origin_of(&user, api_key.is_some());
    let out = review(&state, &user, origin, &id, &v, false, optional_body(&body)?).await?;
    Ok(Json(out).into_response())
}

// ── Settings ──

/// GET /api/v1/settings/design-history
#[utoipa::path(
    get,
    path = "/api/v1/settings/design-history",
    responses((status = 200, description = "`{publishing, approval, squash_window_secs, squash_after_days, keep_days}`")),
    tag = "Version history",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_settings(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    Ok(Json(state.storage.design_settings().await?))
}

/// PUT /api/v1/settings/design-history (admin)
#[utoipa::path(
    put,
    path = "/api/v1/settings/design-history",
    request_body(description = "`{publishing?, approval?: none|required, squash_window_secs?, squash_after_days?, keep_days?}`"),
    responses((status = 200), (status = 400)),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn put_settings(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let mut merged = serde_json::to_value(state.storage.design_settings().await?)?;
    let Some(obj) = body.as_object() else {
        return Err(AppError::bad_request("expected an object"));
    };
    for (k, v) in obj {
        merged[k] = v.clone();
    }
    let s: DesignSettings = serde_json::from_value(merged)
        .map_err(|e| AppError::bad_request(format!("invalid settings: {e}")))?;
    if s.squash_window_secs > 3600 {
        return Err(AppError::bad_request("squash_window_secs is at most 3600"));
    }
    state.storage.save_design_settings(&s).await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "design_history_settings".into(),
        username: user.username.clone(),
        resource: "settings:design_history".into(),
        details: Some(serde_json::to_string(&s)?),
    });
    Ok(Json(s))
}

// ── Git sync ──

/// GET /api/v1/design/git — configuration (the credential is never
/// returned, only whether one is stored) and the last sync.
#[utoipa::path(
    get,
    path = "/api/v1/design/git",
    responses((status = 200, description = "`{config, has_secret, status}`")),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn get_git(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let config = crate::design_git::load_config(&state.storage).await?;
    Ok(Json(json!({
        "config": config,
        "has_secret": crate::design_git::has_secret(&state.storage).await,
        "status": crate::design_git::last_status(&state.storage).await,
    })))
}

/// PUT /api/v1/design/git — `{config fields..., secret?: string|null}`;
/// `secret` absent keeps the stored one, `""`/null clears it.
#[utoipa::path(
    put,
    path = "/api/v1/design/git",
    request_body(description = "`{enabled, remote, branch, path, auth: none|token|ssh_key, username, push_on_publish, author_email_domain, work_dir, secret?}`"),
    responses((status = 200), (status = 400)),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn put_git(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(mut body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let Some(obj) = body.as_object_mut() else {
        return Err(AppError::bad_request("expected an object"));
    };
    let secret = obj.remove("secret");
    let mut merged = serde_json::to_value(crate::design_git::load_config(&state.storage).await?)?;
    for (k, v) in obj.iter() {
        merged[k] = v.clone();
    }
    let config: crate::design_git::GitConfig = serde_json::from_value(merged)
        .map_err(|e| AppError::bad_request(format!("invalid git settings: {e}")))?;
    crate::design_git::save_config(&state.storage, &config)
        .await
        .map_err(|e| AppError::bad_request(format!("{e:#}")))?;
    if let Some(secret) = secret {
        crate::design_git::save_secret(&state.storage, &state.auth_secret, secret.as_str()).await?;
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "design_git_settings".into(),
        username: user.username.clone(),
        resource: "settings:design_git".into(),
        details: Some(format!(
            "enabled={} remote={} branch={} auth={:?}",
            config.enabled, config.remote, config.branch, config.auth
        )),
    });
    Ok(Json(json!({
        "config": config,
        "has_secret": crate::design_git::has_secret(&state.storage).await,
    })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SyncBody {
    #[serde(default)]
    pub message: String,
}

/// POST /api/v1/design/git/sync — export, commit and push now.
#[utoipa::path(
    post,
    path = "/api/v1/design/git/sync",
    request_body(description = "`{message?}`"),
    responses((status = 200, description = "`{commit, committed, pushed, file, message}`"), (status = 502, description = "git failed")),
    tag = "Version history",
    security(("cookie_auth" = []))
)]
pub async fn post_git_sync(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    let body: SyncBody = optional_body(&body)?;
    let message = Some(body.message)
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| "Sync the project from Voltrus".to_string());
    check_text(None, &message)?;
    let reason = crate::design_git::SyncReason {
        message,
        author: user.username.clone(),
    };
    match crate::design_git::sync(&state.storage, &state.auth_secret, &reason).await {
        Ok(o) => {
            state.audit_logger.log(crate::security::AuditEvent {
                action: "design_git_sync".into(),
                username: user.username.clone(),
                resource: "design_git".into(),
                details: Some(format!(
                    "commit={} pushed={}",
                    o.commit.clone().unwrap_or_default(),
                    o.pushed
                )),
            });
            Ok(Json(serde_json::to_value(o)?))
        }
        Err(e) => Err(rejected(
            StatusCode::BAD_GATEWAY,
            json!({"error": format!("{e:#}"), "code": "git_failed"}),
        )),
    }
}
