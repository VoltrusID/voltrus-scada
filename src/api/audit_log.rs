use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState};

#[derive(Debug, Deserialize)]
pub struct AuditQuery {
    pub search: Option<String>,
    pub action: Option<String>,
    pub username: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default = "default_sort")]
    pub sort: String,
    #[serde(default = "default_order")]
    pub order: String,
}

fn default_page() -> u32 {
    1
}
fn default_limit() -> u32 {
    50
}
fn default_sort() -> String {
    "created_at".to_string()
}
fn default_order() -> String {
    "desc".to_string()
}

/// GET /api/v1/audit
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Result<impl IntoResponse, AppError> {
    let limit = params.limit.min(1000);
    let (entries, total) = state
        .storage
        .list_audit_logs_filtered(
            params.search.as_deref(),
            params.action.as_deref(),
            params.username.as_deref(),
            params.from,
            params.to,
            params.page,
            limit,
            &params.sort,
            &params.order,
        )
        .await?;

    let total_pages = ((total as f64) / (limit as f64)).ceil() as u32;

    Ok(Json(json!({
        "entries": entries,
        "total": total,
        "page": params.page,
        "limit": limit,
        "total_pages": total_pages,
    })))
}

/// GET /api/v1/audit/export
pub async fn export_audit_csv(
    State(state): State<AppState>,
    Query(params): Query<AuditQuery>,
) -> Result<impl IntoResponse, AppError> {
    let csv = state
        .storage
        .export_audit_logs_csv(
            params.search.as_deref(),
            params.action.as_deref(),
            params.username.as_deref(),
            params.from,
            params.to,
        )
        .await?;

    let filename = format!(
        "audit-log-{}.csv",
        time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Iso8601::DATE)
            .unwrap_or_else(|_| "unknown".to_string())
    );

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/csv")),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename)).unwrap(),
            ),
        ],
        csv,
    ))
}
