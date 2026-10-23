use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use axum::Json;
use axum::extract::{Extension, Multipart, State};
use axum::response::IntoResponse;
use rand::Rng;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::asset::models::CreateAssetRequest;

/// In-memory store for import previews with expiry.
struct PreviewEntry {
    rows: Vec<HashMap<String, String>>,
    created_at: Instant,
}

/// 15-minute expiry for import previews.
const PREVIEW_TTL_SECS: u64 = 900;

static IMPORT_STORE: Mutex<Option<HashMap<String, PreviewEntry>>> = Mutex::new(None);

fn get_store() -> std::sync::MutexGuard<'static, Option<HashMap<String, PreviewEntry>>> {
    let mut store = IMPORT_STORE.lock().unwrap();
    if store.is_none() {
        *store = Some(HashMap::new());
    }
    store
}

/// Case-insensitive header-to-field mapping for CSV/XLSX columns.
fn auto_map_header(header: &str) -> Option<&'static str> {
    let h = header.trim().to_lowercase().replace(['_', '-'], " ");
    match h.as_str() {
        "name" | "asset name" => Some("name"),
        "type" | "asset type" => Some("asset_type"),
        "vendor" | "manufacturer" => Some("vendor"),
        "model" => Some("model"),
        "firmware" | "firmware version" | "firmware_version" => Some("firmware_version"),
        "serial" | "serial number" | "serial_number" => Some("serial_number"),
        "ip" | "ip address" | "ip_address" => Some("ip_address"),
        "mac" | "mac address" | "mac_address" => Some("mac_address"),
        "location" => Some("location"),
        "criticality" => Some("criticality"),
        "notes" | "description" => Some("notes"),
        _ => None,
    }
}

/// Parse a single row of string values into a CreateAssetRequest.
fn parse_row(row: &HashMap<String, String>) -> Result<CreateAssetRequest, String> {
    let name = row
        .get("name")
        .cloned()
        .unwrap_or_default()
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("name is required".to_string());
    }

    let asset_type = row
        .get("asset_type")
        .cloned()
        .unwrap_or_else(|| "other".to_string());

    Ok(CreateAssetRequest {
        name,
        asset_type,
        vendor: row.get("vendor").cloned(),
        model: row.get("model").cloned(),
        firmware_version: row.get("firmware_version").cloned(),
        serial_number: row.get("serial_number").cloned(),
        ip_address: row.get("ip_address").cloned(),
        mac_address: row.get("mac_address").cloned(),
        location: row.get("location").cloned(),
        criticality: row
            .get("criticality")
            .cloned()
            .unwrap_or_else(|| "medium".to_string()),
        tags: Vec::new(),
        notes: row.get("notes").cloned(),
    })
}

// ── Upload Import ──

#[utoipa::path(
    post,
    path = "/api/v1/assets/import",
    responses(
        (status = 200, body = crate::asset::models::ImportPreview),
        (status = 400),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn upload_import(
    State(_state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    // Read the file field
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut file_name: Option<String> = None;

    while let Some(field) = multipart.next_field().await? {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" {
            file_name = field.file_name().map(|s: &str| s.to_string());
            file_bytes = Some(field.bytes().await?.to_vec());
        }
    }

    let bytes = file_bytes.ok_or_else(|| AppError(anyhow::anyhow!("No file uploaded")))?;
    let original_name = file_name.unwrap_or_else(|| "unknown".to_string());

    let rows = if original_name.to_lowercase().ends_with(".xlsx")
        || original_name.to_lowercase().ends_with(".xls")
    {
        parse_xlsx(&bytes)?
    } else {
        // Default to CSV
        parse_csv(&bytes)?
    };

    let total = rows.len();
    let mut valid = 0;
    let mut errors = Vec::new();
    let mut preview_rows = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        match parse_row(row) {
            Ok(req) => {
                valid += 1;
                preview_rows.push(json!({
                    "row": idx + 1,
                    "name": req.name,
                    "asset_type": req.asset_type,
                    "vendor": req.vendor,
                    "model": req.model,
                    "ip_address": req.ip_address,
                }));
            }
            Err(e) => {
                errors.push(format!("Row {}: {}", idx + 1, e));
            }
        }
    }

    let import_id: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(12)
        .map(char::from)
        .collect();

    {
        let mut store = get_store();
        if let Some(ref mut s) = *store {
            s.insert(
                import_id.clone(),
                PreviewEntry {
                    rows: rows.clone(),
                    created_at: Instant::now(),
                },
            );
        }
    }

    let _ = &auth; // auth used for audit

    Ok(Json(json!({
        "import_id": import_id,
        "rows": preview_rows,
        "total": total,
        "valid": valid,
        "errors": errors,
    })))
}

