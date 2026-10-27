//! Device template library routes (`/api/v1/device-templates`).
//!
//! Reads are open to every signed-in user. Writes sit with the device routes
//! (admin, demo and integration roles); BACnet devices stay admin-only as on
//! the BACnet routes. Importing from the catalog fetches from the network and
//! is admin-only.

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::{AppError, AppState, AuthUser, Rejected};
use crate::device_templates::catalog::{
    self, CatalogError, DEFAULT_CATALOG_URL, SETTING_CATALOG_URL,
};
use crate::device_templates::{
    DEFAULT_LICENSE, DeviceTemplate, Protocol, SOURCE_CATALOG_PREFIX, SOURCE_LOCAL,
    TEMPLATE_FORMAT, TemplateFilter, TemplateObject, TemplateRegister, TemplateSummary, builtin,
    resolve, slugify,
};

fn err(status: StatusCode, message: impl Into<String>) -> AppError {
    AppError(anyhow::Error::new(Rejected(status, message.into())))
}

fn catalog_err(e: CatalogError) -> AppError {
    match e {
        CatalogError::Fetch(m) => err(StatusCode::BAD_GATEWAY, m),
        CatalogError::Invalid(m) => err(StatusCode::UNPROCESSABLE_ENTITY, m),
    }
}

/// Built-ins followed by the stored templates.
async fn all_templates(state: &AppState) -> Result<Vec<DeviceTemplate>, AppError> {
    let mut all: Vec<DeviceTemplate> = builtin::builtins().to_vec();
    all.extend(state.storage.list_device_templates().await?);
    Ok(all)
}

fn parse_template(body: Value) -> Result<DeviceTemplate, AppError> {
    let mut t: DeviceTemplate = serde_json::from_value(body)
        .map_err(|e| AppError::bad_request(format!("Not a device template: {e}")))?;
    t.normalize_and_validate()
        .map_err(|errs| AppError::bad_request(format!("Invalid template: {}", errs.join("; "))))?;
    Ok(t)
}

fn with_read_only(t: &DeviceTemplate) -> Value {
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("read_only".into(), Value::Bool(t.is_builtin()));
    }
    v
}

#[derive(Debug, Deserialize)]
pub struct RefQuery {
    pub version: Option<String>,
    pub source: Option<String>,
}

async fn find(state: &AppState, id: &str, q: &RefQuery) -> Result<DeviceTemplate, AppError> {
    let all = all_templates(state).await?;
    resolve(&all, id, q.version.as_deref(), q.source.as_deref())
        .cloned()
        .ok_or_else(|| AppError::not_found(format!("No device template '{id}'")))
}

/// GET /api/v1/device-templates
#[utoipa::path(
    get,
    path = "/api/v1/device-templates",
    params(
        ("q" = Option<String>, Query, description = "Substring of id, manufacturer, model or description"),
        ("manufacturer" = Option<String>, Query, description = "Manufacturer, case-insensitive"),
        ("protocol" = Option<String>, Query, description = "modbus or bacnet"),
        ("source" = Option<String>, Query, description = "builtin, local, catalog or a full source"),
    ),
    responses((status = 200, description = "Matching templates, every version", body = serde_json::Value)),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_templates(
    State(state): State<AppState>,
    Query(filter): Query<TemplateFilter>,
) -> Result<impl IntoResponse, AppError> {
    let mut rows: Vec<TemplateSummary> = all_templates(&state)
        .await?
        .iter()
        .filter(|t| filter.matches(t))
        .map(TemplateSummary::from)
        .collect();
    rows.sort_by(|a, b| {
        a.manufacturer
            .to_lowercase()
            .cmp(&b.manufacturer.to_lowercase())
            .then_with(|| a.model.to_lowercase().cmp(&b.model.to_lowercase()))
            .then_with(|| a.template_id.cmp(&b.template_id))
            .then_with(|| crate::device_templates::compare_versions(&b.version, &a.version))
    });
    let count = rows.len();
    Ok(Json(json!({ "templates": rows, "count": count })))
}

