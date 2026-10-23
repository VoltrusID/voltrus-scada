use anyhow::{Context, Result};
use rand::Rng;

use crate::models::{Command, Recipe};
use crate::storage::Storage;

/// Generate a random 12-character alphanumeric ID for commands.
fn random_command_suffix() -> String {
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(12)
        .map(char::from)
        .collect::<String>()
        .to_lowercase()
}

/// Load a recipe's values to the target PLC/device by creating real commands
/// in the command queue. Returns a list of created command IDs.
pub async fn load_recipe_to_plc(recipe: &Recipe, storage: &Storage) -> Result<Vec<String>> {
    let values: std::collections::HashMap<String, f64> = serde_json::from_str(&recipe.values)
        .with_context(|| format!("Failed to parse recipe {} values JSON", recipe.id))?;

    tracing::info!(
        "Loading recipe '{}' (v{}) to device '{}': {} registers",
        recipe.name,
        recipe.version,
        recipe.device_id,
        values.len(),
    );

    let mut command_ids = Vec::new();
    let now = crate::timeutil::now_secs();

    for (register_name, value) in &values {
        // Look up the data source by device_id and key (register name)
        let ds = match storage
            .find_data_source_by_device_and_key(&recipe.device_id, register_name)
            .await
        {
            Ok(Some(ds)) => ds,
            Ok(None) => {
                tracing::warn!(
                    "  Skipping '{}': no data_source found for device '{}' with key '{}'",
                    register_name,
                    recipe.device_id,
                    register_name,
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    "  Skipping '{}': error looking up data_source: {}",
                    register_name,
                    e,
                );
                continue;
            }
        };

        // Get device info (device_id integer, protocol) for the command
        let (device_id_int, protocol) = match storage.get_data_source_device_id(ds.id).await {
            Ok(Some(info)) => info,
            Ok(None) => {
                tracing::warn!(
                    "  Skipping '{}': device info not found for data_source_id {}",
                    register_name,
                    ds.id,
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    "  Skipping '{}': error looking up device info: {}",
                    register_name,
                    e,
                );
                continue;
            }
        };

        let cmd_id = format!("cmd_{}", random_command_suffix());
        let cmd = Command {
            id: cmd_id.clone(),
            data_source_id: ds.id,
            device_id: device_id_int,
            protocol,
            value: value.to_string(),
            status: "pending".to_string(),
            policy: "immediate".to_string(),
            scheduled_at: None,
            retry_count: 0,
            max_retries: 3,
            error_message: None,
            created_at: now,
            sent_at: None,
            acked_at: None,
            resolved_at: None,
            created_by: format!("recipe:{}", recipe.name),
            readback_status: None,
            readback_value: None,
            readback_attempts: 0,
            readback_ts: None,
        };

        match storage.insert_command(&cmd).await {
            Ok(()) => {
                tracing::info!(
                    "  Queued command {} for register '{}' = {} (data_source_id: {})",
                    cmd_id,
                    register_name,
                    value,
                    ds.id,
                );
                command_ids.push(cmd_id);
            }
            Err(e) => {
                tracing::warn!(
                    "  Failed to queue command for register '{}': {}",
                    register_name,
                    e,
                );
            }
        }
    }

    tracing::info!(
        "Recipe '{}' load complete: {} of {} commands queued",
        recipe.name,
        command_ids.len(),
        values.len(),
    );

    // Log the recipe load event for audit trail
    if let Err(e) = storage
        .log_audit_event(
            "recipe_load",
            "system",
            &format!("recipe:{}:device:{}", recipe.id, recipe.device_id),
            Some(&format!(
                "Loaded recipe '{}' v{} to device '{}': {} commands queued",
                recipe.name,
                recipe.version,
                recipe.device_id,
                command_ids.len(),
            )),
        )
        .await
    {
        tracing::warn!("Failed to log recipe load audit event: {}", e);
    }

    Ok(command_ids)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_parse_recipe_values() {
        let json = r#"{"temp": 25.0, "pressure": 1.5}"#;
        let values: std::collections::HashMap<String, f64> = serde_json::from_str(json).unwrap();
        assert_eq!(values.len(), 2);
        assert!((values["temp"] - 25.0).abs() < 0.001);
        assert!((values["pressure"] - 1.5).abs() < 0.001);
    }
}
