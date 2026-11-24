//! Linked device templates: Modbus devices that follow a stored template
//! instead of keeping a copy of its register map (docs/device-templates.md,
//! "Linked devices").
//!
//! * `PUT /api/v1/device-templates/{id}` saves a changed template (same id,
//!   version and source) and rewrites the registers of every linked device;
//!   with `dry_run=true` it only answers which devices would change and how.
//! * `GET /api/v1/device-templates/{id}/linked-devices` lists the linked
//!   devices and whether each still matches the template.
//! * `PUT /api/v1/modbus-devices/{id}/template` links a device (its registers
//!   become the template's), `DELETE` unlinks it (it keeps its registers).

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{AppError, AppState, AuthUser, Rejected};
use crate::device_templates::link::{self, RegisterDiff, TemplateLink};
use crate::device_templates::{DeviceTemplate, Protocol, TemplateRegister, builtin, resolve};
use crate::models::ModbusRegisterInput;

fn conflict(message: impl Into<String>) -> AppError {
    AppError(anyhow::Error::new(Rejected(
        StatusCode::CONFLICT,
        message.into(),
    )))
}

fn link_of(t: &DeviceTemplate) -> TemplateLink {
    TemplateLink {
        template_id: t.template_id.clone(),
        version: t.version.clone(),
        source: t.source.clone(),
    }
}

async fn find_template(
    state: &AppState,
    id: &str,
    version: Option<&str>,
    source: Option<&str>,
) -> Result<DeviceTemplate, AppError> {
    let mut all: Vec<DeviceTemplate> = builtin::builtins().to_vec();
    all.extend(state.storage.list_device_templates().await?);
    resolve(&all, id, version, source)
        .cloned()
        .ok_or_else(|| AppError::not_found(format!("No device template '{id}'")))
}

fn register_inputs(regs: &[TemplateRegister]) -> Vec<ModbusRegisterInput> {
    regs.iter()
        .map(|r| ModbusRegisterInput {
            name: r.name.clone(),
            address: None,
            offset: Some(r.offset),
            zero_based: true,
            register_type: r.register_type.clone(),
            scale: r.scale,
            register_area: r.register_area.clone(),
        })
        .collect()
}

/// Write the template's registers to a device and restart its poller.
async fn apply_registers(
    state: &AppState,
    device_id: &str,
    t: &DeviceTemplate,
) -> Result<(), AppError> {
    state
        .storage
        .replace_modbus_registers(device_id, &register_inputs(&t.registers))
        .await?;
    let configs = state.storage.get_all_device_configs().await?;
    if let Some(dc) = configs.iter().find(|c| c.id == device_id) {
        state.poller_manager.restart(dc.clone()).await;
    }
    Ok(())
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct LinkedDevice {
    pub id: String,
    pub name: String,
    /// What saving the template changes (or changed) on this device.
    pub changes: RegisterDiff,
}

async fn linked_devices(
    state: &AppState,
    t: &DeviceTemplate,
) -> Result<Vec<LinkedDevice>, AppError> {
    let mut out = Vec::new();
    for (id, name) in state.storage.modbus_devices_linked_to(&link_of(t)).await? {
        let current = state.storage.modbus_register_rows(&id).await?;
        out.push(LinkedDevice {
            id,
            name,
            changes: link::diff(&current, &t.registers),
        });
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
pub struct UpdateQuery {
    pub version: Option<String>,
    pub source: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

/// PUT /api/v1/device-templates/{id}
#[utoipa::path(
    put,
    path = "/api/v1/device-templates/{id}",
    params(
        ("id" = String, Path, description = "Template id"),
        ("version" = Option<String>, Query, description = "Version; the newest when omitted"),
        ("source" = Option<String>, Query, description = "Source, when the same id comes from several"),
        ("dry_run" = Option<bool>, Query, description = "Only list the linked devices and what would change"),
    ),
    request_body = DeviceTemplate,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400, description = "Not a valid template, or the body changes id, version or protocol"),
        (status = 403, description = "Built-in templates cannot be changed"),
        (status = 404),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_template(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<UpdateQuery>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let old = find_template(&state, &id, q.version.as_deref(), q.source.as_deref()).await?;
    if old.is_builtin() {
        return Err(AppError::forbidden(format!(
            "'{id}' is a built-in template and cannot be changed; save a copy with another id"
        )));
    }
    let mut t: DeviceTemplate = serde_json::from_value(body)
        .map_err(|e| AppError::bad_request(format!("Not a device template: {e}")))?;
    t.normalize_and_validate()
        .map_err(|errs| AppError::bad_request(format!("Invalid template: {}", errs.join("; "))))?;
    if t.template_id != old.template_id || t.version != old.version || t.protocol != old.protocol {
        return Err(AppError::bad_request(
            "template_id, version and protocol cannot change; import a new version instead",
        ));
    }
    t.source = old.source.clone();
    let mut devices = linked_devices(&state, &t).await?;
    if !q.dry_run {
        state.storage.update_device_template(&t).await?;
        for d in &mut devices {
            if !d.changes.is_empty() {
                apply_registers(&state, &d.id, &t).await?;
            }
        }
        state.audit_logger.log(crate::security::AuditEvent {
            action: "update_device_template".to_string(),
            username: auth.username.clone(),
            resource: format!("device_template:{}", t.template_id),
            details: Some(format!(
                "version={} source={} linked_devices={}",
                t.version,
                t.source,
                devices.len()
            )),
        });
    }
    Ok(Json(json!({
        "ok": true,
        "dry_run": q.dry_run,
        "template_id": t.template_id,
        "version": t.version,
        "source": t.source,
        "linked_devices": devices,
    })))
}

#[derive(Debug, Deserialize)]
pub struct RefQuery {
    pub version: Option<String>,
    pub source: Option<String>,
}

/// GET /api/v1/device-templates/{id}/linked-devices
#[utoipa::path(
    get,
    path = "/api/v1/device-templates/{id}/linked-devices",
    params(
        ("id" = String, Path, description = "Template id"),
        ("version" = Option<String>, Query, description = "Version; the newest when omitted"),
        ("source" = Option<String>, Query, description = "Source"),
    ),
    responses((status = 200, body = Vec<LinkedDevice>), (status = 404)),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_linked_devices(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RefQuery>,
) -> Result<impl IntoResponse, AppError> {
    let t = find_template(&state, &id, q.version.as_deref(), q.source.as_deref()).await?;
    Ok(Json(linked_devices(&state, &t).await?))
}

/// PUT /api/v1/modbus-devices/{id}/template
#[utoipa::path(
    put,
    path = "/api/v1/modbus-devices/{id}/template",
    params(("id" = String, Path, description = "Device id")),
    request_body = TemplateLink,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400, description = "Not a Modbus template"),
        (status = 404),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn link_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(device_id): Path<String>,
    Json(req): Json<TemplateLink>,
) -> Result<impl IntoResponse, AppError> {
    if state.storage.get_modbus_device(&device_id).await?.is_none() {
        return Err(AppError::not_found(format!(
            "No Modbus device '{device_id}'"
        )));
    }
    let t = find_template(
        &state,
        &req.template_id,
        Some(&req.version),
        Some(&req.source),
    )
    .await?;
    if t.protocol != Protocol::Modbus {
        return Err(AppError::bad_request("only Modbus templates can be linked"));
    }
    let link = link_of(&t);
    let current = state.storage.modbus_register_rows(&device_id).await?;
    let changes = link::diff(&current, &t.registers);
    state
        .storage
        .set_modbus_template_link(&device_id, Some(&link))
        .await?;
    if !changes.is_empty() {
        apply_registers(&state, &device_id, &t).await?;
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "link_device_template".to_string(),
        username: auth.username.clone(),
        resource: format!("modbus_device:{device_id}"),
        details: Some(format!("template={link}")),
    });
    Ok(Json(
        json!({"ok": true, "id": device_id, "template": link, "changes": changes}),
    ))
}

