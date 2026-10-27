use axum::Json;
use axum::extract::{Extension, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DeleteTelemetryQuery {
    /// The device the readings belong to (CSV/Excel imports without a device
    /// column write `default`).
    pub device_id: String,
    /// Only this tag's readings; omitted: every tag of the device.
    pub metric: Option<String>,
    /// First second to delete (Unix seconds, inclusive).
    pub from: i64,
    /// Last second to delete (Unix seconds, inclusive).
    pub to: i64,
}

/// DELETE /api/v1/telemetry?device_id=&metric=&from=&to=
///
/// Deletes stored readings in a time range, e.g. a CSV/Excel import that was
/// wrong. `device_id`, `from` and `to` are required so a bare call cannot
/// empty the store.
#[utoipa::path(
    delete,
    path = "/api/v1/telemetry",
    params(DeleteTelemetryQuery),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Data",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_telemetry(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<DeleteTelemetryQuery>,
) -> Result<impl IntoResponse, AppError> {
    if q.device_id.is_empty() {
        return Err(AppError::bad_request("device_id must not be empty"));
    }
    if q.from > q.to {
        return Err(AppError::bad_request("from must not be after to"));
    }
    let metric = q.metric.as_deref().filter(|m| !m.is_empty());

    let (raw, hourly, daily) = state
        .storage
        .delete_telemetry_range(&q.device_id, metric, q.from, q.to)
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "telemetry_delete".to_string(),
        username: auth.username.clone(),
        resource: format!("telemetry:{}", q.device_id),
        details: Some(format!(
            "metric={}, from={}, to={}, raw={}, hourly={}, daily={}",
            metric.unwrap_or("*"),
            q.from,
            q.to,
            raw,
            hourly,
            daily
        )),
    });

    Ok(Json(json!({
        "deleted": {"raw": raw, "hourly": hourly, "daily": daily},
    })))
}
