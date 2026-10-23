use serde_json::json;

use super::super::{AppError, AppState};
use crate::ai_copilot::{AiProviderType, AiSettings};

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
    let api_key = state
        .storage
        .get_ai_setting("api_key")
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("API key not configured")))?;
    let default_model = state.storage.get_ai_setting("default_model").await?;

    Ok(AiSettings {
        provider,
        api_key,
        default_model,
    })
}

pub(crate) async fn apply_register_map(
    state: &AppState,
    config: &serde_json::Value,
    device_id: &Option<String>,
) -> Result<(), AppError> {
    let device_id_str = device_id
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("No device ID specified for register map")))?
        .as_str();

    let registers = config
        .as_array()
        .ok_or_else(|| AppError(anyhow::anyhow!("Invalid register map format")))?;

    for reg in registers {
        let name = reg["name"].as_str().unwrap_or("unnamed");
        let address = reg["address"].as_i64().unwrap_or(0) as u16;
        let register_type = reg["register_type"].as_str().unwrap_or("float32");
        let scale = reg["scale"].as_f64().unwrap_or(1.0);

        state
            .storage
            .create_data_source(
                &format!("{}:{}", device_id_str, name),
                name,
                reg["unit"].as_str().unwrap_or(""),
                "#f97316",
                0.0, // min_val
                0.0, // max_val
                0.0, // warn_val
                0.0, // crit_val
                0.0, // base_val
                0,   // sort_order
                1,   // decimals
                "modbus",
                &serde_json::json!({
                    "device_id": device_id_str,
                    "address": address,
                    "register_type": register_type,
                    "scale": scale,
                })
                .to_string(),
                device_id_str,
                None,    // equipment_id
                "gauge", // chart_type
            )
            .await
            .map_err(AppError)?;
    }

    Ok(())
}

pub(crate) async fn apply_alarm_thresholds(
    state: &AppState,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    let thresholds = config
        .as_array()
        .ok_or_else(|| AppError(anyhow::anyhow!("Invalid alarm thresholds format")))?;

    let data_sources = state.storage.list_data_sources(None).await?;

    for threshold in thresholds {
        let source_key = threshold["source_key"]
            .as_str()
            .ok_or_else(|| AppError(anyhow::anyhow!("Missing source_key in threshold")))?;

        if let Some(ds) = data_sources
            .iter()
            .find(|ds| ds.key.contains(source_key) || source_key.contains(&ds.key))
        {
            let warn_val = threshold["warn_val"].as_f64().unwrap_or(ds.warn_val);
            let crit_val = threshold["crit_val"].as_f64().unwrap_or(ds.crit_val);

            state
                .storage
                .update_data_source(ds.id, &json!({"warn_val": warn_val, "crit_val": crit_val}))
                .await
                .map_err(AppError)?;
        }
    }

    Ok(())
}

pub(crate) async fn apply_tag_rename(
    state: &AppState,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    let renames = config
        .as_array()
        .ok_or_else(|| AppError(anyhow::anyhow!("Invalid tag rename format")))?;

    let data_sources = state.storage.list_data_sources(None).await?;

    for entry in renames {
        let current = entry["current"].as_str().unwrap_or("");
        let proposed = entry["proposed"].as_str().unwrap_or("");
        if current.is_empty() || proposed.is_empty() {
            continue;
        }

        if let Some(ds) = data_sources
            .iter()
            .find(|ds| ds.name == current || ds.key == current)
        {
            state
                .storage
                .update_data_source(ds.id, &json!({"name": proposed}))
                .await
                .map_err(AppError)?;
        }
    }

    Ok(())
}

pub(crate) async fn apply_dashboard_layout(
    state: &AppState,
    config: &serde_json::Value,
    screen_id: &Option<String>,
) -> Result<(), AppError> {
    let screen_id = screen_id.as_ref().ok_or_else(|| {
        AppError(anyhow::anyhow!(
            "No screen ID specified for dashboard layout"
        ))
    })?;

    let layout = config
        .get("layout")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            AppError(anyhow::anyhow!(
                "Invalid dashboard layout format: missing 'layout' array"
            ))
        })?;

    let layouts_array = serde_json::json!([
        {"breakpoint": "lg", "layout": layout},
        {"breakpoint": "md", "layout": layout},
        {"breakpoint": "sm", "layout": layout}
    ]);

    state
        .storage
        .save_layouts(screen_id, &layouts_array)
        .await
        .map_err(AppError)?;

    Ok(())
}

pub(crate) async fn apply_flow_rule(
    state: &AppState,
    config: &serde_json::Value,
) -> Result<(), AppError> {
    let name = config["name"].as_str().unwrap_or("AI-generated flow");
    let trigger_config = config["trigger_config"].to_string();
    let nodes = config["nodes"].to_string();

    let id = format!("ai_flow_{}", chrono_like_id());

    state
        .storage
        .create_flow(
            &id,
            name,
            false, // Disabled by default — engineer must enable
            &trigger_config,
            &nodes,
        )
        .await
        .map_err(AppError)?;

    Ok(())
}

pub(crate) async fn apply_equipment_template(
    state: &AppState,
    config: &serde_json::Value,
    device_id: &Option<String>,
) -> Result<(), AppError> {
    if let Some(tags) = config["tags"].as_array() {
        let prefix = config["prefix"].as_str().unwrap_or("equip");
        let dev_id = device_id.as_deref().unwrap_or(prefix);

        for tag in tags {
            let name = tag["name_pattern"]
                .as_str()
                .unwrap_or("TAG")
                .replace("{n}", "1");
            state.storage.create_data_source(
                &format!("{}:{}", prefix, name),
                &name,
                tag["unit"].as_str().unwrap_or(""),
                "#f97316",
                0.0, 0.0, 0.0, 0.0, 0.0, // min/max/warn/crit/base
                0,   // sort_order
                1,   // decimals
                tag["register_area"].as_str().unwrap_or("holding"),
                &serde_json::json!({"default_type": tag["default_type"].as_str().unwrap_or("float32")}).to_string(),
                dev_id,
                None,   // equipment_id
                "gauge", // chart_type
            ).await.map_err(AppError)?;
        }
    }
    Ok(())
}

pub(crate) fn chrono_like_id() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1000) as i64
}
