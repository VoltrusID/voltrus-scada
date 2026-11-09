use anyhow::{Context, Result};
use rand::Rng;

use crate::cache::DeviceCache;
use crate::models::Recipe;
use crate::storage::Storage;

/// The outcome of loading a recipe.
#[derive(Debug, Default)]
pub struct RecipeLoad {
    /// Commands queued, one per accepted value.
    pub command_ids: Vec<String>,
    /// `(register, why)` for each value that was not queued.
    pub refused: Vec<(String, String)>,
}

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
/// Every value goes through the tag's write policy, like a write from the API;
/// values the policy refuses are reported in `RecipeLoad::refused` and not sent.
pub async fn load_recipe_to_plc(
    recipe: &Recipe,
    storage: &Storage,
    cache: &DeviceCache,
    username: &str,
) -> Result<RecipeLoad> {
    let values: std::collections::HashMap<String, f64> = serde_json::from_str(&recipe.values)
        .with_context(|| format!("Failed to parse recipe {} values JSON", recipe.id))?;

    tracing::info!(
        "Loading recipe '{}' (v{}) to device '{}': {} registers",
        recipe.name,
        recipe.version,
        recipe.device_id,
        values.len(),
    );

    let mut load = RecipeLoad::default();

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

        let submission = crate::api::commands::TagWriteSubmission {
            tag_id: ds.id,
            value: *value,
            policy: "immediate".to_string(),
            scheduled_at: None,
            reason: Some(format!("recipe {} v{}", recipe.name, recipe.version)),
            command_id: None,
        };
        match crate::api::commands::enqueue_checked_write(storage, cache, username, submission)
            .await
        {
            Ok(cmd) => {
                tracing::info!(
                    "  Queued command {} for register '{}' = {} (data_source_id: {})",
                    cmd.id,
                    register_name,
                    value,
                    ds.id,
                );
                load.command_ids.push(cmd.id);
            }
            Err(e) => {
                let why = e.message();
                tracing::warn!("  Refused '{}' = {}: {}", register_name, value, why);
                load.refused.push((register_name.clone(), why));
            }
        }
    }

    tracing::info!(
        "Recipe '{}' load complete: {} of {} commands queued",
        recipe.name,
        load.command_ids.len(),
        values.len(),
    );

    // Log the recipe load event for audit trail
    if let Err(e) = storage
        .log_audit_event(
            "recipe_load",
            username,
            &format!("recipe:{}:device:{}", recipe.id, recipe.device_id),
            Some(&format!(
                "Loaded recipe '{}' v{} to device '{}': {} commands queued, {} refused",
                recipe.name,
                recipe.version,
                recipe.device_id,
                load.command_ids.len(),
                load.refused.len(),
            )),
        )
        .await
    {
        tracing::warn!("Failed to log recipe load audit event: {}", e);
    }

    Ok(load)
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
