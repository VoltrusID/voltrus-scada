use super::*;

// ── Discover ──

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct DiscoverRequest {
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    5
}

/// POST /api/v1/bacnet/discover
#[utoipa::path(
    post,
    path = "/api/v1/bacnet/discover",
    request_body = Option<DiscoverRequest>,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
        (status = 500),
    ),
    tag = "BACnet",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn discover_network(
    State(_state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<Option<DiscoverRequest>>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let timeout_secs = body.map(|b| b.timeout_secs).unwrap_or(5);

    let discovered = crate::bacnet::discover_devices(timeout_secs)
        .await
        .map_err(|e| AppError(anyhow::anyhow!("BACnet discover failed: {}", e)))?;

    let device_list: Vec<serde_json::Value> = discovered
        .iter()
        .map(|d| {
            json!({
                "device_instance": d.device_instance,
                "ip": d.ip,
                "port": d.port,
                "vendor_name": d.vendor_name,
                "model_name": d.model_name,
                "firmware_version": d.firmware_version,
                "object_list_count": d.object_list_count,
            })
        })
        .collect();

    Ok(Json(json!({
        "devices": device_list,
        "count": device_list.len(),
    })))
}
