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
    let license_status = state.license_status.read().unwrap();
    let mut status = serde_json::to_value(&*license_status).unwrap();
    drop(license_status);
    let tier = state.tier();
    status["tier"] = serde_json::to_value(tier).unwrap();
    status["feature_flags"] = tier.to_feature_flags();
    Json(status)
}
