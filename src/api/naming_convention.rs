use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

const ISA_5_1_DEFAULT: &str = r"^[A-Za-z0-9_ .-]+$";

const BUILTIN_TEMPLATES: &[(&str, &str, &str)] = &[
    (
        "isa-5.1",
        "ISA-5.1 (e.g., PT001_TEMP, TT102_ALARM)",
        ISA_5_1_DEFAULT,
    ),
    (
        "iec-61346",
        "IEC 61346 (e.g., =PLT+CV101-TT001)",
        r"^=[A-Z]+\+[A-Z0-9]+-[A-Z0-9]+$",
    ),
    (
        "kks",
        "KKS (e.g., LAB10CT001)",
        r"^[A-Z]{2}\d{2}[A-Z]{2}\d{3}$",
    ),
    ("custom", "Custom (user-defined regex)", ""),
    (
        "permissive",
        "Permissive (accept any name)",
        r"^[A-Za-z0-9_ .-]+$",
    ),
];

/// GET /api/v1/settings/naming-convention
pub async fn get_naming_convention(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let settings = state
        .storage
        .get_settings(&["naming_template", "naming_regex"])
        .await?;

    let mut template = "permissive".to_string();
    let mut custom_regex: Option<String> = None;

    for s in &settings {
        match s.key.as_str() {
            "naming_template" => template = s.value.clone(),
            "naming_regex" if !s.value.is_empty() => {
                custom_regex = Some(s.value.clone());
            }
            _ => {}
        }
    }

    let effective_regex = match template.as_str() {
        "custom" => custom_regex
            .clone()
            .unwrap_or_else(|| ISA_5_1_DEFAULT.to_string()),
        _ => BUILTIN_TEMPLATES
            .iter()
            .find(|(t, _, _)| *t == template)
            .map(|(_, _, regex)| regex.to_string())
            .unwrap_or_else(|| ISA_5_1_DEFAULT.to_string()),
    };

    Ok(Json(json!({
        "template": template,
        "regex": effective_regex,
        "custom_regex": custom_regex,
        "templates": BUILTIN_TEMPLATES.iter().map(|(id, label, _)| {
            json!({"id": id, "label": label})
        }).collect::<Vec<_>>(),
    })))
}

/// PUT /api/v1/settings/naming-convention
pub async fn update_naming_convention(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    let template = body
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("isa-5.1")
        .to_string();

    let valid_template = BUILTIN_TEMPLATES.iter().any(|(t, _, _)| *t == template);

    if !valid_template {
        return Err(AppError(anyhow::anyhow!(
            "Invalid template. Available: {}",
            BUILTIN_TEMPLATES
                .iter()
                .map(|(t, _, _)| *t)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let custom_regex = body
        .get("custom_regex")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if template == "custom" && !custom_regex.is_empty() && regex::Regex::new(custom_regex).is_err()
    {
        return Err(AppError(anyhow::anyhow!("Invalid regex pattern")));
    }

    state
        .storage
        .upsert_setting("naming_template", &template)
        .await?;
    state
        .storage
        .upsert_setting("naming_regex", custom_regex)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_naming_convention".to_string(),
        username: auth.username.clone(),
        resource: "settings:naming-convention".to_string(),
        details: Some(format!(
            "template={}, custom_regex={}",
            template, custom_regex
        )),
    });

    Ok(Json(json!({"ok": true})))
}

/// Validate a tag name against the current naming convention.
/// Returns Err with a human-readable message if the name does not match.
pub async fn validate_tag_name(state: &AppState, name: &str) -> Result<(), String> {
    let settings = state
        .storage
        .get_settings(&["naming_template", "naming_regex"])
        .await
        .map_err(|e| format!("Failed to read naming settings: {:#}", e))?;

    let mut template = "permissive".to_string();
    let mut custom_regex = String::new();

    for s in &settings {
        match s.key.as_str() {
            "naming_template" => template = s.value.clone(),
            "naming_regex" => custom_regex = s.value.clone(),
            _ => {}
        }
    }

    let effective_regex = match template.as_str() {
        "custom" if !custom_regex.is_empty() => custom_regex,
        _ => BUILTIN_TEMPLATES
            .iter()
            .find(|(t, _, _)| *t == template)
            .map(|(_, _, regex)| regex.to_string())
            .unwrap_or_else(|| ISA_5_1_DEFAULT.to_string()),
    };

    if let Ok(re) = regex::Regex::new(&effective_regex)
        && !re.is_match(name)
    {
        return Err(format!(
            "Tag name '{}' does not match naming convention (pattern: {})",
            name, effective_regex
        ));
    }

    Ok(())
}
