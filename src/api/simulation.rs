use crate::api::*;
use crate::models::*;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

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

pub async fn toggle_simulation(
    state: State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SimulateRequest>,
) -> Result<StatusCode, StatusCode> {
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
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

pub async fn go_live(
    state: State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    // Create a pre-go-live snapshot
    let _ = state
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
        .await;

    // Toggle simulation off
    let updated = state
        .storage
        .set_device_simulation(&id, false, None, None)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if updated {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
