//! Settings loading and the apply step of AI proposals.
//!
//! Applying a proposal goes through the REST handlers (the same ones the MCP
//! tools call), so it gets their validation and side effects: poller restart,
//! naming convention, flow graph checks, flow engine reload. Each apply checks
//! every item first and then writes; if a write still fails, the writes made
//! before it are undone, so a proposal is applied whole or not at all. When
//! the undo fails as well, the error carries [`UndoFailed`] and the proposal
//! is marked `failed` (see `proposals::apply_proposal`, which also marks a
//! proposal `applying` for the length of the writes).

use std::collections::HashSet;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};

use super::super::{AppError, AppState, AuthUser, Rejected};
use crate::ai_copilot::{AiProviderType, AiSettings, parser};

// ── Helpers ──

pub(crate) async fn load_ai_settings(state: &AppState) -> Result<AiSettings, AppError> {
    let provider_str = state
        .storage
        .get_ai_setting("provider")
        .await?
        .ok_or_else(|| {
            AppError(anyhow::anyhow!(
                "AI provider not configured. Please set up AI Copilot in settings."
            ))
        })?;
    let provider = AiProviderType::from_str(&provider_str)
        .ok_or_else(|| AppError(anyhow::anyhow!("Invalid AI provider: {}", provider_str)))?;
    let api_key = match crate::ai_copilot::key_store::load(&state.storage, &state.auth_secret)
        .await?
    {
        crate::ai_copilot::key_store::StoredKey::Readable(key) => key,
        // Optional for OpenAI-compatible servers; `validate` below catches the rest.
        crate::ai_copilot::key_store::StoredKey::None => String::new(),
        crate::ai_copilot::key_store::StoredKey::Unreadable => {
            return Err(AppError(anyhow::anyhow!(
                "The stored API key cannot be decrypted (auth_secret changed?); enter it again in the AI settings"
            )));
        }
    };
    let default_model = state
        .storage
        .get_ai_setting("default_model")
        .await?
        .filter(|m| !m.trim().is_empty());
    let base_url = state
        .storage
        .get_ai_setting("base_url")
        .await?
        .filter(|u| !u.trim().is_empty());

    let settings = AiSettings {
        provider,
        api_key,
        default_model,
        base_url,
    };
    settings.validate().map_err(AppError::bad_request)?;
    Ok(settings)
}

pub(crate) const PDF_GUIDANCE: &str = "PDF datasheets cannot be read yet. Open the PDF, copy the register table (or export it to CSV) and upload it as a .csv or .txt file.";

/// Refuse datasheet content that is a PDF (or other binary file) read as
/// text: the model would get garbage and could still answer with a
/// plausible-looking register map.
pub(crate) fn reject_pdf(file_type: &str, content: &str) -> Result<(), AppError> {
    let head = content.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("%PDF-") {
        return Err(AppError::bad_request(PDF_GUIDANCE));
    }
    let total = content.chars().count().max(1);
    let garbage = content
        .chars()
        .filter(|c| *c == '\u{fffd}' || *c == '\0')
        .count();
    if garbage * 100 > total {
        let msg = if file_type.trim().eq_ignore_ascii_case("pdf") {
            PDF_GUIDANCE.to_string()
        } else {
            "The datasheet looks like a binary file, not text. Upload the register table as a .csv or .txt file.".to_string()
        };
        return Err(AppError::bad_request(msg));
    }
    Ok(())
}

/// The tags an alarm threshold request is about, as `(id, key)`, resolved
/// exactly: by id when given (its key must match), else by key, which must
/// then belong to a single tag.
pub(crate) async fn resolve_threshold_tags(
    state: &AppState,
    tags: &[crate::models::AlarmThresholdTag],
) -> Result<Vec<(i64, String)>, AppError> {
    let all = state.storage.list_data_sources(None).await?;
    let mut out = Vec::with_capacity(tags.len());
    for t in tags {
        let ds = match t.id {
            Some(id) => all
                .iter()
                .find(|d| d.id == id && d.key == t.source_key)
                .ok_or_else(|| {
                    AppError::bad_request(format!("No tag with id {id} and key '{}'", t.source_key))
                })?,
            None => unique_by(&all, |d| d.key == t.source_key, &t.source_key, "key")?,
        };
        out.push((ds.id, ds.key.clone()));
    }
    Ok(out)
}

