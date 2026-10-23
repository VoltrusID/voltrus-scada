//! OEE state machine: monitors device metrics to detect machine state transitions.
//!
//! The monitor runs as a background task. It watches device cache values and
//! creates/closes downtime events when a device transitions between states.
//! Thresholds are configurable per device via the settings KV store.

use crate::cache::DeviceCache;
use crate::storage::Storage;
use std::collections::HashMap;
use std::time::Duration;

/// Settings key prefix for per-device OEE thresholds.
const OEE_THRESHOLD_PREFIX: &str = "oee_threshold_";

/// Default threshold: a device is considered "running" if the primary metric > 0.
/// If the metric drops to 0 or below, the device transitions to "idle".
/// Custom thresholds override this: key = "oee_threshold_{device_id}", value = JSON { "metric": "...", "running_above": N }
const DEFAULT_RUNNING_ABOVE: f64 = 0.0;

/// How often the OEE monitor checks for state transitions.
const MONITOR_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone)]
struct DeviceOeeState {
    current_state: String,
    open_event_id: Option<i64>,
}

/// Load per-device OEE threshold config from settings.
/// Returns (metric_name, running_above_threshold).
async fn get_device_threshold(storage: &Storage, device_id: &str) -> (Option<String>, f64) {
    let key = format!("{}{}", OEE_THRESHOLD_PREFIX, device_id);
    match storage.get_settings(&[&key]).await {
        Ok(settings) => {
            if let Some(setting) = settings.into_iter().next()
                && let Ok(config) = serde_json::from_str::<serde_json::Value>(&setting.value)
            {
                let metric = config
                    .get("metric")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let running_above = config
                    .get("running_above")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(DEFAULT_RUNNING_ABOVE);
                return (metric, running_above);
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load OEE threshold for {}: {}", device_id, e);
        }
    }
    (None, DEFAULT_RUNNING_ABOVE)
}

/// Determine machine state based on metric value and threshold.
fn determine_state(value: f64, running_above: f64) -> &'static str {
    if value > running_above {
        "running"
    } else {
        "idle"
    }
}

/// Background task that monitors device metrics and creates/closes downtime events.
pub async fn run_oee_monitor(cache: DeviceCache, storage: Storage) {
    let mut interval = tokio::time::interval(Duration::from_secs(MONITOR_INTERVAL_SECS));
    let mut device_states: HashMap<String, DeviceOeeState> = HashMap::new();

    loop {
        interval.tick().await;

        let snapshots = cache.get_all().await;
        if snapshots.is_empty() {
            continue;
        }

        for snap in &snapshots {
            if snap.status != crate::models::DeviceStatus::Online {
                continue;
            }

            if snap.metrics.is_empty() {
                continue;
            }

            let device_id = &snap.device_id;
            let (threshold_metric, running_above) = get_device_threshold(&storage, device_id).await;

            // Determine the value to evaluate
            let (value, _metric_name) = if let Some(ref metric_name) = threshold_metric {
                // Use the configured metric
                match snap.metrics.get(metric_name) {
                    Some(v) => (*v, metric_name.clone()),
                    None => continue, // metric not found, skip
                }
            } else {
                // Use the first available metric
                let first = snap.metrics.iter().next().unwrap();
                (*first.1, first.0.clone())
            };

            let new_state = determine_state(value, running_above);
            let prev_state = device_states
                .get(device_id)
                .map(|s| s.current_state.as_str())
                .unwrap_or("running"); // Assume running on first observation

            if new_state != prev_state {
                // State transition detected
                if new_state == "running" {
                    // Close any open downtime event
                    if let Some(existing) = device_states.get(device_id)
                        && let Some(event_id) = existing.open_event_id
                        && let Err(e) = storage
                            .close_downtime_event(event_id, Some("Auto-resumed: running detected"))
                            .await
                    {
                        tracing::warn!("Failed to close downtime event for {}: {}", device_id, e);
                    }
                } else {
                    // Create a new downtime event
                    match storage.create_downtime_event(device_id, new_state).await {
                        Ok(event_id) => {
                            tracing::info!(
                                "OEE: device {} transitioned {} -> {}, created downtime event #{}",
                                device_id,
                                prev_state,
                                new_state,
                                event_id
                            );
                            device_states.insert(
                                device_id.clone(),
                                DeviceOeeState {
                                    current_state: new_state.to_string(),
                                    open_event_id: Some(event_id),
                                },
                            );
                            continue; // skip the update below
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to create downtime event for {}: {}",
                                device_id,
                                e
                            );
                        }
                    }
                }

                device_states.insert(
                    device_id.clone(),
                    DeviceOeeState {
                        current_state: new_state.to_string(),
                        open_event_id: None,
                    },
                );
            }
        }
    }
}
