use super::*;

// ── Object CRUD ──

/// GET /api/v1/bacnet/devices/{id}/objects
#[utoipa::path(
    get,
    path = "/api/v1/bacnet/devices/{id}/objects",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = Vec<serde_json::Value>),
        (status = 403),
    ),
    tag = "BACnet",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_objects(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    let objects = state.storage.list_bacnet_objects(id).await?;
    Ok(Json(objects))
}

/// POST /api/v1/bacnet/devices/{id}/objects
#[utoipa::path(
    post,
    path = "/api/v1/bacnet/devices/{id}/objects",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    request_body = CreateBacnetObjectRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "BACnet",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_object(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(device_id): Path<i64>,
    Json(req): Json<CreateBacnetObjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    // Validate object type
    let valid_types = [
        "analogInput",
        "analogOutput",
        "analogValue",
        "binaryInput",
        "binaryOutput",
        "binaryValue",
        "multiStateInput",
        "multiStateOutput",
        "multiStateValue",
    ];
    if !valid_types.contains(&req.object_type.as_str()) {
        return Err(AppError(anyhow::anyhow!(
            "Invalid object_type '{}'. Must be one of: {}",
            req.object_type,
            valid_types.join(", ")
        )));
    }

    // Load device name for data source key generation
    let devices = state.storage.list_bacnet_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let id = state
        .storage
        .create_bacnet_object(
            device_id,
            &device.name,
            &req.object_type,
            req.object_instance,
            req.property.as_deref().unwrap_or("presentValue"),
            &req.name,
            &req.display_name,
            req.cov_increment,
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError(anyhow::anyhow!("Object already exists on this device"))
            } else {
                AppError(e)
            }
        })?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_bacnet_object".to_string(),
        username: user.username.clone(),
        resource: format!("bacnet_object:{}", id),
        details: Some(format!(
            "device_id={}, object_type={}, object_instance={}",
            device_id, req.object_type, req.object_instance
        )),
    });

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// DELETE /api/v1/bacnet/devices/{id}/objects/{object_id}
#[utoipa::path(
    delete,
    path = "/api/v1/bacnet/devices/{id}/objects/{object_id}",
    params(
        ("id" = i64, Path, description = "Device ID"),
        ("object_id" = i64, Path, description = "Object ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
    ),
    tag = "BACnet",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_object(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_device_id, object_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    state.storage.delete_bacnet_object(object_id).await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_bacnet_object".to_string(),
        username: user.username.clone(),
        resource: format!("bacnet_object:{}", object_id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Browse ──

/// POST /api/v1/bacnet/devices/{id}/browse
#[utoipa::path(
    post,
    operation_id = "bacnet_browse_device",
    path = "/api/v1/bacnet/devices/{id}/browse",
    params(
        ("id" = i64, Path, description = "Device ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
        (status = 500),
    ),
    tag = "BACnet",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn browse_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError(anyhow::anyhow!("Admin access required")));
    }

    // Load device from DB
    let devices = state.storage.list_bacnet_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let objects = crate::bacnet::browse_objects(&device.ip, device.port, device.device_instance)
        .await
        .map_err(|e| AppError(anyhow::anyhow!("BACnet browse failed: {}", e)))?;

    let object_list: Vec<serde_json::Value> = objects
        .iter()
        .map(|obj| {
            json!({
                "object_type": obj.object_type,
                "object_instance": obj.object_instance,
                "object_name": obj.object_name,
            })
        })
        .collect();

    Ok(Json(json!({
        "objects": object_list,
        "count": object_list.len(),
    })))
}