/// Parse CSV bytes into rows.
fn parse_csv(bytes: &[u8]) -> Result<Vec<HashMap<String, String>>, AppError> {
    let content = String::from_utf8_lossy(bytes);
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.to_string()).collect();

    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record?;
        let mut row = HashMap::new();
        for (i, val) in record.iter().enumerate() {
            if let Some(header) = headers.get(i)
                && let Some(field) = auto_map_header(header)
                && !val.trim().is_empty()
            {
                row.insert(field.to_string(), val.trim().to_string());
            }
        }
        rows.push(row);
    }

    Ok(rows)
}

/// Parse XLSX bytes into rows using calamine.
fn parse_xlsx(bytes: &[u8]) -> Result<Vec<HashMap<String, String>>, AppError> {
    use calamine::{DataType, Reader, open_workbook_from_rs};

    let cursor = std::io::Cursor::new(bytes);
    let mut workbook: calamine::Xlsx<_> =
        open_workbook_from_rs(cursor).map_err(|e| anyhow::anyhow!("{}", e))?;

    let sheet_names = workbook.sheet_names().to_vec();
    if sheet_names.is_empty() {
        return Ok(Vec::new());
    }

    let range = workbook
        .worksheet_range(&sheet_names[0])
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut rows = Vec::new();
    let mut headers: Vec<Option<String>> = Vec::new();

    for (row_idx, row) in range.rows().enumerate() {
        if row_idx == 0 {
            // Header row
            headers = row.iter().map(|cell| cell.as_string()).collect();
            continue;
        }

        let mut mapped_row = HashMap::new();
        for (col_idx, cell) in row.iter().enumerate() {
            if let Some(Some(header)) = headers.get(col_idx)
                && let Some(field) = auto_map_header(header)
            {
                let val = cell.as_string().unwrap_or_default();
                if !val.trim().is_empty() {
                    mapped_row.insert(field.to_string(), val.trim().to_string());
                }
            }
        }
        if !mapped_row.is_empty() {
            rows.push(mapped_row);
        }
    }

    Ok(rows)
}

// ── Confirm Import ──

#[utoipa::path(
    post,
    path = "/api/v1/assets/import/confirm",
    request_body = crate::asset::models::ImportConfirmRequest,
    responses(
        (status = 200, body = crate::asset::models::ImportResult),
        (status = 400),
        (status = 404),
    ),
    tag = "Assets",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn confirm_import(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(body): Json<crate::asset::models::ImportConfirmRequest>,
) -> Result<impl IntoResponse, AppError> {
    let import_id = &body.import_id;

    // Retrieve preview from store
    let rows = {
        let mut store = get_store();
        let entry = store
            .as_mut()
            .and_then(|s| s.remove(import_id))
            .ok_or_else(|| AppError(anyhow::anyhow!("Import preview not found or expired")))?;

        // Check expiry
        if entry.created_at.elapsed().as_secs() > PREVIEW_TTL_SECS {
            return Err(AppError(anyhow::anyhow!("Import preview has expired")));
        }

        entry.rows
    };

    let mut created = 0usize;
    let skipped = 0usize;
    let mut errors = Vec::new();

    for (idx, row) in rows.iter().enumerate() {
        match parse_row(row) {
            Ok(req) => {
                match state.storage.asset().create_asset(&req).await {
                    Ok(asset) => {
                        // Recompute risk score
                        let asset_storage = state.storage.asset();
                        let _ = crate::asset::risk::recompute_and_update(&asset_storage, &asset.id)
                            .await;
                        created += 1;
                    }
                    Err(e) => {
                        errors.push(format!("Row {}: {}", idx + 1, e));
                    }
                }
            }
            Err(e) => {
                errors.push(format!("Row {}: {}", idx + 1, e));
            }
        }
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "confirm_asset_import".to_string(),
        username: auth.username.clone(),
        resource: format!("import:{}", import_id),
        details: Some(format!(
            "created={}, skipped={}, errors={}",
            created,
            skipped,
            errors.len()
        )),
    });

    Ok(Json(json!({
        "total": rows.len(),
        "created": created,
        "skipped_duplicates": skipped,
        "errors": errors,
    })))
}
