//! Equipment types and instances (`/api/v1/equipment-types`,
//! `/api/v1/equipment-instances`), docs/equipment-types.md.
//!
//! Every write is a small project import (merge) built from the request:
//! the expansion in `project_yaml/equipment.rs` turns an instance into its
//! tags, registers, flows and widget bindings, and a changed type into the
//! re-expansion of all its instances, in one transaction. `dry_run=true`
//! answers what would change and writes nothing.

//!
//! Every write that is not a dry run takes a configuration snapshot first
//! (rollback point, as a project import does). A write whose expansion
//! changes the write policy of any tag needs the electronic signature the
//! write-policy route needs (`PATCH /data-sources/{id}/write-policy`), and is
//! applied only when the policy changes found inside the transaction are the
//! ones signed. The audit entry lists every tag attribute changed, before and
//! after.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppError, AppState, AuthUser, Rejected, RejectedJson};
use crate::edition::{ActionOrigin, SignedAction};
use crate::equipment_types::library;
use crate::equipment_types::resolve;
use crate::equipment_types::{EquipmentInstance, EquipmentType};
use crate::models::SignatureInput;
use crate::project_yaml::equipment::{self as expansion, Expanded};
use crate::project_yaml::{Conflict, ImportPlan, InvalidProject, Mode, ProjectDoc, Severity};
use crate::storage::equipment_types::TypeDelete;

/// A failure already answered (boxed: a response is large).
type Res<T> = Result<T, Box<Response>>;

fn resp(e: AppError) -> Box<Response> {
    Box::new(e.into_response())
}

fn conflict(message: impl Into<String>) -> AppError {
    AppError(anyhow::Error::new(Rejected(
        StatusCode::CONFLICT,
        message.into(),
    )))
}

#[derive(Debug, Default, Deserialize)]
pub struct DryRunQuery {
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct InstanceQuery {
    #[serde(rename = "type")]
    pub type_id: Option<String>,
}

#[derive(Debug, Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    /// Library type ids; all when left out.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
}

fn audit(
    state: &AppState,
    auth: &AuthUser,
    action: &str,
    resource: String,
    details: Option<String>,
) {
    state.audit_logger.log(crate::security::AuditEvent {
        action: action.to_string(),
        username: auth.username.clone(),
        resource,
        details,
    });
}

/// The `signature` a body may carry (added by the client after a 428).
fn take_signature(body: &mut Value) -> Result<Option<SignatureInput>, AppError> {
    match body.as_object_mut().and_then(|o| o.remove("signature")) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v)
            .map(Some)
            .map_err(|e| AppError::bad_request(format!("signature: {e}"))),
    }
}

fn parse_type(body: Value) -> Result<EquipmentType, AppError> {
    let mut body = body;
    if let Some(o) = body.as_object_mut() {
        // What GET adds, so a read-modify-write passes.
        o.remove("instance_count");
    }
    let t: EquipmentType = serde_json::from_value(body)
        .map_err(|e| AppError::bad_request(format!("Not an equipment type: {e}")))?;
    let problems = t.problems();
    if !problems.is_empty() {
        return Err(AppError::bad_request(format!(
            "Invalid equipment type: {}",
            problems.join("; ")
        )));
    }
    Ok(t)
}

fn parse_instance(body: Value) -> Result<EquipmentInstance, AppError> {
    let mut body = body;
    if let Some(o) = body.as_object_mut() {
        for k in ["resolved", "changes", "warnings"] {
            o.remove(k);
        }
    }
    let i: EquipmentInstance = serde_json::from_value(body)
        .map_err(|e| AppError::bad_request(format!("Not an equipment instance: {e}")))?;
    let problems = i.problems(None);
    if !problems.is_empty() {
        return Err(AppError::bad_request(format!(
            "Invalid equipment instance: {}",
            problems.join("; ")
        )));
    }
    Ok(i)
}

