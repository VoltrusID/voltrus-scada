use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::asset::models::CreateAssetRequest;

const VALID_ASSET_TYPES: &[&str] = &[
    "plc",
    "rtu",
    "hmi",
    "sensor",
    "actuator",
    "vfd",
    "switch",
    "gateway",
    "workstation",
    "printer",
    "robot",
    "other",
];

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListAssetsQuery {
    #[serde(rename = "asset_type")]
    pub type_: Option<String>,
    pub criticality: Option<String>,
    pub vendor: Option<String>,
    pub search: Option<String>,
}

// ── List Assets ──

#[utoipa::path(
    get,
    path = "/api/v1/assets",
    params(ListAssetsQuery),
    responses(
        (status = 200, body = Vec<crate::asset::models::Asset>)
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_assets(
    State(state): State<AppState>,
    Query(params): Query<ListAssetsQuery>,
) -> Result<impl IntoResponse, AppError> {
    let assets = state
        .storage
        .asset()
        .list_assets(
            params.type_.as_deref(),
            params.criticality.as_deref(),
            params.vendor.as_deref(),
            params.search.as_deref(),
        )
        .await?;
    Ok(Json(assets))
}

// ── Create Asset ──

#[utoipa::path(
    post,
    path = "/api/v1/assets",
    request_body = CreateAssetRequest,
    responses(
        (status = 201, body = crate::asset::models::Asset),
        (status = 400),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<CreateAssetRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError(anyhow::anyhow!("Asset name is required")));
    }

    let normalized_type = body.asset_type.trim().to_lowercase();
    if !VALID_ASSET_TYPES.contains(&normalized_type.as_str()) {
        return Err(AppError(anyhow::anyhow!(
            "Invalid asset type '{}'. Must be one of: {}",
            body.asset_type,
            VALID_ASSET_TYPES.join(", ")
        )));
    }

    let mut req = body;
    req.asset_type = normalized_type;

    let asset = state.storage.asset().create_asset(&req).await?;

    // Recompute risk score
    let asset_storage = state.storage.asset();
    let _ = crate::asset::risk::recompute_and_update(&asset_storage, &asset.id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_asset".to_string(),
        username: auth.username.clone(),
        resource: format!("asset:{}", asset.id),
        details: Some(format!("name={}, type={}", asset.name, asset.asset_type)),
    });

    Ok((axum::http::StatusCode::CREATED, Json(asset)))
}

// ── Get Asset ──

#[utoipa::path(
    get,
    path = "/api/v1/assets/{id}",
    params(("id" = String, Path, description = "Asset ID")),
    responses(
        (status = 200, body = crate::asset::models::Asset),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_asset(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let asset = state
        .storage
        .asset()
        .get_asset(&id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Asset not found")))?;
    Ok(Json(asset))
}

// ── Update Asset ──

#[utoipa::path(
    patch,
    path = "/api/v1/assets/{id}",
    params(("id" = String, Path, description = "Asset ID")),
    request_body = serde_json::Value,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, AppError> {
    // Verify asset exists
    state
        .storage
        .asset()
        .get_asset(&id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Asset not found")))?;

    state.storage.asset().update_asset(&id, &body).await?;

    // Recompute risk score
    let asset_storage = state.storage.asset();
    let _ = crate::asset::risk::recompute_and_update(&asset_storage, &id).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_asset".to_string(),
        username: auth.username.clone(),
        resource: format!("asset:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Delete Asset ──

#[utoipa::path(
    delete,
    path = "/api/v1/assets/{id}",
    params(("id" = String, Path, description = "Asset ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_asset(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Verify asset exists
    state
        .storage
        .asset()
        .get_asset(&id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Asset not found")))?;

    state.storage.asset().delete_asset(&id).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_asset".to_string(),
        username: auth.username.clone(),
        resource: format!("asset:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── IP Conflicts ──

#[utoipa::path(
    get,
    path = "/api/v1/assets/ip-conflicts",
    responses(
        (status = 200, body = Vec<crate::asset::models::IpConflict>)
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_ip_conflicts(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let conflicts = state.storage.asset().find_ip_conflicts().await?;

    let result: Vec<serde_json::Value> = conflicts
        .into_iter()
        .map(|(ip, assets)| {
            json!({
                "ip": ip,
                "assets": assets,
            })
        })
        .collect();

    Ok(Json(result))
}
