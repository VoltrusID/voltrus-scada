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

/// GET /api/v1/audit/verify — walk the audit log's hash chain and report the
/// first row that was altered, inserted out of chain or follows a deleted one.
pub async fn verify_audit_chain(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let report = state.storage.verify_audit_chain().await?;
    Ok(Json(json!({
        "ok": report.ok,
        "checked": report.checked,
        "unhashed_legacy": report.unhashed_legacy,
        "first_bad_id": report.first_bad_id,
        "dropped_since_start": state.audit_logger.dropped(),
    })))
}

/// Audit entry for every state-changing call on the routes it wraps, named
/// after the route (`PUT /api/v1/screens/{id}/layouts`) with the concrete path
/// as the resource and the answer's status, so refused attempts show too.
/// For handlers whose own entry would add nothing beyond who, what and when.
pub async fn audit_writes_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::Method;
    let method = req.method().clone();
    if matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let route = req
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| path.clone());
    let username = req
        .extensions()
        .get::<super::AuthUser>()
        .map(|u| u.username.clone())
        .unwrap_or_else(|| "unknown".to_string());
    let resp = next.run(req).await;
    state.audit_logger.log(crate::security::AuditEvent {
        action: format!("{method} {route}"),
        username,
        resource: path,
        details: Some(format!("status={}", resp.status().as_u16())),
    });
    resp
}