/// Tie each proposed threshold to one of the `requested` tags: by `tag_id`
/// (whose key must equal `source_key`), else by an exact `source_key` that
/// only one requested tag has. A threshold for any other tag is an error, as
/// is a second threshold for the same tag. Each result carries `tag_id`.
pub(crate) fn bind_thresholds(
    thresholds: Vec<Value>,
    requested: &[(i64, String)],
) -> Result<Vec<Value>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(thresholds.len());
    for mut t in thresholds {
        let key = t["source_key"].as_str().unwrap_or_default().to_string();
        let id = match t.get("tag_id").and_then(Value::as_i64) {
            Some(id) => {
                if !requested
                    .iter()
                    .any(|(rid, rkey)| *rid == id && *rkey == key)
                {
                    return Err(format!(
                        "the AI proposed a threshold for tag_id {id} / '{key}', which is not one of the requested tags"
                    ));
                }
                id
            }
            None => {
                let matches: Vec<i64> = requested
                    .iter()
                    .filter(|(_, rkey)| *rkey == key)
                    .map(|(id, _)| *id)
                    .collect();
                match matches.as_slice() {
                    [id] => *id,
                    [] => {
                        return Err(format!(
                            "the AI proposed a threshold for '{key}', which is not one of the requested tags"
                        ));
                    }
                    _ => {
                        return Err(format!(
                            "the AI gave no tag_id for '{key}', and several requested tags have that key"
                        ));
                    }
                }
            }
        };
        if !seen.insert(id) {
            return Err(format!(
                "the AI proposed two thresholds for tag {id} ('{key}')"
            ));
        }
        t["tag_id"] = json!(id);
        out.push(t);
    }
    Ok(out)
}

fn unique_by<'a, T>(
    items: &'a [T],
    pred: impl Fn(&T) -> bool,
    what: &str,
    field: &str,
) -> Result<&'a T, AppError> {
    let mut found = items.iter().filter(|i| pred(i));
    match (found.next(), found.next()) {
        (Some(one), None) => Ok(one),
        (None, _) => Err(AppError::bad_request(format!(
            "No tag with {field} '{what}'"
        ))),
        (Some(_), Some(_)) => Err(AppError::bad_request(format!(
            "Several tags have the {field} '{what}'; name the tag by its id"
        ))),
    }
}

// ── Calling the REST handlers ──

/// A REST handler's refusal: its status and `error` message.
struct Refused {
    status: StatusCode,
    message: String,
}

impl Refused {
    /// The refusal as the answer to the apply request, naming the item.
    fn about(self, item: &str) -> AppError {
        let status = if self.status.is_success() {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            self.status
        };
        AppError(anyhow::Error::new(Rejected(
            status,
            format!("{item}: {}", self.message),
        )))
    }
}

/// Run a REST handler's result to its JSON body, or its refusal. Some
/// handlers answer a refusal as `Ok` with a 4xx status, so the status decides.
async fn call<R: IntoResponse>(result: Result<R, AppError>) -> Result<Value, Refused> {
    let response = match result {
        Ok(r) => r.into_response(),
        Err(e) => e.into_response(),
    };
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .map_err(|e| Refused {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: format!("could not read the result: {e}"),
        })?;
    let body: Value = if bytes.is_empty() {
        json!({"ok": true})
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
    };
    if status.is_success() {
        return Ok(body);
    }
    let message = body
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| body.as_str().map(str::to_string))
        .unwrap_or_else(|| body.to_string());
    Err(Refused { status, message })
}

/// Attached to an apply error when undoing the writes made before the
/// failure failed too: the configuration may now hold part of the proposal.
#[derive(Debug)]
pub(crate) struct UndoFailed(pub Vec<String>);

impl std::fmt::Display for UndoFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Undoing the changes made before the failure also failed, so part of the proposal may be in the configuration; check it ({})",
            self.0.join("; ")
        )
    }
}

