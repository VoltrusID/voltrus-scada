use axum::Json;
use axum::extract::{Extension, State};
use axum::response::IntoResponse;

use crate::api::{AppError, AppState, AuthUser};
use crate::wizard::WizardConfig;
use crate::wizard::feed::feed_summaries;
use crate::wizard::feed::get_feed_template;
use crate::wizard::health::run_health_check;

/// POST /api/v1/wizard/create — Run the project creation wizard
pub async fn wizard_create(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    Json(config): Json<WizardConfig>,
) -> Result<impl IntoResponse, AppError> {
    let result = crate::wizard::execute_wizard(&state.storage, config).await?;
    Ok(Json(result))
}

/// GET /api/v1/wizard/health-check — Run a health check on the current project
pub async fn wizard_health_check(
    State(state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let report = run_health_check(&state.storage).await?;
    Ok(Json(report))
}

/// GET /api/v1/wizard/feed-templates — List all FEED templates
pub async fn list_feed_templates(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    Ok(Json(serde_json::json!({
        "templates": feed_summaries()
    })))
}

/// GET /api/v1/wizard/feed-templates/:id — Get a single FEED template
pub async fn get_feed_template_handler(
    State(_state): State<AppState>,
    Extension(_auth): Extension<AuthUser>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    match get_feed_template(&id) {
        Some(template) => Ok(Json(template)),
        None => Err(AppError(anyhow::anyhow!(
            "FEED template '{}' not found",
            id
        ))),
    }
}
