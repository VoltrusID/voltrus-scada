use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use opcua::client::{ClientBuilder, IdentityToken, UARequest, services::Browse};
use opcua::types::{
    BrowseDescription, BrowseDescriptionResultMask, BrowseDirection, NodeClass, NodeClassMask,
    NodeId, UserTokenPolicy,
};

use crate::api::{AppError, AppState, AuthUser};
use crate::models::*;
use crate::opcua::{security_mode_to_opcua, security_policy_to_str};

// ── Device CRUD ──

/// GET /api/v1/opcua/devices
#[utoipa::path(
    get,
    operation_id = "opcua_list_devices",
    path = "/api/v1/opcua/devices",
    responses(
        (status = 200, body = Vec<serde_json::Value>),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_devices(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let devices = state.storage.list_opcua_devices().await?;
    let mut result = Vec::new();
    for device in &devices {
        let nodes = state.storage.list_opcua_nodes(device.id).await?;
        result.push(json!({
            "id": device.id,
            "name": device.name,
            "url": device.url,
            "security_policy": device.security_policy,
            "security_mode": device.security_mode,
            "username": device.username,
            "interval_ms": device.interval_ms,
            "enabled": device.enabled,
            "created_at": device.created_at,
            "node_count": nodes.len(),
        }));
    }
    Ok(Json(result))
}

/// POST /api/v1/opcua/devices
#[utoipa::path(
    post,
    operation_id = "opcua_create_device",
    path = "/api/v1/opcua/devices",
    request_body = CreateOpcUaDeviceRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateOpcUaDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device name is required")));
    }
    if req.url.is_empty() {
        return Err(AppError(anyhow::anyhow!("Device URL is required")));
    }
    if !req.url.starts_with("opc.tcp://") {
        return Err(AppError(anyhow::anyhow!(
            "Device URL must start with opc.tcp://"
        )));
    }

    super::check_poll_interval(req.interval_ms)?;

    let id = state
        .storage
        .create_opcua_device(
            &req.name,
            &req.url,
            &req.security_policy,
            &req.security_mode,
            req.username.as_deref(),
            req.password.as_deref(),
            req.interval_ms,
            req.enabled,
        )
        .await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_created(
        &state.storage.asset(),
        "opcua",
        &id.to_string(),
        &req.name,
        None,
        "gateway",
    )
    .await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: Some(format!("name={}, url={}", req.name, req.url)),
    });

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// PUT /api/v1/opcua/devices/{id}
#[utoipa::path(
    put,
    operation_id = "opcua_update_device",
    path = "/api/v1/opcua/devices/{id}",
    params(("id" = i64, Path, description = "Device ID")),
    request_body = UpdateOpcUaDeviceRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateOpcUaDeviceRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if let Some(ref url) = req.url
        && !url.is_empty()
        && !url.starts_with("opc.tcp://")
    {
        return Err(AppError(anyhow::anyhow!(
            "Device URL must start with opc.tcp://"
        )));
    }

    if let Some(interval_ms) = req.interval_ms {
        super::check_poll_interval(interval_ms)?;
    }

    state
        .storage
        .update_opcua_device(
            id,
            req.name.as_deref(),
            req.url.as_deref(),
            req.security_policy.as_ref(),
            req.security_mode.as_ref(),
            req.username.as_ref().map(|o| o.as_deref()),
            req.password.as_ref().map(|o| o.as_deref()),
            req.interval_ms,
            req.enabled,
        )
        .await?;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/v1/opcua/devices/{id}
#[utoipa::path(
    delete,
    operation_id = "opcua_delete_device",
    path = "/api/v1/opcua/devices/{id}",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Stop poller if running
    let device_key = format!("opcua_{}", id);
    state.poller_manager.stop(&device_key).await;
    state.cache.remove(&device_key).await;

    state.storage.delete_opcua_device(id).await?;

    // Sync to asset inventory
    crate::asset::sync::on_device_deleted(&state.storage.asset(), "opcua", &id.to_string()).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

// ── Node CRUD ──

/// GET /api/v1/opcua/devices/{id}/nodes
#[utoipa::path(
    get,
    path = "/api/v1/opcua/devices/{id}/nodes",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = Vec<OpcUaNode>),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_nodes(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let nodes = state.storage.list_opcua_nodes(id).await?;
    Ok(Json(nodes))
}