/// What could not be undone after a failed apply, if anything.
pub(crate) fn undo_failures(err: &AppError) -> Option<&[String]> {
    err.0.downcast_ref::<UndoFailed>().map(|u| u.0.as_slice())
}

/// `err` with the outcome of undoing the writes made before it.
fn rolled_back(err: AppError, undo_failures: Vec<String>) -> AppError {
    if undo_failures.is_empty() {
        return AppError(err.0.context("Nothing was applied"));
    }
    tracing::error!(
        "AI proposal apply: undoing a partial apply failed: {}",
        undo_failures.join("; ")
    );
    AppError(err.0.context(UndoFailed(undo_failures)))
}

async fn update_tag(state: &AppState, id: i64, body: Value) -> Result<Value, Refused> {
    call(
        super::super::data_sources::update_data_source(State(state.clone()), Path(id), Json(body))
            .await,
    )
    .await
}

async fn create_tag(state: &AppState, body: Value) -> Result<i64, Refused> {
    let out = call(
        super::super::data_sources::create_data_source(State(state.clone()), Json(body)).await,
    )
    .await?;
    out["id"].as_i64().ok_or_else(|| Refused {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: format!("the new tag has no id: {out}"),
    })
}

/// Put a tag's old values back, straight through storage: they were valid
/// when written, and going through the tag route would check the old name
/// against today's naming convention, which may refuse it.
async fn restore_tag(state: &AppState, id: i64, label: &str, old: &Value) -> Result<(), String> {
    match state.storage.update_data_source(id, old).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("tag '{label}': it no longer exists")),
        Err(e) => Err(format!("tag '{label}': {e:#}")),
    }
}

/// Delete tags made by a failed apply, newest first; returns what failed.
async fn delete_tags(state: &AppState, ids: &[i64]) -> Vec<String> {
    let mut failures = Vec::new();
    for id in ids.iter().rev() {
        if let Err(e) = state.storage.delete_data_source(*id).await {
            failures.push(format!("tag {id}: {e:#}"));
        }
    }
    failures
}

/// Check tag names against the naming convention before anything is written.
async fn check_tag_names(state: &AppState, names: Vec<String>) -> Result<(), AppError> {
    for name in &names {
        super::super::naming_convention::validate_tag_name(state, name)
            .await
            .map_err(|msg| AppError::bad_request(format!("tag name '{name}': {msg}")))?;
    }
    Ok(())
}

/// A tag update: `(id, label, new, old)`.
pub(in crate::api) type TagChange = (i64, String, Value, Value);

/// Update tags one after the other; on a failure, put back the ones already
/// changed.
pub(in crate::api) async fn update_tags_all_or_nothing(
    state: &AppState,
    plan: &[TagChange],
) -> Result<(), AppError> {
    for (done, (id, label, new, _)) in plan.iter().enumerate() {
        if let Err(e) = update_tag(state, *id, new.clone()).await {
            let mut undo_failures = Vec::new();
            for (id, label, _, old) in plan[..done].iter().rev() {
                if let Err(u) = restore_tag(state, *id, label, old).await {
                    undo_failures.push(u);
                }
            }
            return Err(rolled_back(
                e.about(&format!("tag '{label}'")),
                undo_failures,
            ));
        }
    }
    Ok(())
}

/// Each tag's values before and after, for the audit log (and to undo by hand).
fn tag_changes(plan: &[TagChange]) -> Value {
    plan.iter()
        .map(|(id, label, new, old)| json!({"tag_id": id, "tag": label, "before": old, "after": new}))
        .collect()
}

/// Create tags one after the other; on a failure, delete the ones created.
pub(in crate::api) async fn create_tags_all_or_nothing(
    state: &AppState,
    bodies: Vec<(String, Value)>,
) -> Result<Vec<i64>, AppError> {
    let mut created = Vec::new();
    for (label, body) in bodies {
        match create_tag(state, body).await {
            Ok(id) => created.push(id),
            Err(e) => {
                let undo_failures = delete_tags(state, &created).await;
                return Err(rolled_back(
                    e.about(&format!("tag '{label}'")),
                    undo_failures,
                ));
            }
        }
    }
    Ok(created)
}

