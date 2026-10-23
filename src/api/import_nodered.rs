use axum::Json;
use axum::extract::{Extension, Multipart, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::import_nodered::{converter, parser};

/// POST /api/v1/import/node-red
///
/// Accepts a multipart upload with a flows.json file, parses it, and returns
/// a conversion preview showing how Node-RED nodes map to Voltrus entities.
pub async fn import_node_red(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let mut json_content = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError(anyhow::anyhow!("Multipart error: {}", e)))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" || name == "flows" {
            json_content = Some(
                field
                    .text()
                    .await
                    .map_err(|e| AppError(anyhow::anyhow!("Failed to read file: {}", e)))?,
            );
            break;
        }
    }

    let json_str = json_content.ok_or_else(|| {
        AppError(anyhow::anyhow!(
            "No 'file' field found in upload. Send flows.json as multipart field named 'file'."
        ))
    })?;

    let parse_result =
        parser::parse_flows(&json_str).map_err(|e| AppError(anyhow::anyhow!("{}", e)))?;

    let conversion = converter::convert(&parse_result);

    let total_new_devices = conversion.modbus_devices.len() as u32;
    let current_device_count = state.storage.count_modbus_devices().await.unwrap_or(0);

    let warnings = conversion.warnings.clone();

    Ok((
        StatusCode::OK,
        Json(json!({
            "parse": {
                "total_nodes": parse_result.total_nodes,
                "convertible": parse_result.convertible,
                "manual_review": parse_result.manual_review,
                "ui_decoration": parse_result.ui_decoration,
                "unsupported": parse_result.unsupported,
            },
            "conversion": {
                "modbus_devices": conversion.modbus_devices.len(),
                "mqtt_sources": conversion.mqtt_sources.len(),
                "widgets": conversion.widgets.len(),
                "flow_nodes": conversion.flow_nodes.len(),
                "function_nodes": conversion.function_nodes.len(),
                "skipped_nodes": conversion.skipped_nodes.len(),
            },
            "details": {
                "modbus_devices": conversion.modbus_devices,
                "mqtt_sources": conversion.mqtt_sources,
                "widgets": conversion.widgets,
                "flow_nodes": conversion.flow_nodes,
                "function_nodes": conversion.function_nodes,
                "skipped_nodes": conversion.skipped_nodes,
            },
            "warnings": warnings,
            "license": {
                "current_devices": current_device_count,
                "new_devices": total_new_devices,
                "limit": null,
            },
        })),
    ))
}
