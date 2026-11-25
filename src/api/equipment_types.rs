//! Equipment types and instances (`/api/v1/equipment-types`,
//! `/api/v1/equipment-instances`), docs/equipment-types.md.
//!
//! Every write is a small project import (merge) built from the request:
//! the expansion in `project_yaml/equipment.rs` turns an instance into its
//! tags, registers, flows and widget bindings, and a changed type into the
//! re-expansion of all its instances, in one transaction. `dry_run=true`
//! answers what would change and writes nothing.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppError, AppState, AuthUser, Rejected, RejectedJson};
use crate::equipment_types::library;
use crate::equipment_types::resolve;
use crate::equipment_types::{EquipmentInstance, EquipmentType};
use crate::project_yaml::equipment::{self as expansion, Expanded};
use crate::project_yaml::{ImportPlan, InvalidProject, Mode, ProjectDoc, Severity};
use crate::storage::equipment_types::TypeDelete;

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

/// Plan (`dry_run`) or apply a small merge import; afterwards restart the
/// Modbus pollers and load the flows it touched. An invalid result is 400
/// with every issue.
async fn run(state: &AppState, doc: &ProjectDoc, dry_run: bool) -> Result<ImportPlan, AppError> {
    let result = if dry_run {
        crate::project_yaml::plan_import(&state.storage, doc, Mode::Merge).await
    } else {
        crate::project_yaml::apply_import(&state.storage, doc, Mode::Merge).await
    };
    let plan = match result {
        Ok(p) => p,
        Err(e) => {
            if let Some(invalid) = e.chain().find_map(|c| c.downcast_ref::<InvalidProject>()) {
                let errors: Vec<String> = invalid
                    .0
                    .iter()
                    .filter(|i| i.severity == Severity::Error)
                    .map(|i| format!("{}: {}", i.at, i.message))
                    .collect();
                return Err(AppError(
                    RejectedJson(
                        StatusCode::BAD_REQUEST,
                        json!({"error": errors.join("; "), "issues": invalid.0}),
                    )
                    .into(),
                ));
            }
            return Err(AppError(e));
        }
    };
    if !dry_run {
        #[cfg(feature = "modbus")]
        super::project::sync_modbus_pollers(state, &plan).await;
        sync_flows(state, &plan).await;
    }
    Ok(plan)
}