/// DELETE /api/v1/modbus-devices/{id}/template
#[utoipa::path(
    delete,
    path = "/api/v1/modbus-devices/{id}/template",
    params(("id" = String, Path, description = "Device id")),
    responses((status = 200, body = serde_json::Value), (status = 404)),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn unlink_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(device_id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let Some(link) = state.storage.modbus_template_link(&device_id).await? else {
        return Err(AppError::not_found(format!(
            "Modbus device '{device_id}' is not linked to a template"
        )));
    };
    state
        .storage
        .set_modbus_template_link(&device_id, None)
        .await?;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "unlink_device_template".to_string(),
        username: auth.username.clone(),
        resource: format!("modbus_device:{device_id}"),
        details: Some(format!("template={link}")),
    });
    Ok(Json(json!({"ok": true, "id": device_id, "unlinked": link})))
}

/// Refuse a register edit on a linked device: it follows its template.
pub async fn refuse_register_edit(state: &AppState, device_id: &str) -> Result<(), AppError> {
    if let Some(link) = state.storage.modbus_template_link(device_id).await? {
        return Err(conflict(format!(
            "Modbus device '{device_id}' follows template {link}; change the template, or unlink the device (DELETE /api/v1/modbus-devices/{device_id}/template) to edit its registers"
        )));
    }
    Ok(())
}

/// Refuse deleting a template that devices follow.
pub async fn refuse_delete_linked(state: &AppState, t: &DeviceTemplate) -> Result<(), AppError> {
    let linked = state.storage.modbus_devices_linked_to(&link_of(t)).await?;
    if !linked.is_empty() {
        let ids: Vec<&str> = linked.iter().map(|(id, _)| id.as_str()).collect();
        return Err(conflict(format!(
            "Template {} is linked to {} device(s) ({}); unlink them first",
            link_of(t),
            ids.len(),
            ids.join(", ")
        )));
    }
    Ok(())
}

/// Link a device that was just created from a template.
pub async fn link_new_device(
    state: &AppState,
    device_id: &str,
    t: &DeviceTemplate,
) -> Result<(), AppError> {
    state
        .storage
        .set_modbus_template_link(device_id, Some(&link_of(t)))
        .await?;
    Ok(())
}
