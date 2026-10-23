use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode as HttpStatus,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::json;

use crate::api::AppState;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct OeeMetricsQuery {
    pub device_id: String,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DowntimeQuery {
    pub device_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

/// GET /api/v1/oee/metrics?device_id=&from=&to= -- OEE breakdown
#[utoipa::path(
    get,
    path = "/api/v1/oee/metrics",
    params(OeeMetricsQuery),
    responses(
        (status = 200, body = crate::models::OeeMetrics),
        (status = 400),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_oee_metrics(
    State(state): State<AppState>,
    Query(q): Query<OeeMetricsQuery>,
) -> impl IntoResponse {
    let now = crate::timeutil::now_secs();
    let to = q.to.unwrap_or(now);
    let from = q.from.unwrap_or(to - 8 * 3600); // default 8-hour window

    if from >= to {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "from must be less than to"})),
        )
            .into_response();
    }

    match state.storage.calculate_oee(&q.device_id, from, to).await {
        Ok(metrics) => (HttpStatus::OK, Json(json!(metrics))).into_response(),
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to calculate OEE: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/v1/oee/downtime?device_id=&from=&to= -- list downtime events
#[utoipa::path(
    get,
    path = "/api/v1/oee/downtime",
    params(DowntimeQuery),
    responses(
        (status = 200, body = Vec<crate::models::DowntimeEvent>),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_downtime(
    State(state): State<AppState>,
    Query(q): Query<DowntimeQuery>,
) -> impl IntoResponse {
    match state
        .storage
        .list_downtime_events(q.device_id.as_deref(), q.from, q.to)
        .await
    {
        Ok(events) => (HttpStatus::OK, Json(json!(events))).into_response(),
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to list downtime events: {}", e)})),
        )
            .into_response(),
    }
}

/// POST /api/v1/oee/downtime -- manually create downtime event
#[utoipa::path(
    post,
    path = "/api/v1/oee/downtime",
    request_body = crate::models::CreateDowntimeEventRequest,
    responses(
        (status = 201, body = crate::models::DowntimeEvent),
        (status = 400),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_downtime(
    State(state): State<AppState>,
    Json(req): Json<crate::models::CreateDowntimeEventRequest>,
) -> impl IntoResponse {
    let device_id = req.device_id.trim();
    if device_id.is_empty() {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "device_id is required"})),
        )
            .into_response();
    }

    let state_str = req.state.trim().to_lowercase();
    if !matches!(
        state_str.as_str(),
        "idle" | "faulted" | "setup" | "maintenance" | "running"
    ) {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "state must be one of: idle, faulted, setup, maintenance, running"})),
        )
            .into_response();
    }

    match state
        .storage
        .create_downtime_event(device_id, &state_str)
        .await
    {
        Ok(_id) => match state.storage.get_open_downtime_event(device_id).await {
            Ok(Some(event)) => (HttpStatus::CREATED, Json(json!(event))).into_response(),
            Ok(None) => (
                HttpStatus::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Downtime event created but could not be retrieved"})),
            )
                .into_response(),
            Err(e) => (
                HttpStatus::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to retrieve downtime event: {}", e)})),
            )
                .into_response(),
        },
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create downtime event: {}", e)})),
        )
            .into_response(),
    }
}

/// PUT /api/v1/oee/downtime/{id}/close -- close event with reason
#[utoipa::path(
    put,
    path = "/api/v1/oee/downtime/{id}/close",
    params(
        ("id" = i64, Path, description = "Downtime event ID")
    ),
    request_body = crate::models::CloseDowntimeEventRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn close_downtime(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<crate::models::CloseDowntimeEventRequest>,
) -> impl IntoResponse {
    match state
        .storage
        .close_downtime_event(id, req.reason.as_deref())
        .await
    {
        Ok(()) => (HttpStatus::OK, Json(json!({"ok": true, "id": id}))).into_response(),
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to close downtime event: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/v1/oee/shifts -- list shifts
#[utoipa::path(
    get,
    path = "/api/v1/oee/shifts",
    responses(
        (status = 200, body = Vec<crate::models::ShiftConfig>),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_shifts(State(state): State<AppState>) -> impl IntoResponse {
    match state.storage.list_shifts().await {
        Ok(shifts) => (HttpStatus::OK, Json(json!(shifts))).into_response(),
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to list shifts: {}", e)})),
        )
            .into_response(),
    }
}

/// POST /api/v1/oee/shifts -- create shift
#[utoipa::path(
    post,
    path = "/api/v1/oee/shifts",
    request_body = crate::models::CreateShiftRequest,
    responses(
        (status = 201, body = crate::models::ShiftConfig),
        (status = 400),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_shift(
    State(state): State<AppState>,
    Json(req): Json<crate::models::CreateShiftRequest>,
) -> impl IntoResponse {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "Shift name is required"})),
        )
            .into_response();
    }

    if req.start_hour > 23 {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "start_hour must be 0-23"})),
        )
            .into_response();
    }

    if req.start_minute > 59 {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "start_minute must be 0-59"})),
        )
            .into_response();
    }

    if req.duration_hours == 0 || req.duration_hours > 24 {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "duration_hours must be 1-24"})),
        )
            .into_response();
    }

    if req.days_mask > 127 {
        return (
            HttpStatus::BAD_REQUEST,
            Json(json!({"error": "days_mask must be 0-127 (bitfield: bit0=Mon..bit6=Sun)"})),
        )
            .into_response();
    }

    match state
        .storage
        .create_shift(
            &name,
            req.start_hour,
            req.start_minute,
            req.duration_hours,
            req.days_mask,
        )
        .await
    {
        Ok(id) => match state.storage.list_shifts().await {
            Ok(shifts) => {
                let created = shifts.into_iter().find(|s| s.id == id);
                match created {
                    Some(shift) => (HttpStatus::CREATED, Json(json!(shift))).into_response(),
                    None => (
                        HttpStatus::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": "Shift created but could not be retrieved"})),
                    )
                        .into_response(),
                }
            }
            Err(e) => (
                HttpStatus::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to retrieve shifts: {}", e)})),
            )
                .into_response(),
        },
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create shift: {}", e)})),
        )
            .into_response(),
    }
}

/// DELETE /api/v1/oee/shifts/{id} -- delete shift
#[utoipa::path(
    delete,
    path = "/api/v1/oee/shifts/{id}",
    params(
        ("id" = i64, Path, description = "Shift ID")
    ),
    responses(
        (status = 204),
        (status = 403),
    ),
    tag = "OEE",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_shift(State(state): State<AppState>, Path(id): Path<i64>) -> impl IntoResponse {
    match state.storage.delete_shift(id).await {
        Ok(()) => (HttpStatus::NO_CONTENT, "").into_response(),
        Err(e) => (
            HttpStatus::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to delete shift: {}", e)})),
        )
            .into_response(),
    }
}