/// GET /api/v1/device-templates/{id}
#[utoipa::path(
    get,
    path = "/api/v1/device-templates/{id}",
    params(
        ("id" = String, Path, description = "Template id"),
        ("version" = Option<String>, Query, description = "Version; the newest when omitted"),
        ("source" = Option<String>, Query, description = "Source, when the same id comes from several"),
    ),
    responses(
        (status = 200, description = "The template and `read_only`", body = serde_json::Value),
        (status = 404),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_template(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RefQuery>,
) -> Result<impl IntoResponse, AppError> {
    let t = find(&state, &id, &q).await?;
    Ok(Json(with_read_only(&t)))
}

/// GET /api/v1/device-templates/{id}/export
#[utoipa::path(
    get,
    path = "/api/v1/device-templates/{id}/export",
    params(
        ("id" = String, Path, description = "Template id"),
        ("version" = Option<String>, Query, description = "Version; the newest when omitted"),
        ("source" = Option<String>, Query, description = "Source, when the same id comes from several"),
    ),
    responses(
        (status = 200, description = "The template file as a JSON download", body = serde_json::Value),
        (status = 404),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn export_template(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RefQuery>,
) -> Result<Response, AppError> {
    let t = find(&state, &id, &q).await?;
    let mut body = serde_json::to_string_pretty(&t).map_err(anyhow::Error::from)?;
    body.push('\n');
    let filename = format!("{}-{}.json", t.template_id, t.version);
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        body,
    )
        .into_response())
}

/// POST /api/v1/device-templates
///
/// Import a template file. The `source` in the file is ignored; the template is stored as `local`.
#[utoipa::path(
    post,
    path = "/api/v1/device-templates",
    request_body = DeviceTemplate,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400, description = "Not a valid template"),
        (status = 409, description = "That template id and version is already stored as local"),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_template(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<Value>,
) -> Result<impl IntoResponse, AppError> {
    let mut t = parse_template(body)?;
    t.source = SOURCE_LOCAL.to_string();
    store_new(&state, &auth, &t, "create_device_template").await?;
    Ok((StatusCode::CREATED, Json(created_body(&t))))
}

fn created_body(t: &DeviceTemplate) -> Value {
    json!({
        "ok": true,
        "template_id": t.template_id,
        "version": t.version,
        "source": t.source,
    })
}

async fn store_new(
    state: &AppState,
    auth: &AuthUser,
    t: &DeviceTemplate,
    action: &str,
) -> Result<(), AppError> {
    if !state.storage.insert_device_template(t).await? {
        return Err(err(
            StatusCode::CONFLICT,
            format!(
                "Template '{}' version {} from {} already exists; raise the version or delete the old one",
                t.template_id, t.version, t.source
            ),
        ));
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: action.to_string(),
        username: auth.username.clone(),
        resource: format!("device_template:{}", t.template_id),
        details: Some(format!("version={} source={}", t.version, t.source)),
    });
    Ok(())
}

/// DELETE /api/v1/device-templates/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/device-templates/{id}",
    params(
        ("id" = String, Path, description = "Template id"),
        ("version" = Option<String>, Query, description = "Version; the newest when omitted"),
        ("source" = Option<String>, Query, description = "Source, when the same id comes from several"),
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403, description = "Built-in templates cannot be deleted"),
        (status = 404),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_template(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<RefQuery>,
) -> Result<impl IntoResponse, AppError> {
    let t = find(&state, &id, &q).await?;
    if t.is_builtin() {
        return Err(AppError::forbidden(format!(
            "'{id}' is a built-in template and cannot be deleted"
        )));
    }
    if !state
        .storage
        .delete_device_template(&t.template_id, &t.source, &t.version)
        .await?
    {
        return Err(AppError::not_found(format!("No device template '{id}'")));
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_device_template".to_string(),
        username: auth.username.clone(),
        resource: format!("device_template:{}", t.template_id),
        details: Some(format!("version={} source={}", t.version, t.source)),
    });
    Ok(Json(
        json!({"ok": true, "template_id": t.template_id, "version": t.version, "source": t.source}),
    ))
}

// ── Save a device as a template ──

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct FromDeviceRequest {
    pub manufacturer: String,
    pub model: String,
    /// Default: the manufacturer and model as a slug.
    pub template_id: Option<String>,
    /// Default 1.0.0.
    pub version: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub verified_notes: String,
    #[serde(default)]
    pub quirks: Vec<String>,
    pub license: Option<String>,
    /// `modbus` or `bacnet`; when omitted a Modbus device with that id is tried first, then a BACnet one.
    pub protocol: Option<String>,
}

