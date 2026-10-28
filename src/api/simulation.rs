use crate::api::*;
use crate::models::*;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct SimulateRequest {
    pub simulation_mode: bool,
    pub sim_profile_id: Option<i64>,
    pub sim_config: Option<String>,
}

// ── Sim Profiles ──

pub async fn list_sim_profiles(
    state: State<AppState>,
) -> Result<Json<Vec<SimProfile>>, StatusCode> {
    state
        .storage
        .list_sim_profiles()
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn create_sim_profile(
    state: State<AppState>,
    Json(req): Json<CreateSimProfileRequest>,
) -> Result<(StatusCode, Json<SimProfile>), StatusCode> {
    let id = state
        .storage
        .create_sim_profile(&req.name, &req.data_type, &req.config)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let profile = state
        .storage
        .get_sim_profile(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(profile)))
}

pub async fn update_sim_profile(
    state: State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateSimProfileRequest>,
) -> Result<Json<SimProfile>, StatusCode> {
    state
        .storage
        .update_sim_profile(
            id,
            req.name.as_deref(),
            req.data_type.as_deref(),
            req.config.as_deref(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .storage
        .get_sim_profile(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn delete_sim_profile(
    state: State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    state
        .storage
        .delete_sim_profile(id)
        .await
        .map(|deleted| {
            if deleted {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::NOT_FOUND
            }
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

// ── Device Simulation ──

/// GET /api/v1/devices/{id}/simulate — the device's current simulation state.
pub async fn get_simulation(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let (simulation_mode, sim_profile_id, sim_config) = state
        .storage
        .get_device_simulation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(serde_json::json!({
        "simulation_mode": simulation_mode,
        "sim_profile_id": sim_profile_id,
        "sim_config": sim_config,
    })))
}

pub async fn toggle_simulation(
    state: State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SimulateRequest>,
) -> Result<StatusCode, StatusCode> {
    // Switching simulation on by hand supersedes any pending go-live.
    if req.simulation_mode {
        let _ = state.storage.delete_setting(&go_live_key(&id)).await;
    }
    let updated = state
        .storage
        .set_device_simulation(
            &id,
            req.simulation_mode,
            req.sim_profile_id,
            req.sim_config.as_deref(),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if updated {
        restart_device_poller(&state, &id).await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

// ── Go-live and rollback ──

const SETTING_GRACE_SECS: &str = "go_live_grace_secs";
const SETTING_AUTO_ROLLBACK: &str = "go_live_auto_rollback";
const DEFAULT_GRACE_SECS: i64 = 60;
const MIN_GRACE_SECS: i64 = 5;
const MAX_GRACE_SECS: i64 = 3600;
/// How long after going live the pre-go-live snapshot can still be restored in
/// one click. Restoring rewrites every versioned config table, so an old
/// snapshot would silently discard days of unrelated edits.
const ROLLBACK_WINDOW_SECS: i64 = 24 * 3600;

fn go_live_key(device_id: &str) -> String {
    format!("go_live:{device_id}")
}

/// Persisted at go-live; the grace window and auto-rollback choice are frozen
/// here so changing the settings later does not alter a go-live in flight.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoLiveRecord {
    snapshot_id: i64,
    went_live_at: i64,
    grace_secs: i64,
    auto_rollback: bool,
}

#[derive(Serialize, Deserialize)]
pub struct GoLiveSettings {
    pub grace_secs: i64,
    pub auto_rollback: bool,
}

async fn load_go_live_settings(storage: &crate::storage::Storage) -> GoLiveSettings {
    let rows = storage
        .get_settings(&[SETTING_GRACE_SECS, SETTING_AUTO_ROLLBACK])
        .await
        .unwrap_or_default();
    let get = |key: &str| rows.iter().find(|r| r.key == key).map(|r| r.value.as_str());
    GoLiveSettings {
        grace_secs: get(SETTING_GRACE_SECS)
            .and_then(|v| v.parse::<i64>().ok())
            .map_or(DEFAULT_GRACE_SECS, |v| v.clamp(MIN_GRACE_SECS, MAX_GRACE_SECS)),
        auto_rollback: get(SETTING_AUTO_ROLLBACK) == Some("true"),
    }
}

async fn load_record(
    storage: &crate::storage::Storage,
    device_id: &str,
) -> Result<Option<GoLiveRecord>, StatusCode> {
    let key = go_live_key(device_id);
    let rows = storage
        .get_settings(&[key.as_str()])
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let record: GoLiveRecord =
        serde_json::from_str(&row.value).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if crate::timeutil::now_secs() - record.went_live_at > ROLLBACK_WINDOW_SECS {
        return Ok(None);
    }
    Ok(Some(record))
}

/// Pollers read the simulation settings once at start, so a change only takes
/// effect after the device's poller is restarted.
async fn restart_device_poller(state: &AppState, device_id: &str) {
    #[cfg(feature = "modbus")]
    if let Ok(configs) = state.storage.get_all_device_configs().await
        && let Some(dc) = configs.iter().find(|c| c.id == device_id)
    {
        state.poller_manager.restart(dc.clone()).await;
    }
    #[cfg(not(feature = "modbus"))]
    let _ = (state, device_id);
}

/// GET /api/v1/settings/go-live
pub async fn get_go_live_settings(state: State<AppState>) -> Json<GoLiveSettings> {
    Json(load_go_live_settings(&state.storage).await)
}

/// PUT /api/v1/settings/go-live. `grace_secs` is clamped to 5..=3600.
/// `auto_rollback` is off unless explicitly enabled.
pub async fn update_go_live_settings(
    state: State<AppState>,
    Json(req): Json<GoLiveSettings>,
) -> Result<Json<GoLiveSettings>, StatusCode> {
    let grace = req.grace_secs.clamp(MIN_GRACE_SECS, MAX_GRACE_SECS);
    state
        .storage
        .upsert_setting(SETTING_GRACE_SECS, &grace.to_string())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state
        .storage
        .upsert_setting(SETTING_AUTO_ROLLBACK, &req.auto_rollback.to_string())
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(GoLiveSettings {
        grace_secs: grace,
        auto_rollback: req.auto_rollback,
    }))
}

/// POST /api/v1/devices/{id}/go-live. Snapshots the configuration, switches the
/// device to live mode and starts the connection grace window. The snapshot is
/// mandatory: without it there is nothing to roll back to, so a failed snapshot
/// aborts the go-live.
pub async fn go_live(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if state
        .storage
        .get_device_simulation(&id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .is_none()
    {
        return Err(StatusCode::NOT_FOUND);
    }

    let snapshot_id = state
        .storage
        .create_config_snapshot(
            Some(&format!("pre-go-live: {}", id)),
            Some(&format!(
                "Auto-snapshot before switching device {} to live mode",
                id
            )),
            "auto",
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let settings = load_go_live_settings(&state.storage).await;
    let record = GoLiveRecord {
        snapshot_id,
        went_live_at: crate::timeutil::now_secs(),
        grace_secs: settings.grace_secs,
        auto_rollback: settings.auto_rollback,
    };
    state
        .storage
        .upsert_setting(
            &go_live_key(&id),
            &serde_json::to_string(&record).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let updated = state
        .storage
        .set_device_simulation(&id, false, None, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !updated {
        let _ = state.storage.delete_setting(&go_live_key(&id)).await;
        return Err(StatusCode::NOT_FOUND);
    }
    restart_device_poller(&state, &id).await;

    if record.auto_rollback {
        spawn_auto_rollback(state.0.clone(), id.clone(), record.clone());
    }
    Ok(Json(serde_json::json!({
        "snapshot_id": record.snapshot_id,
        "grace_secs": record.grace_secs,
        "auto_rollback": record.auto_rollback,
    })))
}

/// After the grace window, roll back if the device never connected. Only runs
/// when auto-rollback was enabled at go-live; the record must still be the same
/// go-live (not rolled back or replaced meanwhile).
fn spawn_auto_rollback(state: AppState, device_id: String, record: GoLiveRecord) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(record.grace_secs as u64)).await;
        let current = match load_record(&state.storage, &device_id).await {
            Ok(Some(r)) if r.snapshot_id == record.snapshot_id => r,
            _ => return,
        };
        if state.cache.is_device_online(&device_id).await {
            return;
        }
        tracing::warn!(
            "Device {} did not connect within {}s of going live; rolling back to snapshot #{}",
            device_id,
            current.grace_secs,
            current.snapshot_id
        );
        if let Err(code) = perform_rollback(&state, &device_id, &current).await {
            tracing::error!("Automatic go-live rollback for {} failed: {}", device_id, code);
        }
    });
}

async fn perform_rollback(
    state: &AppState,
    device_id: &str,
    record: &GoLiveRecord,
) -> Result<(), StatusCode> {
    // Keep the live configuration recoverable, like a manual snapshot rollback.
    let _ = state
        .storage
        .create_config_snapshot(
            Some(&format!("pre-rollback: go-live {}", device_id)),
            Some(&format!(
                "Auto-snapshot before rolling device {} back to simulation",
                device_id
            )),
            "restore",
            None,
        )
        .await;
    let restored = state
        .storage
        .rollback_go_live(device_id, record.snapshot_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !restored {
        return Err(StatusCode::GONE);
    }
    restart_device_poller(state, device_id).await;
    Ok(())
}

/// POST /api/v1/devices/{id}/go-live/rollback. Restores the pre-go-live
/// snapshot and puts the device back in simulation mode in one transaction.
/// 404 when the device has no go-live to undo (never went live, already rolled
/// back, or older than 24 hours); 410 when its snapshot was deleted.
pub async fn rollback_go_live(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let record = load_record(&state.storage, &id)
        .await?
        .ok_or(StatusCode::NOT_FOUND)?;
    perform_rollback(&state, &id, &record).await?;
    Ok(Json(serde_json::json!({
        "snapshot_id": record.snapshot_id,
        "simulation_mode": true,
    })))
}

/// GET /api/v1/devices/{id}/go-live. Where the last go-live stands:
/// `verifying` (inside the grace window, not yet connected), `connected`, or
/// `failed` (grace window over and the device never came online).
pub async fn get_go_live_status(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let Some(record) = load_record(&state.storage, &id).await? else {
        return Ok(Json(serde_json::json!({"active": false})));
    };
    let online = state.cache.is_device_online(&id).await;
    let elapsed = crate::timeutil::now_secs() - record.went_live_at;
    let phase = if online {
        "connected"
    } else if elapsed < record.grace_secs {
        "verifying"
    } else {
        "failed"
    };
    Ok(Json(serde_json::json!({
        "active": true,
        "state": phase,
        "device_online": online,
        "snapshot_id": record.snapshot_id,
        "went_live_at": record.went_live_at,
        "grace_secs": record.grace_secs,
        "grace_remaining_secs": (record.grace_secs - elapsed).max(0),
        "auto_rollback": record.auto_rollback,
    })))
}