// ── Apply, per proposal type ──

/// Add the proposed registers to an existing Modbus device (through the
/// register route, which validates them and restarts the poller) and create
/// a modbus tag for each register that has none, so its value shows up.
///
/// The tags are created first and the registers written last: a failure
/// then only has tags to delete, and the poller is restarted once, by the
/// write that succeeds.
#[cfg(feature = "modbus")]
pub(crate) async fn apply_register_map(
    state: &AppState,
    config: &Value,
    device_id: &Option<String>,
) -> Result<Value, AppError> {
    use super::super::modbus_devices::replace_registers;

    let device_id = device_id
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| {
            AppError::bad_request(
                "This register map has no target device. Create the Modbus device first, then generate the map with its device id.",
            )
        })?;
    let registers = parser::check_register_map(config).map_err(AppError::bad_request)?;
    if state.storage.get_modbus_device(device_id).await?.is_none() {
        return Err(AppError::bad_request(format!(
            "Modbus device '{device_id}' does not exist; create it first"
        )));
    }
    let existing = state.storage.list_modbus_registers(device_id).await?;
    for r in &registers {
        let name = r["name"].as_str().unwrap_or_default();
        if existing.iter().any(|e| e.name == name) {
            return Err(AppError::bad_request(format!(
                "Device '{device_id}' already has a register named '{name}'"
            )));
        }
    }

    // Registers that already have a tag (key = register name, or device.name) keep it.
    let tags = state.storage.list_data_sources(None).await?;
    let dotted = |name: &str| format!("{device_id}.{name}");
    let needs_tag: Vec<&Value> = registers
        .iter()
        .filter(|r| {
            let name = r["name"].as_str().unwrap_or_default();
            !tags
                .iter()
                .any(|t| t.device_id == device_id && (t.key == name || t.key == dotted(name)))
        })
        .collect();
    let names = needs_tag
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default().to_string())
        .collect();
    check_tag_names(state, names).await?;

    let old_registers: Vec<Value> = existing
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "address": e.address,
                "zero_based": e.zero_based,
                "register_type": e.register_type,
                "register_area": e.register_area,
                "scale": e.scale,
            })
        })
        .collect();
    let mut all_registers = old_registers.clone();
    all_registers.extend(registers.iter().map(|r| {
        let mut r = r.clone();
        if let Some(obj) = r.as_object_mut() {
            obj.remove("unit");
            obj.remove("description");
        }
        r
    }));

    let bodies = needs_tag
        .iter()
        .map(|r| {
            let name = r["name"].as_str().unwrap_or_default().to_string();
            let body = json!({
                "key": name,
                "name": name,
                "unit": r["unit"].as_str().unwrap_or(""),
                "source_type": "modbus",
                "device_id": device_id,
                "chart_type": "sparkline",
            });
            (name, body)
        })
        .collect();
    let created = create_tags_all_or_nothing(state, bodies).await?;

    let written = call(
        replace_registers(
            State(state.clone()),
            Path(device_id.to_string()),
            Json(json!({ "registers": all_registers })),
        )
        .await,
    )
    .await;
    if let Err(e) = written {
        let undo_failures = delete_tags(state, &created).await;
        return Err(rolled_back(
            e.about(&format!("registers of device '{device_id}'")),
            undo_failures,
        ));
    }
    Ok(json!({
        "device_id": device_id,
        "registers_added": registers.len(),
        "tags_created": created.len(),
        "tag_ids": created,
        "previous_registers": old_registers,
    }))
}

#[cfg(not(feature = "modbus"))]
pub(crate) async fn apply_register_map(
    _state: &AppState,
    _config: &Value,
    _device_id: &Option<String>,
) -> Result<Value, AppError> {
    Err(AppError::bad_request("Modbus support is not built in"))
}