#[cfg(feature = "modbus")]
async fn template_from_modbus(
    state: &AppState,
    device_id: &str,
) -> Result<Option<DeviceTemplate>, AppError> {
    let Some(device) = state.storage.get_modbus_device(device_id).await? else {
        return Ok(None);
    };
    let registers = state.storage.list_modbus_registers(device_id).await?;
    let config = device.to_device_config(&registers);
    let mut t = blank_template(Protocol::Modbus);
    t.port = Some(device.port);
    t.unit_id = Some(device.unit_id);
    t.interval_ms = Some(device.interval_ms);
    t.endianness = Some(device.endianness.clone());
    t.registers = registers
        .iter()
        .zip(&config.registers)
        .map(|(r, c)| TemplateRegister {
            name: r.name.clone(),
            offset: c.protocol_address(),
            register_type: r.register_type.clone(),
            register_area: r.register_area.clone(),
            scale: r.scale,
            unit: String::new(),
            description: String::new(),
            writable: false,
        })
        .collect();
    Ok(Some(t))
}

#[cfg(not(feature = "modbus"))]
async fn template_from_modbus(
    _state: &AppState,
    _device_id: &str,
) -> Result<Option<DeviceTemplate>, AppError> {
    Ok(None)
}

#[cfg(feature = "bacnet")]
async fn template_from_bacnet(
    state: &AppState,
    auth: &AuthUser,
    device_id: &str,
) -> Result<Option<DeviceTemplate>, AppError> {
    if auth.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }
    let Ok(id) = device_id.parse::<i64>() else {
        return Ok(None);
    };
    let Some(device) = state.storage.get_bacnet_device(id).await? else {
        return Ok(None);
    };
    let objects = state.storage.list_bacnet_objects(id).await?;
    let mut t = blank_template(Protocol::Bacnet);
    t.port = Some(device.port);
    t.interval_ms = Some(device.interval_ms);
    t.cov_enabled = Some(device.cov_enabled);
    t.cov_lifetime = Some(device.cov_lifetime);
    t.objects = objects
        .into_iter()
        .map(|o| TemplateObject {
            object_type: o.object_type,
            object_instance: o.object_instance,
            property: o.property,
            name: o.name,
            display_name: o.display_name,
            cov_increment: o.cov_increment,
            unit: String::new(),
            description: String::new(),
            writable: false,
        })
        .collect();
    Ok(Some(t))
}

#[cfg(not(feature = "bacnet"))]
async fn template_from_bacnet(
    _state: &AppState,
    _auth: &AuthUser,
    _device_id: &str,
) -> Result<Option<DeviceTemplate>, AppError> {
    Ok(None)
}

fn blank_template(protocol: Protocol) -> DeviceTemplate {
    DeviceTemplate {
        format: TEMPLATE_FORMAT.to_string(),
        template_id: String::new(),
        manufacturer: String::new(),
        model: String::new(),
        protocol,
        description: String::new(),
        version: String::new(),
        verified: false,
        verified_notes: String::new(),
        quirks: Vec::new(),
        source: SOURCE_LOCAL.to_string(),
        license: DEFAULT_LICENSE.to_string(),
        port: None,
        interval_ms: None,
        unit_id: None,
        endianness: None,
        registers: Vec::new(),
        cov_enabled: None,
        cov_lifetime: None,
        objects: Vec::new(),
    }
}

