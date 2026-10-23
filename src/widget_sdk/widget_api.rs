use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};
use serde_json::json;

use crate::api::AppState;

/// GET /api/v1/widgets
pub async fn list_widgets(State(state): State<AppState>) -> impl IntoResponse {
    let widgets = state.widget_registry.list().await;
    Json(widgets)
}

/// GET /api/v1/widgets/{id}
pub async fn get_widget(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.widget_registry.get(&id).await {
        Some(info) => Json(info).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Widget not found"})),
        )
            .into_response(),
    }
}

/// GET /api/v1/widgets/{id}/asset/{filename}
pub async fn get_widget_asset(
    State(state): State<AppState>,
    Path((id, filename)): Path<(String, String)>,
) -> impl IntoResponse {
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return (StatusCode::BAD_REQUEST, "Invalid filename").into_response();
    }

    match state.widget_registry.asset_path(&id, &filename) {
        Some(path) => match tokio::fs::read(&path).await {
            Ok(data) => {
                let content_type = match path.extension().and_then(|e| e.to_str()) {
                    Some("js") => "application/javascript",
                    Some("css") => "text/css",
                    Some("json") => "application/json",
                    _ => "application/octet-stream",
                };
                (
                    StatusCode::OK,
                    [
                        (
                            header::CONTENT_TYPE,
                            header::HeaderValue::from_static(content_type),
                        ),
                        (
                            header::CACHE_CONTROL,
                            header::HeaderValue::from_static("public, max-age=3600"),
                        ),
                    ],
                    data,
                )
                    .into_response()
            }
            Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read asset").into_response(),
        },
        None => (StatusCode::NOT_FOUND, "Asset not found").into_response(),
    }
}

/// POST /api/v1/widgets/{id}/install
pub async fn install_widget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    multipart: Multipart,
) -> impl IntoResponse {
    let mut zip_data = Vec::new();

    let mut multipart = multipart;
    while let Ok(Some(field)) = multipart.next_field().await {
        if field.name() == Some("file") {
            zip_data = field.bytes().await.unwrap_or_default().to_vec();
        }
    }

    if zip_data.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "No file provided"})),
        )
            .into_response();
    }

    match state.widget_registry.install(&id, &zip_data).await {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "installed"}))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// DELETE /api/v1/widgets/{id}
pub async fn delete_widget(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.widget_registry.uninstall(&id).await {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "deleted"}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
