use std::io::Cursor;

use axum::Json;
use axum::extract::{Extension, Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use calamine::{Data, Reader, open_workbook_auto_from_rs};
use serde::Deserialize;
use serde_json::json;

use super::csv_import::{auto_detect_columns_pub, parse_timestamp_pub};
use super::{AppError, AppState, AuthUser};

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    pub dry_run: Option<String>,
    pub mode: Option<String>,
    pub sheet: Option<String>,
    pub column_mapping: Option<String>,
}

fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            // Show integers without decimal point
            if *f == (*f as i64) as f64 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Data::Int(i) => format!("{}", i),
        Data::Bool(b) => format!("{}", b),
        Data::DateTime(dt) => {
            // Convert Excel serial date to a readable string, then let parse_timestamp handle it
            let serial = dt.as_f64();
            // Excel serial date: days since 1900-01-00 (with the 1900 leap year bug)
            // Unix epoch in Excel serial is 25569
            let unix_secs = ((serial - 25569.0) * 86400.0) as i64;
            format!("{}", unix_secs)
        }
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR:{:?}", e),
        Data::Empty => String::new(),
    }
}

fn cell_to_f64(cell: &Data) -> Option<f64> {
    match cell {
        Data::Float(f) => Some(*f),
        Data::Int(i) => Some(*i as f64),
        Data::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Data::String(s) => s.trim().parse::<f64>().ok(),
        Data::Empty => None,
        _ => None,
    }
}

