use axum::extract::{Extension, Json, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};
use crate::models::*;

// ── Query params ──

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListAnnotationsQuery {
    pub tag_id: Option<String>,
    pub from: Option<i64>,
    pub to: Option<i64>,
}

// ── Handlers ──

#[utoipa::path(
    post,
    path = "/api/v1/annotations",
    request_body = crate::models::CreateAnnotationRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 500),
    ),
    tag = "Annotations",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_annotation(
    State(state): State<AppState>,
    Extension(auth_user): Extension<AuthUser>,
    Json(req): Json<CreateAnnotationRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Resolve author user ID from authenticated username
    let author = state
        .storage
        .get_user_by_username(&auth_user.username)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Authenticated user not found"))?;
    let author_id = author.id;

    // Create the annotation
    let annotation_id = state.storage.create_annotation(&req, author_id).await?;

    // Parse @mentions and create notifications for each mentioned user
    let mentions = parse_mentions(&req.annotation_text);
    for username in &mentions {
        if let Ok(Some(mentioned_user)) = state.storage.get_user_by_username(username).await {
            // Don't notify yourself
            if mentioned_user.id != author_id {
                let _ = state
                    .storage
                    .create_notification(&CreateNotificationRequest {
                        user_id: mentioned_user.id,
                        from_user_id: author_id,
                        notification_type: "mention".to_string(),
                        message: format!(
                            "You were mentioned in an annotation on tag {}",
                            req.tag_id
                        ),
                        reference_id: Some(annotation_id),
                    })
                    .await;
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(json!({ "id": annotation_id, "ok": true })),
    ))
}

#[utoipa::path(
    get,
    path = "/api/v1/annotations",
    params(ListAnnotationsQuery),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    tag = "Annotations",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_annotations(
    State(state): State<AppState>,
    Query(params): Query<ListAnnotationsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let annotations = state
        .storage
        .list_annotations(params.tag_id.as_deref(), params.from, params.to)
        .await?;

    Ok(Json(json!({
        "annotations": annotations,
        "count": annotations.len(),
    })))
}

#[utoipa::path(
    get,
    path = "/api/v1/annotations/{id}",
    params(
        ("id" = i64, Path, description = "Annotation ID")
    ),
    responses(
        (status = 200, body = crate::models::Annotation),
        (status = 404),
    ),
    tag = "Annotations",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_annotation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let annotation = state
        .storage
        .get_annotation(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Annotation not found"))?;

    Ok(Json(json!(annotation)))
}

#[utoipa::path(
    put,
    path = "/api/v1/annotations/{id}",
    params(
        ("id" = i64, Path, description = "Annotation ID")
    ),
    request_body = crate::models::UpdateAnnotationRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    tag = "Annotations",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_annotation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateAnnotationRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Verify annotation exists
    state
        .storage
        .get_annotation(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Annotation not found"))?;

    state.storage.update_annotation(id, &req).await?;

    Ok(Json(json!({ "ok": true })))
}

#[utoipa::path(
    delete,
    path = "/api/v1/annotations/{id}",
    params(
        ("id" = i64, Path, description = "Annotation ID")
    ),
    responses(
        (status = 204),
        (status = 404),
    ),
    tag = "Annotations",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_annotation(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    // Verify annotation exists
    state
        .storage
        .get_annotation(id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Annotation not found"))?;

    state.storage.delete_annotation(id).await?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_mentions_basic() {
        let mentions = parse_mentions("Hello @alice and @bob check this");
        assert_eq!(mentions, vec!["alice", "bob"]);
    }

    #[test]
    fn test_parse_mentions_dedup() {
        let mentions = parse_mentions("@alice @alice @bob @alice");
        assert_eq!(mentions, vec!["alice", "bob"]);
    }

    #[test]
    fn test_parse_mentions_none() {
        let mentions = parse_mentions("No mentions here");
        assert!(mentions.is_empty());
    }

    #[test]
    fn test_parse_mentions_underscore() {
        let mentions = parse_mentions("@user_1 and @user_2");
        assert_eq!(mentions, vec!["user_1", "user_2"]);
    }

    #[test]
    fn test_parse_mentions_not_in_word() {
        // The @ must start a new mention — email-like strings should NOT
        // capture the part after @ when preceded by a word char.
        let mentions = parse_mentions("email@test.com @alice");
        assert_eq!(mentions, vec!["alice"]);
    }
}
