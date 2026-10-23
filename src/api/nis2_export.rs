use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;

use crate::api::{AppError, AppState};

/// NIS2 compliance CSV export headers.
const NIS2_HEADERS: &str = "name,type,vendor,model,serial_number,firmware,latest_firmware,firmware_status,last_maintenance,risk_score,criticality,location,ip_address";

// ── NIS2 Export ──

#[utoipa::path(
    get,
    path = "/api/v1/assets/nis2-export",
    responses(
        (status = 200, description = "NIS2 compliance CSV file download")
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn nis2_export(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let assets = state
        .storage
        .asset()
        .list_assets(None, None, None, None)
        .await?;

    let mut csv = String::from(NIS2_HEADERS);
    csv.push('\n');

    for asset in &assets {
        // Look up latest firmware from catalog
        let latest_firmware = if let (Some(vendor), Some(model)) = (&asset.vendor, &asset.model) {
            state
                .storage
                .asset()
                .get_latest_firmware(vendor, model)
                .await
                .unwrap_or(None)
        } else {
            None
        };

        let firmware_status = match (&asset.firmware_version, &latest_firmware) {
            (Some(current), Some(latest)) if current == latest => "up-to-date".to_string(),
            (Some(_), Some(_)) => "outdated".to_string(),
            (None, _) => "unknown".to_string(),
            _ => "unknown".to_string(),
        };

        let risk_score = asset
            .risk_score
            .map(|s| s.to_string())
            .unwrap_or_else(|| "0".to_string());

        csv.push_str(&format!(
            "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
            escape_csv(&asset.name),
            escape_csv(&asset.asset_type),
            escape_csv(&asset.vendor.clone().unwrap_or_default()),
            escape_csv(&asset.model.clone().unwrap_or_default()),
            escape_csv(&asset.serial_number.clone().unwrap_or_default()),
            escape_csv(&asset.firmware_version.clone().unwrap_or_default()),
            escape_csv(&latest_firmware.unwrap_or_default()),
            firmware_status,
            escape_csv(&asset.last_maintenance.clone().unwrap_or_default()),
            risk_score,
            escape_csv(&asset.criticality),
            escape_csv(&asset.location.clone().unwrap_or_default()),
            escape_csv(&asset.ip_address.clone().unwrap_or_default()),
        ));
    }

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"nis2-asset-export.csv\"".to_string(),
            ),
        ],
        csv,
    ))
}

/// Escape a string for CSV: wrap in quotes and double any internal quotes.
fn escape_csv(s: &str) -> String {
    s.replace('"', "\"\"")
}