fn doc_with(sections: Vec<(&str, Vec<serde_yaml_neo::Mapping>)>) -> ProjectDoc {
    let mut doc = ProjectDoc {
        format: crate::project_yaml::FORMAT.to_string(),
        ..Default::default()
    };
    for (name, items) in sections {
        doc.sections.insert(name.to_string(), items);
    }
    doc
}

/// An entity a create needs to be new: checked inside the import's
/// transaction, so of two concurrent creates the second gets 409.
#[derive(Clone)]
struct MustBeNew {
    section: &'static str,
    key: String,
    message: String,
}

/// A write through the import engine.
struct Write<'a> {
    state: &'a AppState,
    auth: &'a AuthUser,
    origin: ActionOrigin,
    dry_run: bool,
    signature: Option<SignatureInput>,
    /// `equipment_type:<id>` or `equipment_instance:<name>`.
    resource: String,
    must_be_new: Option<MustBeNew>,
}

/// Every write-policy change of an expansion: `{instance, tag, field,
/// before, after}`, in a fixed order.
fn policy_changes(expanded: &[Expanded]) -> Vec<Value> {
    let mut out = Vec::new();
    for e in expanded {
        for t in &e.tags {
            for d in t.policy_diff() {
                out.push(json!({
                    "instance": e.instance,
                    "tag": t.reference,
                    "field": d.field,
                    "before": d.before,
                    "after": d.after,
                }));
            }
        }
    }
    out
}

/// What the audit entry of a write lists: per instance, every tag attribute
/// changed (before and after), registers added or moved, flows written or
/// deleted, widgets rebound.
fn audit_changes(expanded: &[Expanded]) -> Value {
    Value::Array(
        expanded
            .iter()
            .map(|e| {
                let tags: Vec<Value> = e
                    .tags
                    .iter()
                    .filter(|t| !t.diff.is_empty())
                    .map(|t| json!({"tag": t.reference, "action": t.action, "diff": t.diff}))
                    .collect();
                let mut v = json!({"instance": e.instance, "type": e.type_id, "tags": tags});
                for (k, list) in [
                    ("registers_added", &e.registers_added),
                    ("registers_moved", &e.registers_moved),
                    ("flows", &e.flows),
                    ("flows_removed", &e.flows_removed),
                ] {
                    if !list.is_empty() {
                        v[k] = json!(list);
                    }
                }
                if !e.widgets.is_empty() {
                    v["widgets"] = json!(e.widgets.len());
                }
                v
            })
            .collect(),
    )
}

fn import_error(e: anyhow::Error) -> Box<Response> {
    if let Some(invalid) = e.chain().find_map(|c| c.downcast_ref::<InvalidProject>()) {
        let errors: Vec<String> = invalid
            .0
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .map(|i| format!("{}: {}", i.at, i.message))
            .collect();
        return resp(AppError(
            RejectedJson(
                StatusCode::BAD_REQUEST,
                json!({"error": errors.join("; "), "issues": invalid.0}),
            )
            .into(),
        ));
    }
    if let Some(c) = e.chain().find_map(|c| c.downcast_ref::<Conflict>()) {
        return resp(conflict(c.0.clone()));
    }
    resp(AppError(e))
}

fn guard(
    must_be_new: Option<MustBeNew>,
    signed_policy: Option<Vec<Value>>,
) -> crate::project_yaml::Guard {
    Box::new(move |snap, expanded| {
        if let Some(m) = &must_be_new
            && snap
                .get(m.section)
                .is_some_and(|rows| rows.iter().any(|r| r.key == m.key))
        {
            return Err(Conflict(m.message.clone()).into());
        }
        if let Some(signed) = signed_policy
            && policy_changes(expanded) != signed
        {
            return Err(Conflict(
                "The write-policy changes are not the ones previewed and signed (something else changed meanwhile); preview the change and sign it again"
                    .to_string(),
            )
            .into());
        }
        Ok(())
    })
}

