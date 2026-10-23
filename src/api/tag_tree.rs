use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState};
use crate::storage::Storage;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TreeQuery {
    pub search: Option<String>,
}

/// Response envelope for the hierarchical UNS tag tree.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TagTreeResponse {
    pub tree: Vec<serde_json::Value>,
}

/// Acknowledgement for a tag path update.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct TagPathUpdatedResponse {
    pub ok: bool,
}

/// GET /api/v1/tags/tree — hierarchical UNS tree view of all tags
#[utoipa::path(
    get,
    path = "/api/v1/tags/tree",
    tag = "Tags",
    params(TreeQuery),
    responses(
        (status = 200, description = "Hierarchical UNS tag tree", body = TagTreeResponse),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_tag_tree(
    State(state): State<AppState>,
    Query(params): Query<TreeQuery>,
) -> Result<impl IntoResponse, AppError> {
    let tags = state.storage.list_data_sources_for_tree().await?;
    let tree = Storage::build_tag_tree(&tags, params.search.as_deref());
    Ok(Json(json!({ "tree": tree })))
}

/// PUT /api/v1/tags/{id}/path — update a tag's UNS path
#[utoipa::path(
    put,
    path = "/api/v1/tags/{id}/path",
    tag = "Tags",
    params(("id" = i64, Path, description = "Data source (tag) ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, description = "Path updated", body = TagPathUpdatedResponse),
        (status = 400, description = "Invalid path"),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_tag_path(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<serde_json::Value>,
) -> Result<axum::response::Response, AppError> {
    let path = body.get("path").and_then(|v| v.as_str());

    if let Some(p) = path {
        if p.is_empty() {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "path must not be empty string; use null to clear"})),
            )
                .into_response());
        }
        for segment in p.split('/') {
            if segment.trim().is_empty() {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "path segments must not be empty (e.g., 'Site/Area/Unit')"})),
                )
                    .into_response());
            }
        }
    }

    state.storage.update_data_source_path(id, path).await?;
    Ok(Json(json!({"ok": true})).into_response())
}