/// Set the proposed warning and critical limits on exactly the tags they
/// name (by `tag_id`, else by an exact key that only one tag has).
pub(crate) async fn apply_alarm_thresholds(
    state: &AppState,
    config: &Value,
) -> Result<Value, AppError> {
    let thresholds = parser::check_alarm_thresholds(config).map_err(AppError::bad_request)?;
    let tags = state.storage.list_data_sources(None).await?;

    let mut seen = HashSet::new();
    let mut plan = Vec::with_capacity(thresholds.len());
    for t in &thresholds {
        let key = t["source_key"].as_str().unwrap_or_default();
        let ds = match t.get("tag_id").and_then(Value::as_i64) {
            Some(id) => {
                let ds = tags.iter().find(|d| d.id == id).ok_or_else(|| {
                    AppError::bad_request(format!("Tag {id} ('{key}') no longer exists"))
                })?;
                if ds.key != key {
                    return Err(AppError::bad_request(format!(
                        "Tag {id} has the key '{}', not '{key}'; regenerate the proposal",
                        ds.key
                    )));
                }
                ds
            }
            None => unique_by(&tags, |d| d.key == key, key, "key")?,
        };
        if !seen.insert(ds.id) {
            return Err(AppError::bad_request(format!(
                "Two thresholds are for the same tag '{key}'"
            )));
        }
        let warn = t["warn_val"].as_f64().unwrap_or(ds.warn_val);
        let crit = t["crit_val"].as_f64().unwrap_or(ds.crit_val);
        if warn != 0.0 && crit != 0.0 && warn >= crit {
            return Err(AppError::bad_request(format!(
                "'{key}': the warning limit {warn} must be below the critical limit {crit}"
            )));
        }
        plan.push((
            ds.id,
            ds.key.clone(),
            json!({"warn_val": warn, "crit_val": crit}),
            json!({"warn_val": ds.warn_val, "crit_val": ds.crit_val}),
        ));
    }
    update_tags_all_or_nothing(state, &plan).await?;
    Ok(json!({"tags_updated": plan.len(), "changes": tag_changes(&plan)}))
}

/// Rename tags. Each entry names its tag by `tag_id` (whose key or name must
/// be `current`), else `current` must name one tag exactly (by key, else by
/// name); every new name must pass the naming convention.
pub(crate) async fn apply_tag_rename(state: &AppState, config: &Value) -> Result<Value, AppError> {
    let renames = parser::parse_tag_rename(&config.to_string())
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let tags = state.storage.list_data_sources(None).await?;

    let mut seen = HashSet::new();
    let mut plan = Vec::with_capacity(renames.len());
    for entry in &renames {
        let current = entry["current"].as_str().unwrap_or("").trim();
        let proposed = entry["proposed"].as_str().unwrap_or("").trim();
        if current.is_empty() || proposed.is_empty() {
            return Err(AppError::bad_request(
                "Every rename needs a non-empty 'current' and 'proposed'",
            ));
        }
        let ds = match entry.get("tag_id").and_then(Value::as_i64) {
            Some(id) => {
                let ds = tags.iter().find(|d| d.id == id).ok_or_else(|| {
                    AppError::bad_request(format!("Tag {id} ('{current}') no longer exists"))
                })?;
                if ds.key != current && ds.name != current {
                    return Err(AppError::bad_request(format!(
                        "Tag {id} is '{}' (key '{}'), not '{current}'; regenerate the proposal",
                        ds.name, ds.key
                    )));
                }
                ds
            }
            None if tags.iter().any(|d| d.key == current) => {
                unique_by(&tags, |d| d.key == current, current, "key")?
            }
            None => unique_by(&tags, |d| d.name == current, current, "name")?,
        };
        if !seen.insert(ds.id) {
            return Err(AppError::bad_request(format!(
                "Tag '{current}' is renamed twice"
            )));
        }
        plan.push((
            ds.id,
            current.to_string(),
            json!({"name": proposed}),
            json!({"name": ds.name}),
        ));
    }
    let names = plan
        .iter()
        .map(|(_, _, new, _)| new["name"].as_str().unwrap_or_default().to_string())
        .collect();
    check_tag_names(state, names).await?;
    update_tags_all_or_nothing(state, &plan).await?;
    Ok(json!({"tags_renamed": plan.len(), "changes": tag_changes(&plan)}))
}

