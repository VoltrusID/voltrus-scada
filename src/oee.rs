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

/// Default threshold: a device is considered "running" if its state metric > 0.
/// If the metric drops to 0 or below, the device transitions to "idle".
/// Custom thresholds override this: key = "oee_threshold_{device_id}", value = JSON { "metric": "...", "running_above": N }
/// Without a configured metric only a device with a single metric is tracked.
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

/// The value that decides a device's state: the configured metric, or the
/// only metric of a device that has one. A device with several metrics and
/// no configured one is not tracked: picking one of them would be a guess.
/// Only a Good, finite reading counts; anything else leaves the state as it
/// is rather than calling a machine idle because its value is unknown.
fn state_reading(snap: &crate::models::DeviceSnapshot, metric: Option<&str>) -> Option<f64> {
    let name = match metric {
        Some(m) => m,
        None if snap.metrics.len() == 1 => snap.metrics.keys().next()?.as_str(),
        None => return None,
    };
    let value = *snap.metrics.get(name)?;
    (value.is_finite() && snap.quality_of(name) == crate::quality::Quality::Good).then_some(value)
}

/// Background task that monitors device metrics and creates/closes downtime events.
pub async fn run_oee_monitor(cache: DeviceCache, storage: Storage) {
    let mut interval = tokio::time::interval(Duration::from_secs(MONITOR_INTERVAL_SECS));
    let mut device_states: HashMap<String, DeviceOeeState> = HashMap::new();

    loop {
        interval.tick().await;
        if !crate::node_role::is_active() {
            continue;
        }
        for snap in &cache.get_all().await {
            evaluate_device(&storage, &mut device_states, snap).await;
        }
    }
}

/// One monitor tick for one device.
async fn evaluate_device(
    storage: &Storage,
    device_states: &mut HashMap<String, DeviceOeeState>,
    snap: &crate::models::DeviceSnapshot,
) {
    if snap.status != crate::models::DeviceStatus::Online {
        return;
    }
    let device_id = &snap.device_id;
    let (threshold_metric, running_above) = get_device_threshold(storage, device_id).await;
    let Some(value) = state_reading(snap, threshold_metric.as_deref()) else {
        return;
    };

    if !device_states.contains_key(device_id) {
        // First look since start: an idle event a previous run left open is
        // picked up, so it closes when the machine runs again (now, if it is
        // already running) instead of staying open for ever.
        let adopted = match storage.get_open_downtime_event(device_id).await {
            Ok(Some(event)) if event.state == "idle" => DeviceOeeState {
                current_state: "idle".to_string(),
                open_event_id: Some(event.id),
            },
            Ok(_) => DeviceOeeState {
                current_state: "running".to_string(),
                open_event_id: None,
            },
            Err(e) => {
                tracing::warn!("Failed to load open downtime event for {device_id}: {e}");
                return;
            }
        };
        device_states.insert(device_id.clone(), adopted);
    }

    let new_state = determine_state(value, running_above);
    let prev = &device_states[device_id];
    if new_state == prev.current_state {
        return;
    }
    let prev_state = prev.current_state.clone();
    let mut open_event_id = None;
    if new_state == "running" {
        // Close any open downtime event
        if let Some(event_id) = prev.open_event_id
            && let Err(e) = storage
                .close_downtime_event(event_id, Some("Auto-resumed: running detected"))
                .await
        {
            tracing::warn!("Failed to close downtime event for {}: {}", device_id, e);
        }
    } else {
        match storage.create_downtime_event(device_id, new_state).await {
            Ok(event_id) => {
                tracing::info!(
                    "OEE: device {} transitioned {} -> {}, created downtime event #{}",
                    device_id,
                    prev_state,
                    new_state,
                    event_id
                );
                open_event_id = Some(event_id);
            }
            Err(e) => {
                tracing::warn!("Failed to create downtime event for {}: {}", device_id, e);
            }
        }
    }
    device_states.insert(
        device_id.clone(),
        DeviceOeeState {
            current_state: new_state.to_string(),
            open_event_id,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::Quality;

    async fn snapshot(cache: &DeviceCache, id: &str) -> crate::models::DeviceSnapshot {
        cache
            .set_status(id, crate::models::DeviceStatus::Online)
            .await;
        cache
            .get_all()
            .await
            .into_iter()
            .find(|s| s.device_id == id)
            .unwrap()
    }

    #[tokio::test]
    async fn only_good_readings_of_a_known_metric_count() {
        let cache = DeviceCache::new();
        cache
            .update_with_quality(
                "m1",
                HashMap::from([("speed".into(), 0.0), ("temp".into(), 40.0)]),
                HashMap::from([("speed".into(), Quality::BadCommFailure)]),
                false,
            )
            .await;
        let snap = snapshot(&cache, "m1").await;
        assert_eq!(state_reading(&snap, Some("temp")), Some(40.0));
        assert_eq!(state_reading(&snap, Some("speed")), None, "bad quality");
        assert_eq!(state_reading(&snap, Some("nope")), None);
        assert_eq!(state_reading(&snap, None), None, "two metrics, none chosen");

        cache
            .update("m2", HashMap::from([("rpm".into(), f64::NAN)]), false)
            .await;
        let snap = snapshot(&cache, "m2").await;
        assert_eq!(state_reading(&snap, None), None, "NaN");
        cache
            .update("m2", HashMap::from([("rpm".into(), 1200.0)]), false)
            .await;
        let snap = snapshot(&cache, "m2").await;
        assert_eq!(state_reading(&snap, None), Some(1200.0), "the only metric");
    }

    #[tokio::test]
    async fn an_event_left_open_by_a_restart_is_closed_when_running() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        let left_open = storage.create_downtime_event("m3", "idle").await.unwrap();

        // Fresh monitor (after a restart); the machine is running.
        let mut states = HashMap::new();
        cache
            .update("m3", HashMap::from([("rpm".into(), 900.0)]), false)
            .await;
        evaluate_device(&storage, &mut states, &snapshot(&cache, "m3").await).await;
        assert!(
            storage
                .get_open_downtime_event("m3")
                .await
                .unwrap()
                .is_none()
        );
        let events = storage
            .list_downtime_events(Some("m3"), None, None)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, left_open);
        assert!(events[0].ended_at.is_some());
    }

    #[tokio::test]
    async fn an_idle_machine_keeps_its_open_event_across_a_restart() {
        let storage = Storage::new_in_memory().unwrap();
        let cache = DeviceCache::new();
        let left_open = storage.create_downtime_event("m4", "idle").await.unwrap();
        let mut states = HashMap::new();
        cache
            .update("m4", HashMap::from([("rpm".into(), 0.0)]), false)
            .await;
        evaluate_device(&storage, &mut states, &snapshot(&cache, "m4").await).await;
        let open = storage
            .get_open_downtime_event("m4")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(open.id, left_open, "no second event for the same stop");

        // A bad reading changes nothing; a good running one closes it.
        cache
            .update_with_quality(
                "m4",
                HashMap::from([("rpm".into(), 0.0)]),
                HashMap::from([("rpm".into(), Quality::Bad)]),
                false,
            )
            .await;
        evaluate_device(&storage, &mut states, &snapshot(&cache, "m4").await).await;
        assert!(
            storage
                .get_open_downtime_event("m4")
                .await
                .unwrap()
                .is_some()
        );
        cache
            .update("m4", HashMap::from([("rpm".into(), 5.0)]), false)
            .await;
        evaluate_device(&storage, &mut states, &snapshot(&cache, "m4").await).await;
        assert!(
            storage
                .get_open_downtime_event("m4")
                .await
                .unwrap()
                .is_none()
        );
    }
}