pub async fn import_excel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(params): Query<ImportQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let dry_run = params.dry_run.as_deref() == Some("true");
    let mode = params.mode.as_deref().unwrap_or("merge");

    if mode != "merge" && mode != "replace" {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "mode must be 'merge' or 'replace'"})),
        )
            .into_response());
    }
    if mode == "replace" {
        return Ok((
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({"error": "replace mode is not yet supported; use merge"})),
        )
            .into_response());
    }

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
    let original_name = file_name.unwrap_or_else(|| "data.xlsx".to_string());

    let lower_name = original_name.to_lowercase();
    if !lower_name.ends_with(".xlsx")
        && !lower_name.ends_with(".xls")
        && !lower_name.ends_with(".xlsm")
        && !lower_name.ends_with(".xlsb")
    {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Only Excel files are supported (.xlsx, .xls, .xlsm, .xlsb)"})),
        )
            .into_response());
    }

    let cursor = Cursor::new(bytes);
    let mut workbook = open_workbook_auto_from_rs(cursor)
        .map_err(|e| AppError(anyhow::anyhow!("Failed to open Excel file: {}", e)))?;

    let available_sheets: Vec<String> = workbook.sheet_names().to_vec();

    if available_sheets.is_empty() {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Workbook contains no sheets"})),
        )
            .into_response());
    }

    // Select sheet: explicit param > first sheet
    let sheet_name = match &params.sheet {
        Some(s) if !s.is_empty() => {
            if !available_sheets.contains(s) {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": format!("Sheet '{}' not found", s),
                        "available_sheets": available_sheets,
                    })),
                )
                    .into_response());
            }
            s.clone()
        }
        _ => available_sheets[0].clone(),
    };

    let range = workbook.worksheet_range(&sheet_name).map_err(|e| {
        AppError(anyhow::anyhow!(
            "Failed to read sheet '{}': {}",
            sheet_name,
            e
        ))
    })?;

    let mut rows_iter = range.rows();

    // First row = headers
    let headers: Vec<String> = match rows_iter.next() {
        Some(row) => row.iter().map(cell_to_string).collect(),
        None => {
            return Ok((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Sheet is empty (no header row)"})),
            )
                .into_response());
        }
    };

    let column_mapping = match &params.column_mapping {
        Some(json_str) => {
            let provided: std::collections::HashMap<String, usize> = serde_json::from_str(json_str)
                .map_err(|e| AppError(anyhow::anyhow!("Invalid column_mapping JSON: {}", e)))?;
            for (key, &idx) in &provided {
                if idx >= headers.len() {
                    return Ok((
                        StatusCode::BAD_REQUEST,
                        Json(json!({
                            "error": format!("column_mapping '{}' index {} out of bounds (file has {} columns)", key, idx, headers.len()),
                        })),
                    ).into_response());
                }
            }
            if !provided.contains_key("tag_name") {
                return Ok((
                    StatusCode::BAD_REQUEST,
                    Json(json!({
                        "error": "column_mapping must include 'tag_name'",
                    })),
                )
                    .into_response());
            }
            provided
        }
        None => auto_detect_columns_pub(&headers),
    };

    if !column_mapping.contains_key("tag_name") {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Could not auto-detect tag/name column. Ensure your sheet has a column named 'name', 'tag', 'sensor', or 'key'.",
                "detected_columns": headers,
                "expected": ["name/tag/sensor/key", "value", "timestamp (optional)", "device (optional)"],
                "sheet": sheet_name,
                "available_sheets": available_sheets,
            })),
        )
            .into_response());
    }

    let tag_name_idx = column_mapping["tag_name"];
    let value_idx = column_mapping.get("value").copied();
    let timestamp_idx = column_mapping.get("timestamp").copied();
    let device_idx = column_mapping.get("device_id").copied();

    let mut total_rows = 0usize;
    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<serde_json::Value> = Vec::new();

    let mut batch: Vec<(String, i64, String, f64)> = Vec::new();

    for (row_num, row) in rows_iter.enumerate() {
        let display_row = row_num + 2; // 1-based, accounting for header

        total_rows += 1;

        // Tag name
        let tag_name = match row.get(tag_name_idx) {
            Some(cell) => {
                let s = cell_to_string(cell);
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    errors.push(json!({
                        "row": display_row,
                        "error": "Empty tag name",
                    }));
                    skipped += 1;
                    continue;
                }
                trimmed.to_string()
            }
            None => {
                errors.push(json!({
                    "row": display_row,
                    "error": "Missing tag name column",
                }));
                skipped += 1;
                continue;
            }
        };

        // Value
        let value = match value_idx.and_then(|i| row.get(i)) {
            Some(cell) => match cell_to_f64(cell) {
                Some(f) => f,
                None => {
                    let raw = cell_to_string(cell);
                    match raw.trim().parse::<f64>() {
                        Ok(f) => f,
                        Err(_) => {
                            errors.push(json!({
                                "row": display_row,
                                "tag": tag_name,
                                "error": format!("Invalid value: '{}'", raw.trim()),
                            }));
                            skipped += 1;
                            continue;
                        }
                    }
                }
            },
            None => {
                errors.push(json!({
                    "row": display_row,
                    "tag": tag_name,
                    "error": "No value column detected",
                }));
                skipped += 1;
                continue;
            }
        };

        // Device ID
        let device_id = device_idx
            .and_then(|i| row.get(i))
            .map(cell_to_string)
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "default".to_string());

        // Timestamp
        let timestamp = match timestamp_idx.and_then(|i| row.get(i)) {
            Some(cell) => {
                let s = cell_to_string(cell);
                if s.trim().is_empty() {
                    crate::timeutil::now_secs()
                } else {
                    match parse_timestamp_pub(s.trim()) {
                        Ok(ts) => ts,
                        Err(e) => {
                            errors.push(json!({
                                "row": display_row,
                                "tag": tag_name,
                                "error": e,
                            }));
                            skipped += 1;
                            continue;
                        }
                    }
                }
            }
            None => crate::timeutil::now_secs(),
        };

        if !dry_run {
            batch.push((device_id.clone(), timestamp, tag_name.clone(), value));

            if batch.len() >= 500 {
                match state.storage.insert_batch(&batch).await {
                    Ok(_) => imported += batch.len(),
                    Err(e) => {
                        errors.push(json!({
                            "error": format!("Batch insert failed ({} rows dropped): {}", batch.len(), e),
                        }));
                        skipped += batch.len();
                    }
                }
                batch.clear();
            }
        } else {
            imported += 1;
        }
    }

    // Flush remaining batch
    if !batch.is_empty() && !dry_run {
        match state.storage.insert_batch(&batch).await {
            Ok(_) => imported += batch.len(),
            Err(e) => {
                errors.push(json!({
                    "error": format!("Final batch insert failed ({} rows dropped): {}", batch.len(), e),
                }));
                skipped += batch.len();
            }
        }
    }

    state.audit_logger.log(crate::security::AuditEvent {
        action: "excel_data_import".to_string(),
        username: auth.username.clone(),
        resource: format!("import:{}", original_name),
        details: Some(format!(
            "sheet={}, total={}, imported={}, skipped={}, errors={}, dry_run={}",
            sheet_name,
            total_rows,
            imported,
            skipped,
            errors.len(),
            dry_run
        )),
    });

    Ok((
        StatusCode::OK,
        Json(json!({
            "total_rows": total_rows,
            "imported": imported,
            "skipped": skipped,
            "errors": errors,
            "dry_run": dry_run,
            "mode": mode,
            "column_mapping": column_mapping,
            "sheet": sheet_name,
            "available_sheets": available_sheets,
        })),
    )
        .into_response())
}