/// Save the proposed layout on an existing screen through the layouts route.
/// The result keeps the screen's layouts from before, so they can be put back.
pub(crate) async fn apply_dashboard_layout(
    state: &AppState,
    config: &Value,
    screen_id: &Option<String>,
) -> Result<Value, AppError> {
    let screen_id = screen_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::bad_request("No screen ID specified for dashboard layout"))?;
    if screen_id != "__overview"
        && !state
            .storage
            .list_devices()
            .await?
            .iter()
            .any(|d| d.id == screen_id)
    {
        return Err(AppError::bad_request(format!(
            "Screen '{screen_id}' does not exist"
        )));
    }

    let layout = config
        .get("layout")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::bad_request("Invalid dashboard layout format: missing 'layout' array")
        })?;
    for (i, item) in layout.iter().enumerate() {
        for (field, min) in [("x", 0), ("y", 0), ("w", 1), ("h", 1)] {
            if item
                .get(field)
                .and_then(Value::as_u64)
                .is_none_or(|v| v < min)
            {
                return Err(AppError::bad_request(format!(
                    "layout item {}: '{field}' must be a whole number of {min} or more",
                    i + 1
                )));
            }
        }
    }

    let previous: serde_json::Map<String, Value> = state
        .storage
        .get_layouts(screen_id)
        .await?
        .into_iter()
        .map(|row| {
            let layout = serde_json::from_str(&row.layout).unwrap_or(Value::String(row.layout));
            (row.breakpoint, layout)
        })
        .collect();

    call(
        super::super::layouts::save_layouts(
            State(state.clone()),
            Path(screen_id.to_string()),
            Json(json!({"lg": layout, "md": layout, "sm": layout})),
        )
        .await,
    )
    .await
    .map_err(|e| e.about(&format!("layout of screen '{screen_id}'")))?;
    Ok(json!({
        "screen_id": screen_id,
        "items": layout.len(),
        "previous_layouts": previous,
    }))
}

/// Create the proposed flow, disabled, through the flows route (graph checks,
/// flow engine, audit).
pub(crate) async fn apply_flow_rule(
    state: &AppState,
    auth: &AuthUser,
    config: &Value,
) -> Result<Value, AppError> {
    parser::parse_flow_rule(&config.to_string())
        .map_err(|e| AppError::bad_request(e.to_string()))?;
    let id = format!("ai_flow_{}", chrono_like_id());
    let input = super::super::flows::FlowInput {
        id: id.clone(),
        name: config["name"]
            .as_str()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or("AI-generated flow")
            .to_string(),
        // Disabled: an engineer enables it after reviewing it in the flow editor.
        enabled: false,
        trigger_config: config["trigger_config"].clone(),
        nodes: config["nodes"].clone(),
    };
    call(
        super::super::flows::create_flow(
            State(state.clone()),
            Extension(auth.clone()),
            Json(input),
        )
        .await,
    )
    .await
    .map_err(|e| e.about("flow"))?;
    Ok(json!({"flow_id": id}))
}

/// The device an equipment template is instantiated on must be an existing
/// Modbus device: its tags are modbus tags, read by that device's poller.
#[cfg(feature = "modbus")]
async fn check_template_device(state: &AppState, device_id: &str) -> Result<(), AppError> {
    if state.storage.get_modbus_device(device_id).await?.is_none() {
        return Err(AppError::bad_request(format!(
            "Modbus device '{device_id}' does not exist; create it first"
        )));
    }
    Ok(())
}

#[cfg(not(feature = "modbus"))]
async fn check_template_device(_state: &AppState, _device_id: &str) -> Result<(), AppError> {
    Err(AppError::bad_request("Modbus support is not built in"))
}

