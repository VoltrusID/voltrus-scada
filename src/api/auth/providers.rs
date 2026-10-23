use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState};

/// GET /api/v1/auth/providers -- lists the enabled sign-in providers.
///
/// The open core only has local accounts. An add-on edition can contribute
/// additional providers (SSO) through [`crate::edition::Extension::auth_providers`].
#[utoipa::path(
    get,
    path = "/api/v1/auth/providers",
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Auth",
)]
pub async fn list_auth_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let mut providers = vec![json!({
        "provider": "local",
        "enabled": true,
        "label": "Local Authentication",
    })];
    providers.extend(state.ext.auth_providers());
    Ok(Json(json!({"providers": providers})))
}