/// POST /api/v1/opcua/devices/{id}/nodes
#[utoipa::path(
    post,
    path = "/api/v1/opcua/devices/{id}/nodes",
    params(("id" = i64, Path, description = "Device ID")),
    request_body = CreateOpcUaNodeRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn add_node(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(device_id): Path<i64>,
    Json(req): Json<CreateOpcUaNodeRequest>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    if req.node_id.is_empty() {
        return Err(AppError(anyhow::anyhow!("Node ID is required")));
    }
    if req.name.is_empty() {
        return Err(AppError(anyhow::anyhow!("Node name is required")));
    }

    // Look up device name for data source key generation
    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == device_id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    let id = state
        .storage
        .create_opcua_node(
            device_id,
            &device.name,
            &req.node_id,
            &req.name,
            &req.display_name,
            &req.data_type,
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE constraint failed") {
                AppError(anyhow::anyhow!("Node already exists on this device"))
            } else {
                AppError(e)
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "ok": true,
            "id": id,
        })),
    ))
}

/// DELETE /api/v1/opcua/devices/{id}/nodes/{node_id}
#[utoipa::path(
    delete,
    path = "/api/v1/opcua/devices/{id}/nodes/{node_id}",
    params(
        ("id" = i64, Path, description = "Device ID"),
        ("node_id" = i64, Path, description = "Node ID"),
    ),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn remove_node(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((_device_id, node_id)): Path<(i64, i64)>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    state.storage.delete_opcua_node(node_id).await?;

    Ok(Json(json!({"ok": true})))
}

// ── Browse ──

/// POST /api/v1/opcua/devices/{id}/browse
///
/// Connects to the OPC-UA server using the device's stored config and returns
/// available variable nodes from the ObjectsFolder.
/// POST /api/v1/opcua/devices/{id}/browse
#[utoipa::path(
    post,
    operation_id = "opcua_browse_device",
    path = "/api/v1/opcua/devices/{id}/browse",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn browse_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Load device from DB
    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    // Connect to OPC-UA server (temporary connection for browsing)
    let mut client = ClientBuilder::new()
        .application_name("Voltrus Browse")
        .application_uri("urn:voltrus-lite")
        .trust_server_certs(true)
        .create_sample_keypair(true)
        .client()
        .map_err(|e| AppError(anyhow::anyhow!("Client build errors: {:?}", e)))?;

    let security_policy_str = security_policy_to_str(&device.security_policy);
    let message_security_mode = security_mode_to_opcua(&device.security_mode);

    let user_token_policy = match (&device.username, &device.password) {
        (Some(_), Some(_)) => UserTokenPolicy {
            policy_id: opcua::types::UAString::from("voltrus-credential"),
            token_type: opcua::types::UserTokenType::UserName,
            issued_token_type: opcua::types::UAString::null(),
            issuer_endpoint_url: opcua::types::UAString::null(),
            security_policy_uri: opcua::types::UAString::null(),
        },
        _ => UserTokenPolicy::anonymous(),
    };

    let endpoint = opcua::types::EndpointDescription::from((
        device.url.as_str(),
        security_policy_str,
        message_security_mode,
        user_token_policy,
    ));

    let identity_token = match (&device.username, &device.password) {
        (Some(user), Some(pass)) => IdentityToken::new_user_name(user.clone(), pass.clone()),
        _ => IdentityToken::Anonymous,
    };

    let (session, event_loop) = client
        .connect_to_matching_endpoint(endpoint, identity_token)
        .await
        .map_err(|e| AppError(anyhow::anyhow!("OPC-UA connection failed: {:?}", e)))?;

    let mut event_handle = event_loop.spawn();

    // Wait for connection using the same pattern as opcua.rs
    let connection_result: Result<(), AppError> = tokio::select! {
        connected = session.wait_for_connection() => {
            if connected {
                Ok(())
            } else {
                Err(AppError(anyhow::anyhow!("OPC-UA session failed to connect")))
            }
        }
        event_result = &mut event_handle => {
            match event_result {
                Ok(code) => Err(AppError(anyhow::anyhow!(
                    "OPC-UA event loop ended with status {}", code
                ))),
                Err(e) => Err(AppError(anyhow::anyhow!(
                    "OPC-UA event loop panicked: {}", e
                ))),
            }
        }
    };
    connection_result?;

    // Browse the ObjectsFolder using the Browse builder
    let objects_folder = NodeId::objects_folder_id();

    let browse_desc = BrowseDescription {
        node_id: objects_folder,
        browse_direction: BrowseDirection::Forward,
        reference_type_id: NodeId::null(),
        include_subtypes: true,
        node_class_mask: NodeClassMask::OBJECT.bits() | NodeClassMask::VARIABLE.bits(),
        result_mask: BrowseDescriptionResultMask::RESULT_MASK_NODE_CLASS.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_BROWSE_NAME.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_DISPLAY_NAME.bits()
            | BrowseDescriptionResultMask::RESULT_MASK_TYPE_DEFINITION.bits(),
    };

    let browse_response = Browse::new(&session)
        .browse_node(browse_desc)
        .send(session.channel())
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Browse request failed: {:?}", e)))?;

    let mut nodes = Vec::new();
    if let Some(results) = browse_response.results {
        for result in &results {
            if !result.status_code.is_good() {
                continue;
            }
            if let Some(refs) = &result.references {
                for reference in refs {
                    // Include Variable nodes and Object nodes (objects may contain sub-variables)
                    if reference.node_class == NodeClass::Variable
                        || reference.node_class == NodeClass::Object
                    {
                        let node_id_str = reference.node_id.to_string();
                        let browse_name = reference.browse_name.to_string();
                        let display_name = reference.display_name.to_string();
                        let node_class_str = match reference.node_class {
                            NodeClass::Variable => "Variable",
                            NodeClass::Object => "Object",
                            _ => "Unknown",
                        };

                        nodes.push(json!({
                            "node_id": node_id_str,
                            "browse_name": browse_name,
                            "display_name": display_name,
                            "node_class": node_class_str,
                        }));
                    }
                }
            }
        }
    }

    // Disconnect gracefully
    drop(session);
    drop(event_handle);

    Ok(Json(json!({
        "nodes": nodes,
        "count": nodes.len(),
    })))
}