/// POST /api/v1/device-templates/from-device/{device_id}
///
/// Save an existing Modbus or BACnet device, with its registers or objects, as a local template.
#[utoipa::path(
    post,
    path = "/api/v1/device-templates/from-device/{device_id}",
    params(("device_id" = String, Path, description = "Modbus device id or BACnet device id")),
    request_body = FromDeviceRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 404, description = "No such device"),
        (status = 409, description = "That template id and version is already stored as local"),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn template_from_device(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(device_id): Path<String>,
    Json(req): Json<FromDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let protocol = match req.protocol.as_deref() {
        None => None,
        Some(p) => Some(
            Protocol::parse(p)
                .ok_or_else(|| AppError::bad_request("protocol must be modbus or bacnet"))?,
        ),
    };
    let mut found = None;
    if protocol != Some(Protocol::Bacnet) {
        found = template_from_modbus(&state, &device_id).await?;
    }
    if found.is_none() && protocol != Some(Protocol::Modbus) {
        found = template_from_bacnet(&state, &auth, &device_id).await?;
    }
    let mut t =
        found.ok_or_else(|| AppError::not_found(format!("No device '{device_id}' to save")))?;

    t.manufacturer = req.manufacturer;
    t.model = req.model;
    t.template_id = match req.template_id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => slugify(&format!("{} {}", t.manufacturer, t.model)),
    };
    t.version = req.version.unwrap_or_else(|| "1.0.0".to_string());
    t.description = req.description;
    t.verified = req.verified;
    t.verified_notes = req.verified_notes;
    t.quirks = req.quirks;
    if let Some(l) = req.license {
        t.license = l;
    }
    t.normalize_and_validate()
        .map_err(|errs| AppError::bad_request(format!("Invalid template: {}", errs.join("; "))))?;
    store_new(&state, &auth, &t, "create_device_template").await?;
    Ok((StatusCode::CREATED, Json(created_body(&t))))
}

// ── Create a device from a template ──

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct InstantiateRequest {
    pub name: String,
    pub ip: String,
    /// Modbus device id; default: the name as a slug. BACnet devices get a numeric id.
    pub id: Option<String>,
    /// Default: the template's.
    pub port: Option<u16>,
    /// Modbus unit id; default: the template's.
    pub unit_id: Option<u8>,
    /// BACnet device instance; default 0.
    pub device_instance: Option<u32>,
    pub interval_ms: Option<u64>,
    /// Modbus word order; default: the template's.
    pub endianness: Option<String>,
    pub cov_enabled: Option<bool>,
    pub cov_lifetime: Option<u32>,
    /// BACnet only; default true.
    pub enabled: Option<bool>,
    /// Which version or source of the template, when the id alone is ambiguous.
    pub version: Option<String>,
    pub source: Option<String>,
}

#[cfg(feature = "modbus")]
async fn instantiate_modbus(
    state: &AppState,
    auth: &AuthUser,
    t: &DeviceTemplate,
    req: &InstantiateRequest,
    name: &str,
    ip: &str,
) -> Result<Value, AppError> {
    if req.device_instance.is_some()
        || req.cov_enabled.is_some()
        || req.cov_lifetime.is_some()
        || req.enabled.is_some()
    {
        return Err(AppError::bad_request(
            "device_instance, cov_enabled, cov_lifetime and enabled apply to BACnet templates",
        ));
    }
    let id = match req.id.as_deref().map(str::trim) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => slugify(name),
    };
    if id.is_empty() {
        return Err(AppError::bad_request(
            "id is empty; give an id or a name with letters or digits",
        ));
    }
    if state.storage.get_modbus_device(&id).await?.is_some() {
        return Err(err(
            StatusCode::CONFLICT,
            format!("A Modbus device '{id}' already exists; give another id"),
        ));
    }
    let registers: Vec<Value> = t
        .registers
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "offset": r.offset,
                "register_type": r.register_type,
                "register_area": r.register_area,
                "scale": r.scale,
            })
        })
        .collect();
    let body = json!({
        "id": id,
        "name": name,
        "ip": ip,
        "port": req.port.or(t.port).unwrap_or(crate::device_templates::DEFAULT_MODBUS_PORT),
        "unit_id": req.unit_id.or(t.unit_id).unwrap_or(1),
        "interval_ms": req.interval_ms.or(t.interval_ms).unwrap_or(crate::device_templates::DEFAULT_INTERVAL_MS),
        "endianness": req.endianness.clone().or_else(|| t.endianness.clone()).unwrap_or_else(|| "big".to_string()),
        "registers": registers,
    });
    // The device create route does the rest: asset inventory, poller, audit.
    super::modbus_devices::create_modbus_device(
        State(state.clone()),
        Extension(auth.clone()),
        Json(body),
    )
    .await?;
    Ok(
        json!({"ok": true, "protocol": "modbus", "id": id, "name": name, "item_count": t.registers.len()}),
    )
}

#[cfg(not(feature = "modbus"))]
async fn instantiate_modbus(
    _state: &AppState,
    _auth: &AuthUser,
    _t: &DeviceTemplate,
    _req: &InstantiateRequest,
    _name: &str,
    _ip: &str,
) -> Result<Value, AppError> {
    Err(AppError::bad_request("Modbus support is not built in"))
}

