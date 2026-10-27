use axum::{Json, extract::State};

use crate::api::AppState;

#[utoipa::path(
    get,
    path = "/api/license/status",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "License",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_license_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let license_status = state
        .license_status
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let mut status = serde_json::to_value(&*license_status).unwrap();
    status["tier"] = serde_json::to_value(license_status.tier()).unwrap();
    status["support"] = serde_json::to_value(license_status.support()).unwrap();
    status["feature_flags"] = license_status.feature_flags();
    Json(status)
}