// ── Start / Stop ──

/// POST /api/v1/opcua/devices/{id}/start
///
/// Starts the OPC-UA poller for the device. Converts the DB-stored device + nodes
/// into an OpcUaDeviceConfig and starts the poller.
/// POST /api/v1/opcua/devices/{id}/start
#[utoipa::path(
    post,
    operation_id = "opcua_start_device",
    path = "/api/v1/opcua/devices/{id}/start",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn start_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    // Load device from DB
    let devices = state.storage.list_opcua_devices().await?;
    let device = devices
        .iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError(anyhow::anyhow!("Device not found")))?;

    if !device.enabled {
        return Err(AppError(anyhow::anyhow!(
            "Device is disabled. Enable it first before starting."
        )));
    }

    // Load nodes for device
    let nodes = state.storage.list_opcua_nodes(device.id).await?;

    if nodes.is_empty() {
        return Err(AppError(anyhow::anyhow!(
            "Device has no nodes configured. Add nodes before starting."
        )));
    }

    // Convert to runtime config
    let device_key = format!("opcua_{}", device.id);
    let config = OpcUaDeviceConfig {
        id: device_key.clone(),
        url: device.url.clone(),
        security_policy: device.security_policy.clone(),
        security_mode: device.security_mode.clone(),
        username: device.username.clone(),
        password: device.password.clone(),
        interval_ms: device.interval_ms,
        nodes: nodes
            .iter()
            .map(|n| OpcUaNodeConfig {
                node_id: n.node_id.clone(),
                name: n.name.clone(),
                display_name: n.display_name.clone(),
                data_type: n.data_type.clone(),
            })
            .collect(),
    };

    state.poller_manager.start_opcua(config).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "start_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/opcua/devices/{id}/stop
#[utoipa::path(
    post,
    operation_id = "opcua_stop_device",
    path = "/api/v1/opcua/devices/{id}/stop",
    params(("id" = i64, Path, description = "Device ID")),
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    tag = "Devices",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn stop_device(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    if user.role != "admin" {
        return Err(AppError::forbidden("Admin access required"));
    }

    let device_key = format!("opcua_{}", id);
    state.poller_manager.stop(&device_key).await;

    state.audit_logger.log(crate::security::AuditEvent {
        action: "stop_opcua_device".to_string(),
        username: user.username.clone(),
        resource: format!("opcua_device:{}", id),
        details: None,
    });

    Ok(Json(json!({"ok": true})))
}
