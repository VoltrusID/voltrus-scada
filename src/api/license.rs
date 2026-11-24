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
    // This route is public (the sign-in page reads the tier): never the key
    // itself, only a hint that tells keys apart.
    if let Some(key) = status.get("license_key").and_then(|k| k.as_str()) {
        status["license_key"] = crate::ai_copilot::key_store::masked_hint(key).into();
    }
    status["tier"] = serde_json::to_value(license_status.tier()).unwrap();
    status["support"] = serde_json::to_value(license_status.support()).unwrap();
    status["feature_flags"] = license_status.feature_flags();
    Json(status)
}