impl Write<'_> {
    /// Plan (`dry_run`) or apply a small merge import; afterwards restart the
    /// Modbus pollers and load the flows it touched. An invalid result is 400
    /// with every issue; a create of something that exists is 409; a change
    /// of write policies needs the signature (428 without it).
    async fn run(self, doc: &ProjectDoc, audit_action: &str) -> Res<ImportPlan> {
        let state = self.state;
        let plan = crate::project_yaml::plan_import_guarded(
            &state.storage,
            doc,
            Mode::Merge,
            guard(self.must_be_new.clone(), None),
        )
        .await
        .map_err(import_error)?;
        if self.dry_run {
            return Ok(plan);
        }
        let policy = policy_changes(&plan.expanded);
        let signed = if policy.is_empty() {
            None
        } else {
            let ctx = crate::edition::ActionContext {
                action: SignedAction::ModeSwitch,
                origin: self.origin,
                username: self.auth.username.clone(),
                role: self.auth.role.clone(),
                resource: format!("{}:write_policy", self.resource),
                details: json!({"mode": "write_policy", "changes": policy}),
            };
            super::signature::authorize(state, &ctx, self.signature.as_ref())
                .await
                .map_err(|e| Box::new(e.into_response()))?
        };
        // Rollback point, as before a project import.
        if let Err(e) = state
            .storage
            .create_config_snapshot(
                Some("pre-equipment"),
                Some(&format!(
                    "Automatic snapshot before an equipment change ({})",
                    self.resource
                )),
                "import",
                Some(&self.auth.username),
            )
            .await
        {
            tracing::warn!("snapshot before an equipment change: {e:#}");
        }
        let actor = super::design_history::actor_for_origin(self.auth, self.origin, "equipment")
            .with_note(self.resource.clone());
        let plan = crate::project_yaml::apply_import_guarded_as(
            &state.storage,
            doc,
            Mode::Merge,
            guard(self.must_be_new, (!policy.is_empty()).then_some(policy)),
            &actor,
        )
        .await
        .map_err(import_error)?;
        #[cfg(feature = "modbus")]
        super::project::sync_modbus_pollers(state, &plan).await;
        sync_flows(state, &plan).await;
        let mut details = json!({"changes": audit_changes(&plan.expanded)}).to_string();
        if let Some(signed) = &signed {
            details.push_str(&signed.audit_suffix());
        }
        audit(
            state,
            self.auth,
            audit_action,
            self.resource.clone(),
            Some(details),
        );
        Ok(plan)
    }
}

/// Load created or changed flows into the running engine.
async fn sync_flows(state: &AppState, plan: &ImportPlan) {
    let Some(engine) = &state.flow_engine else {
        return;
    };
    for change in plan.changes_in("flows") {
        if change.action == crate::project_yaml::Action::Delete {
            engine.remove_flow(&change.key).await;
            continue;
        }
        let Ok(Some(flow)) = state.storage.get_flow(&change.key).await else {
            continue;
        };
        match crate::flow_engine::engine::Flow::from_db(&flow) {
            Ok(parsed) => {
                if engine.get_flow(&parsed.id).await.is_some() {
                    engine.update_flow(parsed).await;
                } else {
                    engine.add_flow(parsed).await;
                }
            }
            Err(e) => tracing::warn!("flow '{}' from an equipment type: {e}", change.key),
        }
    }
}

fn mapping_err(e: anyhow::Error) -> AppError {
    AppError::bad_request(format!("{e:#}"))
}

/// `resolved` of an instance: its roles against what is stored now, and the
/// binding the Designer applies to a symbol placed for it.
pub async fn resolved(state: &AppState, inst: &EquipmentInstance) -> Result<Value, AppError> {
    let (types, _, world) = state.storage.equipment_state().await?;
    let Some(t) = types.into_iter().find(|t| t.id == inst.type_id) else {
        return Ok(Value::Null);
    };
    let res = resolve::resolve(&t, inst, &world);
    let widget = resolve::widget_patch(&t, inst, &res);
    let mut v = serde_json::to_value(&res).unwrap_or(Value::Null);
    v["widget"] = serde_json::to_value(widget).unwrap_or(Value::Null);
    v["problems"] = json!(res.problems);
    v["notes"] = json!(res.notes);
    Ok(v)
}