#[cfg(feature = "bacnet")]
async fn instantiate_bacnet(
    state: &AppState,
    auth: &AuthUser,
    t: &DeviceTemplate,
    req: &InstantiateRequest,
    name: &str,
    ip: &str,
) -> Result<Value, AppError> {
    if auth.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }
    if req.unit_id.is_some() || req.endianness.is_some() || req.id.is_some() {
        return Err(AppError::bad_request(
            "id, unit_id and endianness apply to Modbus templates",
        ));
    }
    let port = req
        .port
        .or(t.port)
        .unwrap_or(crate::device_templates::DEFAULT_BACNET_PORT);
    let instance = req.device_instance.unwrap_or(0);
    if instance > crate::device_templates::MAX_BACNET_INSTANCE {
        return Err(AppError::bad_request(format!(
            "device_instance must be 0-{}",
            crate::device_templates::MAX_BACNET_INSTANCE
        )));
    }
    let id = state
        .storage
        .create_bacnet_device(
            name,
            ip,
            port,
            instance,
            req.interval_ms
                .or(t.interval_ms)
                .unwrap_or(crate::device_templates::DEFAULT_INTERVAL_MS),
            req.cov_enabled.or(t.cov_enabled).unwrap_or(false),
            req.cov_lifetime
                .or(t.cov_lifetime)
                .unwrap_or(crate::device_templates::DEFAULT_COV_LIFETIME),
            req.enabled.unwrap_or(true),
        )
        .await?;
    for o in &t.objects {
        if let Err(e) = state
            .storage
            .create_bacnet_object(
                id,
                name,
                &o.object_type,
                o.object_instance,
                &o.property,
                &o.name,
                &o.display_name,
                o.cov_increment,
            )
            .await
        {
            // Leave no half-built device behind.
            let _ = state.storage.delete_bacnet_device(id).await;
            return Err(e.into());
        }
    }
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "bacnet",
        &id.to_string(),
        name,
        Some(ip),
        "controller",
    )
    .await;
    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_bacnet_device".to_string(),
        username: auth.username.clone(),
        resource: format!("bacnet_device:{id}"),
        details: Some(format!("name={name}, ip={ip}, template={}", t.template_id)),
    });
    Ok(
        json!({"ok": true, "protocol": "bacnet", "id": id, "name": name, "item_count": t.objects.len()}),
    )
}

#[cfg(not(feature = "bacnet"))]
async fn instantiate_bacnet(
    _state: &AppState,
    _auth: &AuthUser,
    _t: &DeviceTemplate,
    _req: &InstantiateRequest,
    _name: &str,
    _ip: &str,
) -> Result<Value, AppError> {
    Err(AppError::bad_request("BACnet support is not built in"))
}

/// POST /api/v1/device-templates/{id}/instantiate
///
/// Create a device with the template's registers or objects.
#[utoipa::path(
    post,
    path = "/api/v1/device-templates/{id}/instantiate",
    params(("id" = String, Path, description = "Template id")),
    request_body = InstantiateRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
        (status = 404),
        (status = 409, description = "A Modbus device with that id exists"),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn instantiate_template(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<InstantiateRequest>,
) -> Result<impl IntoResponse, AppError> {
    let t = find(
        &state,
        &id,
        &RefQuery {
            version: req.version.clone(),
            source: req.source.clone(),
        },
    )
    .await?;
    let name = req.name.trim();
    let ip = req.ip.trim();
    if name.is_empty() || name.chars().count() > 128 {
        return Err(AppError::bad_request(
            "name is required (at most 128 characters)",
        ));
    }
    if ip.is_empty() {
        return Err(AppError::bad_request("ip is required"));
    }
    if req.port == Some(0) {
        return Err(AppError::bad_request("port must be 1-65535"));
    }
    if req
        .interval_ms
        .is_some_and(|i| i == 0 || i > crate::device_templates::MAX_INTERVAL_MS)
    {
        return Err(AppError::bad_request(format!(
            "interval_ms must be 1-{}",
            crate::device_templates::MAX_INTERVAL_MS
        )));
    }
    if let Some(e) = &req.endianness
        && !crate::device_templates::MODBUS_ENDIANNESS.contains(&e.as_str())
    {
        return Err(AppError::bad_request(
            "endianness must be big, little or swapped",
        ));
    }
    let created = match t.protocol {
        Protocol::Modbus => instantiate_modbus(&state, &auth, &t, &req, name, ip).await?,
        Protocol::Bacnet => instantiate_bacnet(&state, &auth, &t, &req, name, ip).await?,
    };
    Ok((StatusCode::CREATED, Json(created)))
}