/// Create the template's tags through the tag route, on the target Modbus
/// device. They are modbus tags keyed by the tag name, the key the poller
/// gives a register's value (as for the tags of a register map), so they
/// show the value once a register of that name is mapped; the register area
/// and default type are kept in `source_config` until then.
pub(crate) async fn apply_equipment_template(
    state: &AppState,
    config: &Value,
    device_id: &Option<String>,
) -> Result<Value, AppError> {
    let tags = parser::check_equipment_template(config).map_err(AppError::bad_request)?;
    let dev_id = device_id
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .ok_or_else(|| {
            AppError::bad_request(
                "This equipment template has no target device. Create the Modbus device first, then instantiate the template with its device id.",
            )
        })?;
    check_template_device(state, dev_id).await?;

    let bodies: Vec<(String, Value)> = tags
        .into_iter()
        .map(|(name, tag)| {
            let body = json!({
                "key": name,
                "name": name,
                "unit": tag["unit"].as_str().unwrap_or(""),
                "source_type": "modbus",
                "source_config": {
                    "register_area": tag["register_area"].as_str().unwrap_or("holding"),
                    "default_type": tag["default_type"].as_str().unwrap_or("float32"),
                },
                "device_id": dev_id,
                "chart_type": "gauge",
            });
            (name, body)
        })
        .collect();
    let existing = state.storage.list_data_sources(None).await?;
    if let Some((name, _)) = bodies.iter().find(|(name, _)| {
        existing
            .iter()
            .any(|d| d.device_id == dev_id && d.key == *name)
    }) {
        return Err(AppError::bad_request(format!(
            "Device '{dev_id}' already has a tag '{name}'"
        )));
    }
    check_tag_names(state, bodies.iter().map(|(name, _)| name.clone()).collect()).await?;
    let created = create_tags_all_or_nothing(state, bodies).await?;
    Ok(json!({"device_id": dev_id, "tags_created": created.len(), "tag_ids": created}))
}

pub(crate) fn chrono_like_id() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1000) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_content_is_refused_with_guidance() {
        let err = reject_pdf("csv", "%PDF-1.7\n%âãÏÓ\n1 0 obj").unwrap_err();
        let resp = err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let garbage: String = std::iter::repeat_n('\u{fffd}', 50).collect::<String>() + "obj";
        assert!(reject_pdf("pdf", &garbage).is_err());
        assert!(reject_pdf("text", &garbage).is_err());
        assert!(reject_pdf("csv", "name,address\nvoltage,40001\n").is_ok());
        // Text someone extracted from a PDF themselves is fine.
        assert!(reject_pdf("pdf", "Register 40001 Voltage L1-N float32 V").is_ok());
    }

    #[test]
    fn thresholds_bind_to_requested_tags_exactly() {
        let requested = vec![
            (1, "pump1:temp".to_string()),
            (2, "pump1:temp_bearing".to_string()),
        ];
        let bound = bind_thresholds(
            vec![
                json!({"source_key": "pump1:temp", "warn_val": 80}),
                json!({"source_key": "pump1:temp_bearing", "tag_id": 2, "warn_val": 70}),
            ],
            &requested,
        )
        .unwrap();
        assert_eq!(bound[0]["tag_id"], 1);
        assert_eq!(bound[1]["tag_id"], 2);

        // A near miss is not "close enough".
        for bad in [
            json!({"source_key": "pump1", "warn_val": 1}),
            json!({"source_key": "pump1:temp_b", "warn_val": 1}),
            json!({"source_key": "pump1:temp", "tag_id": 2, "warn_val": 1}),
            json!({"source_key": "pump9:temp", "tag_id": 9, "warn_val": 1}),
        ] {
            assert!(
                bind_thresholds(vec![bad.clone()], &requested).is_err(),
                "{bad}"
            );
        }
        // Two tags with one key (two devices) need the id.
        let twins = vec![(1, "temp".to_string()), (2, "temp".to_string())];
        assert!(
            bind_thresholds(vec![json!({"source_key": "temp", "warn_val": 1})], &twins).is_err()
        );
        assert!(
            bind_thresholds(
                vec![json!({"source_key": "temp", "tag_id": 2, "warn_val": 1})],
                &twins
            )
            .is_ok()
        );
        assert!(
            bind_thresholds(
                vec![
                    json!({"source_key": "pump1:temp", "warn_val": 1}),
                    json!({"source_key": "pump1:temp", "tag_id": 1, "warn_val": 2}),
                ],
                &requested
            )
            .is_err()
        );
    }
}