fn affected(e: &Expanded) -> Value {
    serde_json::to_value(e).unwrap_or(Value::Null)
}

// ── Types ──

/// GET /api/v1/equipment-types
#[utoipa::path(
    get,
    path = "/api/v1/equipment-types",
    responses((status = 200, description = "Types, each with `instance_count`", body = Vec<EquipmentType>)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_types(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let out: Vec<Value> = state
        .storage
        .list_equipment_types()
        .await?
        .into_iter()
        .map(|(t, n)| {
            let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
            v["instance_count"] = json!(n);
            v
        })
        .collect();
    Ok(Json(out))
}

/// GET /api/v1/equipment-types/{id}
#[utoipa::path(
    get,
    path = "/api/v1/equipment-types/{id}",
    params(("id" = String, Path, description = "Type id")),
    responses((status = 200, body = EquipmentType), (status = 404)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_type(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let t = state
        .storage
        .get_equipment_type(&id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("No equipment type '{id}'")))?;
    let n = state
        .storage
        .list_equipment_instances(Some(&id))
        .await?
        .len();
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    v["instance_count"] = json!(n);
    Ok(Json(v))
}

/// POST /api/v1/equipment-types
#[utoipa::path(
    post,
    path = "/api/v1/equipment-types",
    request_body = EquipmentType,
    responses(
        (status = 201, body = EquipmentType),
        (status = 400, description = "Unknown field or invalid value, named in the error"),
        (status = 409, description = "The id is taken"),
    ),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_type(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Json(body): Json<Value>,
) -> Response {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    create_type_as(&state, &auth, origin, body).await
}

/// [`create_type`] for a caller of the given origin (the MCP tools).
pub async fn create_type_as(
    state: &AppState,
    auth: &AuthUser,
    origin: ActionOrigin,
    body: Value,
) -> Response {
    let go = async {
        let mut body = body;
        let signature = take_signature(&mut body).map_err(resp)?;
        let t = parse_type(body).map_err(resp)?;
        let entity = expansion::type_entity(&t).map_err(|e| resp(mapping_err(e)))?;
        Write {
            state,
            auth,
            origin,
            dry_run: false,
            signature,
            resource: format!("equipment_type:{}", t.id),
            must_be_new: Some(MustBeNew {
                section: expansion::TYPES,
                key: t.id.clone(),
                message: format!("An equipment type '{}' already exists", t.id),
            }),
        }
        .run(
            &doc_with(vec![(expansion::TYPES, vec![entity])]),
            "create_equipment_type",
        )
        .await?;
        Ok::<_, Box<Response>>((StatusCode::CREATED, Json(t)).into_response())
    };
    go.await.unwrap_or_else(|r| *r)
}

/// PUT /api/v1/equipment-types/{id}
#[utoipa::path(
    put,
    path = "/api/v1/equipment-types/{id}",
    params(
        ("id" = String, Path, description = "Type id"),
        ("dry_run" = Option<bool>, Query, description = "Only list the instances (tags, registers, flows, widgets) the change would update"),
    ),
    request_body = EquipmentType,
    responses(
        (status = 200, description = "`{type, affected: [{instance, tags, registers_added, flows, widgets}], warnings}`", body = serde_json::Value),
        (status = 400),
        (status = 404),
    ),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_type(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Response {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    update_type_as(&state, &auth, origin, &id, q.dry_run, body).await
}

/// [`update_type`] for a caller of the given origin (the MCP tools).
pub async fn update_type_as(
    state: &AppState,
    auth: &AuthUser,
    origin: ActionOrigin,
    id: &str,
    dry_run: bool,
    body: Value,
) -> Response {
    let go = async {
        let mut body = body;
        let signature = take_signature(&mut body).map_err(resp)?;
        let t = parse_type(body).map_err(resp)?;
        if t.id != id {
            return Err(resp(AppError::bad_request(format!(
                "id '{}' in the body differs from '{id}' in the path",
                t.id
            ))));
        }
        if state
            .storage
            .get_equipment_type(id)
            .await
            .map_err(|e| resp(e.into()))?
            .is_none()
        {
            return Err(resp(AppError::not_found(format!(
                "No equipment type '{id}'"
            ))));
        }
        let entity = expansion::type_entity(&t).map_err(|e| resp(mapping_err(e)))?;
        let plan = Write {
            state,
            auth,
            origin,
            dry_run,
            signature,
            resource: format!("equipment_type:{id}"),
            must_be_new: None,
        }
        .run(
            &doc_with(vec![(expansion::TYPES, vec![entity])]),
            "update_equipment_type",
        )
        .await?;
        Ok(Json(json!({
            "type": t,
            "dry_run": dry_run,
            "affected": plan.expanded.iter().map(affected).collect::<Vec<_>>(),
            "warnings": plan.warnings,
        }))
        .into_response())
    };
    go.await.unwrap_or_else(|r| *r)
}

/// DELETE /api/v1/equipment-types/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/equipment-types/{id}",
    params(("id" = String, Path, description = "Type id")),
    responses((status = 200), (status = 404), (status = 409, description = "Instances of the type exist")),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_type(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // A rollback point, unless the delete is refused anyway.
    if state.storage.get_equipment_type(&id).await?.is_some()
        && state
            .storage
            .list_equipment_instances(Some(&id))
            .await?
            .is_empty()
    {
        snapshot(&state, &auth, &format!("equipment_type:{id}")).await;
    }
    let actor = super::design_history::actor(&auth, api_key.is_some(), None, "equipment")
        .with_note(format!("equipment_type:{id} deleted"));
    match state.storage.delete_equipment_type(&id, &actor).await? {
        TypeDelete::Deleted => {
            audit(
                &state,
                &auth,
                "delete_equipment_type",
                format!("equipment_type:{id}"),
                None,
            );
            Ok(Json(json!({"ok": true, "id": id})))
        }
        TypeDelete::NotFound => Err(AppError::not_found(format!("No equipment type '{id}'"))),
        TypeDelete::InUse(names) => Err(conflict(format!(
            "Equipment type '{id}' has {} instance(s) ({}); delete them first",
            names.len(),
            names.join(", ")
        ))),
    }
}

/// GET /api/v1/equipment-types/library
#[utoipa::path(
    get,
    path = "/api/v1/equipment-types/library",
    responses((status = 200, description = "Starter types with their faceplates", body = Vec<library::LibraryEntry>)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_library() -> impl IntoResponse {
    Json(library::entries())
}

/// Copy starter types into the database (with the faceplates they open,
/// when missing). Types that exist are skipped.
pub async fn install(
    state: &AppState,
    auth: &AuthUser,
    origin: ActionOrigin,
    ids: Option<Vec<String>>,
) -> Res<Value> {
    let entries = library::entries();
    let wanted: Vec<library::LibraryEntry> = match &ids {
        None => entries,
        Some(ids) => {
            let mut out = Vec::new();
            for id in ids {
                match entries.iter().find(|e| &e.equipment_type.id == id) {
                    Some(e) => out.push(e.clone()),
                    None => {
                        return Err(resp(AppError::bad_request(format!(
                            "No starter equipment type '{id}'"
                        ))));
                    }
                }
            }
            out
        }
    };
    let stored: Vec<String> = state
        .storage
        .list_equipment_types()
        .await
        .map_err(|e| resp(e.into()))?
        .into_iter()
        .map(|(t, _)| t.id)
        .collect();
    let faceplates: Vec<String> = state
        .storage
        .list_faceplates()
        .await
        .map_err(|e| resp(e.into()))?
        .into_iter()
        .map(|f| f.id)
        .collect();
    let (mut installed, mut skipped) = (Vec::new(), Vec::new());
    let (mut types, mut fps) = (Vec::new(), Vec::new());
    let mut fp_ids: Vec<String> = Vec::new();
    for e in wanted {
        let id = e.equipment_type.id.clone();
        if stored.contains(&id) || installed.contains(&id) {
            skipped.push(id);
            continue;
        }
        if let Some(fp) = e.faceplate
            && !faceplates.contains(&fp.id)
            && !fp_ids.contains(&fp.id)
        {
            fp_ids.push(fp.id.clone());
            let v = serde_json::to_value(&fp).map_err(|e| resp(AppError(e.into())))?;
            let Ok(serde_yaml_neo::Value::Mapping(m)) = crate::project_yaml::norm::json_to_yaml(&v)
            else {
                continue;
            };
            fps.push(m);
        }
        types.push(expansion::type_entity(&e.equipment_type).map_err(|e| resp(mapping_err(e)))?);
        installed.push(id);
    }
    if !types.is_empty() {
        Write {
            state,
            auth,
            origin,
            dry_run: false,
            signature: None,
            resource: format!("equipment_types:{}", installed.join(",")),
            must_be_new: None,
        }
        .run(
            &doc_with(vec![("faceplates", fps), (expansion::TYPES, types)]),
            "install_equipment_types",
        )
        .await?;
    }
    Ok(json!({"installed": installed, "skipped": skipped}))
}

/// POST /api/v1/equipment-types/library/install
#[utoipa::path(
    post,
    path = "/api/v1/equipment-types/library/install",
    request_body = InstallRequest,
    responses((status = 200, description = "`{installed: [ids], skipped: [ids already present]}`", body = serde_json::Value), (status = 400)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn install_library(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    body: Option<Json<InstallRequest>>,
) -> Response {
    let ids = body.and_then(|Json(b)| b.ids);
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    match install(&state, &auth, origin, ids).await {
        Ok(v) => Json(v).into_response(),
        Err(r) => *r,
    }
}

// ── Instances ──

async fn instance_body(state: &AppState, inst: &EquipmentInstance) -> Result<Value, AppError> {
    let mut v = serde_json::to_value(inst).unwrap_or(Value::Null);
    v["resolved"] = resolved(state, inst).await?;
    Ok(v)
}

/// GET /api/v1/equipment-instances?type=
#[utoipa::path(
    get,
    path = "/api/v1/equipment-instances",
    params(("type" = Option<String>, Query, description = "Only instances of this type")),
    responses((status = 200, description = "Instances, each with `resolved` (roles, tags, symbol binding)", body = Vec<EquipmentInstance>)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_instances(
    State(state): State<AppState>,
    Query(q): Query<InstanceQuery>,
) -> Result<impl IntoResponse, AppError> {
    let (types, instances, world) = state.storage.equipment_state().await?;
    let out: Vec<Value> = instances
        .into_iter()
        .filter(|i| q.type_id.as_deref().is_none_or(|t| t == i.type_id))
        .map(|inst| {
            let mut v = serde_json::to_value(&inst).unwrap_or(Value::Null);
            v["resolved"] = match types.iter().find(|t| t.id == inst.type_id) {
                Some(t) => {
                    let res = resolve::resolve(t, &inst, &world);
                    let widget = resolve::widget_patch(t, &inst, &res);
                    let mut r = serde_json::to_value(&res).unwrap_or(Value::Null);
                    r["widget"] = serde_json::to_value(widget).unwrap_or(Value::Null);
                    r["problems"] = json!(res.problems);
                    r["notes"] = json!(res.notes);
                    r
                }
                None => Value::Null,
            };
            v
        })
        .collect();
    Ok(Json(out))
}

/// GET /api/v1/equipment-instances/{name}
#[utoipa::path(
    get,
    path = "/api/v1/equipment-instances/{name}",
    params(("name" = String, Path, description = "Instance name, e.g. P-101A")),
    responses((status = 200, body = EquipmentInstance), (status = 404)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_instance(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let inst = state
        .storage
        .get_equipment_instance(&name)
        .await?
        .ok_or_else(|| AppError::not_found(format!("No equipment instance '{name}'")))?;
    Ok(Json(instance_body(&state, &inst).await?))
}

async fn save_instance(w: Write<'_>, inst: &EquipmentInstance, action: &str) -> Res<Value> {
    let state = w.state;
    let dry_run = w.dry_run;
    let entity = expansion::instance_entity(inst).map_err(|e| resp(mapping_err(e)))?;
    let plan = w
        .run(
            &doc_with(vec![(expansion::INSTANCES, vec![entity])]),
            action,
        )
        .await?;
    let mut v = instance_body(state, inst).await.map_err(resp)?;
    v["dry_run"] = json!(dry_run);
    v["changes"] = plan
        .expanded
        .iter()
        .find(|e| e.instance == inst.name)
        .map(affected)
        .unwrap_or(Value::Null);
    v["warnings"] = json!(plan.warnings);
    Ok(v)
}

/// POST /api/v1/equipment-instances (instantiate)
#[utoipa::path(
    post,
    path = "/api/v1/equipment-instances",
    params(("dry_run" = Option<bool>, Query, description = "Only answer what instantiating would create, change and bind")),
    request_body = EquipmentInstance,
    responses(
        (status = 201, description = "The instance with `resolved` and `changes` (tags with their attribute diffs, registers_added, flows, widgets)", body = serde_json::Value),
        (status = 200, description = "Dry run"),
        (status = 400, description = "Invalid, a required role has no tag, a tag, register, prefix or flow id another instance or user has; every problem named"),
        (status = 409, description = "The name is taken"),
        (status = 428, description = "It changes a tag's write policy: an electronic signature is needed"),
    ),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Response {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    create_instance_as(&state, &auth, origin, q.dry_run, body).await
}

/// [`create_instance`] for a caller of the given origin (the MCP tools).
pub async fn create_instance_as(
    state: &AppState,
    auth: &AuthUser,
    origin: ActionOrigin,
    dry_run: bool,
    body: Value,
) -> Response {
    let go = async {
        let mut body = body;
        let signature = take_signature(&mut body).map_err(resp)?;
        let inst = parse_instance(body).map_err(resp)?;
        if state
            .storage
            .get_equipment_type(&inst.type_id)
            .await
            .map_err(|e| resp(e.into()))?
            .is_none()
        {
            return Err(resp(AppError::bad_request(format!(
                "type: no equipment type '{}'",
                inst.type_id
            ))));
        }
        let w = Write {
            state,
            auth,
            origin,
            dry_run,
            signature,
            resource: format!("equipment_instance:{}", inst.name),
            must_be_new: Some(MustBeNew {
                section: expansion::INSTANCES,
                key: inst.name.clone(),
                message: format!("An equipment instance '{}' already exists", inst.name),
            }),
        };
        let v = save_instance(w, &inst, "create_equipment_instance").await?;
        let status = if dry_run {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        };
        Ok((status, Json(v)).into_response())
    };
    go.await.unwrap_or_else(|r| *r)
}

/// PUT /api/v1/equipment-instances/{name}
#[utoipa::path(
    put,
    path = "/api/v1/equipment-instances/{name}",
    params(
        ("name" = String, Path, description = "Instance name"),
        ("dry_run" = Option<bool>, Query, description = "Only answer what the change would do"),
    ),
    request_body = EquipmentInstance,
    responses((status = 200, body = serde_json::Value), (status = 400), (status = 404), (status = 428, description = "It changes a tag's write policy: an electronic signature is needed")),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(name): Path<String>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Response {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    update_instance_as(&state, &auth, origin, &name, q.dry_run, body).await
}

/// [`update_instance`] for a caller whose origin is known (MCP, revert).
pub async fn update_instance_as(
    state: &AppState,
    auth: &AuthUser,
    origin: ActionOrigin,
    name: &str,
    dry_run: bool,
    body: Value,
) -> Response {
    let go = async {
        let mut body = body;
        let signature = take_signature(&mut body).map_err(resp)?;
        let inst = parse_instance(body).map_err(resp)?;
        if inst.name != name {
            return Err(resp(AppError::bad_request(format!(
                "name '{}' in the body differs from '{name}' in the path",
                inst.name
            ))));
        }
        if state
            .storage
            .get_equipment_instance(name)
            .await
            .map_err(|e| resp(e.into()))?
            .is_none()
        {
            return Err(resp(AppError::not_found(format!(
                "No equipment instance '{name}'"
            ))));
        }
        let w = Write {
            state,
            auth,
            origin,
            dry_run,
            signature,
            resource: format!("equipment_instance:{name}"),
            must_be_new: None,
        };
        Ok(Json(save_instance(w, &inst, "update_equipment_instance").await?).into_response())
    };
    go.await.unwrap_or_else(|r| *r)
}

/// DELETE /api/v1/equipment-instances/{name}
#[utoipa::path(
    delete,
    path = "/api/v1/equipment-instances/{name}",
    params(
        ("name" = String, Path, description = "Instance name"),
        ("dry_run" = Option<bool>, Query, description = "Only list the flows it would delete and the widgets it would unbind"),
    ),
    responses(
        (status = 200, description = "`{ok, name, dry_run, flows_deleted, widgets_unbound}`: its generated flows are deleted and its widgets unbound (`config.equipment` cleared, `config.unboundEquipment` kept); its tags and registers stay"),
        (status = 404),
    ),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(name): Path<String>,
    Query(q): Query<DryRunQuery>,
) -> Result<impl IntoResponse, AppError> {
    let resource = format!("equipment_instance:{name}");
    if !q.dry_run {
        if state.storage.get_equipment_instance(&name).await?.is_none() {
            return Err(AppError::not_found(format!(
                "No equipment instance '{name}'"
            )));
        }
        snapshot(&state, &auth, &resource).await;
    }
    let Some(out) = state
        .storage
        .delete_equipment_instance(
            &name,
            q.dry_run,
            &super::design_history::actor(&auth, api_key.is_some(), None, "equipment")
                .with_note(format!("equipment_instance:{name} deleted")),
        )
        .await?
    else {
        return Err(AppError::not_found(format!(
            "No equipment instance '{name}'"
        )));
    };
    let widgets: Vec<Value> = out
        .widgets
        .iter()
        .map(|(s, w)| json!({"screen_id": s, "equipment_id": w}))
        .collect();
    if !q.dry_run {
        if let Some(engine) = &state.flow_engine {
            for id in &out.flows {
                engine.remove_flow(id).await;
            }
        }
        audit(
            &state,
            &auth,
            "delete_equipment_instance",
            resource,
            Some(json!({"flows_deleted": out.flows, "widgets_unbound": widgets}).to_string()),
        );
    }
    Ok(Json(json!({
        "ok": true,
        "name": name,
        "dry_run": q.dry_run,
        "flows_deleted": out.flows,
        "widgets_unbound": widgets,
    })))
}

/// A configuration snapshot before a write that does not go through
/// [`Write::run`] (deletes).
async fn snapshot(state: &AppState, auth: &AuthUser, resource: &str) {
    if let Err(e) = state
        .storage
        .create_config_snapshot(
            Some("pre-equipment"),
            Some(&format!(
                "Automatic snapshot before an equipment change ({resource})"
            )),
            "import",
            Some(&auth.username),
        )
        .await
    {
        tracing::warn!("snapshot before an equipment change: {e:#}");
    }
}