// ── Catalog ──

/// The catalog index URL: an explicit one (admin only), else the setting, else the public default.
async fn catalog_url(
    state: &AppState,
    auth: &AuthUser,
    explicit: Option<&str>,
) -> Result<String, AppError> {
    if let Some(url) = explicit.map(str::trim).filter(|u| !u.is_empty()) {
        if auth.role != "admin" {
            return Err(AppError::forbidden(
                "Only an admin can use a catalog other than the configured one",
            ));
        }
        catalog::validate_url(url).map_err(AppError::bad_request)?;
        return Ok(url.to_string());
    }
    let stored = state
        .storage
        .get_settings(&[SETTING_CATALOG_URL])
        .await?
        .into_iter()
        .next()
        .map(|s| s.value)
        .filter(|u| !u.trim().is_empty());
    Ok(stored.unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string()))
}

#[derive(Debug, Deserialize)]
pub struct CatalogQuery {
    pub url: Option<String>,
}

/// GET /api/v1/device-templates/catalog
///
/// Fetch the public catalog index and mark the templates already imported from it.
#[utoipa::path(
    get,
    path = "/api/v1/device-templates/catalog",
    params(("url" = Option<String>, Query, description = "Catalog index URL (admin only); default: the configured catalog")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
        (status = 422, description = "The catalog index is not valid"),
        (status = 502, description = "The catalog could not be fetched"),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_catalog(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<CatalogQuery>,
) -> Result<impl IntoResponse, AppError> {
    let url = catalog_url(&state, &auth, q.url.as_deref()).await?;
    let index = catalog::fetch_index(&url).await.map_err(catalog_err)?;
    let source = format!("{SOURCE_CATALOG_PREFIX}{url}");
    let stored = state.storage.list_device_templates().await?;
    let templates: Vec<Value> = index
        .templates
        .iter()
        .map(|e| {
            let installed = stored.iter().any(|t| {
                t.source == source && t.template_id == e.template_id && t.version == e.version
            });
            let mut v = serde_json::to_value(e).unwrap_or(Value::Null);
            if let Value::Object(m) = &mut v {
                m.insert("installed".into(), Value::Bool(installed));
            }
            v
        })
        .collect();
    Ok(Json(json!({
        "url": url,
        "format": index.format,
        "templates": templates,
    })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CatalogImportRequest {
    pub template_id: String,
    /// Default: the newest version in the catalog.
    pub version: Option<String>,
    /// Catalog index URL; default: the configured catalog.
    pub url: Option<String>,
}

/// POST /api/v1/device-templates/catalog/import
///
/// Download a template from the catalog, check its sha256 against the index and store it as `catalog:<url>`.
#[utoipa::path(
    post,
    path = "/api/v1/device-templates/catalog/import",
    request_body = CatalogImportRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 404, description = "The catalog does not list that template"),
        (status = 409, description = "Already imported"),
        (status = 422, description = "sha256 mismatch or the file is not a valid template"),
        (status = 502, description = "The catalog could not be fetched"),
    ),
    tag = "Device Templates",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn import_from_catalog(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<CatalogImportRequest>,
) -> Result<impl IntoResponse, AppError> {
    let url = catalog_url(&state, &auth, req.url.as_deref()).await?;
    let index = catalog::fetch_index(&url).await.map_err(catalog_err)?;
    let entry = index
        .find(&req.template_id, req.version.as_deref())
        .ok_or_else(|| {
            AppError::not_found(format!(
                "The catalog does not list '{}'{}",
                req.template_id,
                req.version
                    .as_deref()
                    .map(|v| format!(" version {v}"))
                    .unwrap_or_default()
            ))
        })?
        .clone();
    let mut t = catalog::fetch_template(&url, &entry)
        .await
        .map_err(catalog_err)?;
    t.source = format!("{SOURCE_CATALOG_PREFIX}{url}");
    store_new(&state, &auth, &t, "import_device_template").await?;
    Ok((StatusCode::CREATED, Json(created_body(&t))))
}