/// Load created or changed flows into the running engine.
async fn sync_flows(state: &AppState, plan: &ImportPlan) {
    let Some(engine) = &state.flow_engine else {
        return;
    };
    for change in plan.changes_in("flows") {
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
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let t = parse_type(body)?;
    if state.storage.get_equipment_type(&t.id).await?.is_some() {
        return Err(conflict(format!(
            "An equipment type '{}' already exists",
            t.id
        )));
    }
    let entity = expansion::type_entity(&t).map_err(mapping_err)?;
    run(
        &state,
        &doc_with(vec![(expansion::TYPES, vec![entity])]),
        false,
    )
    .await?;
    audit(
        &state,
        &auth,
        "create_equipment_type",
        format!("equipment_type:{}", t.id),
        None,
    );
    Ok((StatusCode::CREATED, Json(t)))
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
    Path(id): Path<String>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let t = parse_type(body)?;
    if t.id != id {
        return Err(AppError::bad_request(format!(
            "id '{}' in the body differs from '{id}' in the path",
            t.id
        )));
    }
    if state.storage.get_equipment_type(&id).await?.is_none() {
        return Err(AppError::not_found(format!("No equipment type '{id}'")));
    }
    let entity = expansion::type_entity(&t).map_err(mapping_err)?;
    let plan = run(
        &state,
        &doc_with(vec![(expansion::TYPES, vec![entity])]),
        q.dry_run,
    )
    .await?;
    if !q.dry_run {
        audit(
            &state,
            &auth,
            "update_equipment_type",
            format!("equipment_type:{id}"),
            Some(format!("instances={}", plan.expanded.len())),
        );
    }
    Ok(Json(json!({
        "type": t,
        "dry_run": q.dry_run,
        "affected": plan.expanded.iter().map(affected).collect::<Vec<_>>(),
        "warnings": plan.warnings,
    })))
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
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    match state.storage.delete_equipment_type(&id).await? {
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
    ids: Option<Vec<String>>,
) -> Result<Value, AppError> {
    let entries = library::entries();
    let wanted: Vec<library::LibraryEntry> = match &ids {
        None => entries,
        Some(ids) => {
            let mut out = Vec::new();
            for id in ids {
                match entries.iter().find(|e| &e.equipment_type.id == id) {
                    Some(e) => out.push(e.clone()),
                    None => {
                        return Err(AppError::bad_request(format!(
                            "No starter equipment type '{id}'"
                        )));
                    }
                }
            }
            out
        }
    };
    let stored: Vec<String> = state
        .storage
        .list_equipment_types()
        .await?
        .into_iter()
        .map(|(t, _)| t.id)
        .collect();
    let faceplates: Vec<String> = state
        .storage
        .list_faceplates()
        .await?
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
            let v = serde_json::to_value(&fp).map_err(|e| AppError(e.into()))?;
            let Ok(serde_yaml_neo::Value::Mapping(m)) = crate::project_yaml::norm::json_to_yaml(&v)
            else {
                continue;
            };
            fps.push(m);
        }
        types.push(expansion::type_entity(&e.equipment_type).map_err(mapping_err)?);
        installed.push(id);
    }
    if !types.is_empty() {
        run(
            state,
            &doc_with(vec![("faceplates", fps), (expansion::TYPES, types)]),
            false,
        )
        .await?;
        audit(
            state,
            auth,
            "install_equipment_types",
            "equipment_types".into(),
            Some(installed.join(",")),
        );
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
    body: Option<Json<InstallRequest>>,
) -> Result<impl IntoResponse, AppError> {
    let ids = body.and_then(|Json(b)| b.ids);
    Ok(Json(install(&state, &auth, ids).await?))
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

async fn save_instance(
    state: &AppState,
    auth: &AuthUser,
    inst: &EquipmentInstance,
    dry_run: bool,
    action: &str,
) -> Result<Value, AppError> {
    let entity = expansion::instance_entity(inst).map_err(mapping_err)?;
    let plan = run(
        state,
        &doc_with(vec![(expansion::INSTANCES, vec![entity])]),
        dry_run,
    )
    .await?;
    if !dry_run {
        audit(
            state,
            auth,
            action,
            format!("equipment_instance:{}", inst.name),
            Some(format!("type={} device={}", inst.type_id, inst.device)),
        );
    }
    let mut v = instance_body(state, inst).await?;
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
    params(("dry_run" = Option<bool>, Query, description = "Only answer what instantiating would create and bind")),
    request_body = EquipmentInstance,
    responses(
        (status = 201, description = "The instance with `resolved` and `changes` (tags, registers_added, flows, widgets)", body = serde_json::Value),
        (status = 200, description = "Dry run"),
        (status = 400, description = "Invalid, or a required role has no tag; every problem named"),
        (status = 409, description = "The name is taken"),
    ),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Result<axum::response::Response, AppError> {
    let inst = parse_instance(body)?;
    if state
        .storage
        .get_equipment_instance(&inst.name)
        .await?
        .is_some()
    {
        return Err(conflict(format!(
            "An equipment instance '{}' already exists",
            inst.name
        )));
    }
    if state
        .storage
        .get_equipment_type(&inst.type_id)
        .await?
        .is_none()
    {
        return Err(AppError::bad_request(format!(
            "type: no equipment type '{}'",
            inst.type_id
        )));
    }
    let v = save_instance(&state, &auth, &inst, q.dry_run, "create_equipment_instance").await?;
    let status = if q.dry_run {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(v)).into_response())
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
    responses((status = 200, body = serde_json::Value), (status = 400), (status = 404)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(name): Path<String>,
    Query(q): Query<DryRunQuery>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let inst = parse_instance(body)?;
    if inst.name != name {
        return Err(AppError::bad_request(format!(
            "name '{}' in the body differs from '{name}' in the path",
            inst.name
        )));
    }
    if state.storage.get_equipment_instance(&name).await?.is_none() {
        return Err(AppError::not_found(format!(
            "No equipment instance '{name}'"
        )));
    }
    Ok(Json(
        save_instance(&state, &auth, &inst, q.dry_run, "update_equipment_instance").await?,
    ))
}

/// DELETE /api/v1/equipment-instances/{name}
#[utoipa::path(
    delete,
    path = "/api/v1/equipment-instances/{name}",
    params(("name" = String, Path, description = "Instance name")),
    responses((status = 200, description = "Deleted; its tags, registers, flows and widget bindings are kept"), (status = 404)),
    tag = "Equipment",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_instance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    if !state.storage.delete_equipment_instance(&name).await? {
        return Err(AppError::not_found(format!(
            "No equipment instance '{name}'"
        )));
    }
    audit(
        &state,
        &auth,
        "delete_equipment_instance",
        format!("equipment_instance:{name}"),
        None,
    );
    Ok(Json(json!({"ok": true, "name": name})))
}
