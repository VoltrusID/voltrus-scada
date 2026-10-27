use std::collections::HashMap;

use axum::Json;
use axum::extract::{Extension, Multipart, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState, AuthUser};

#[derive(Debug, Deserialize)]
pub struct ImportQuery {
    pub dry_run: Option<String>,
    pub mode: Option<String>,
    pub delimiter: Option<String>,
    pub column_mapping: Option<String>,
}

fn auto_detect_columns(headers: &[String]) -> HashMap<String, usize> {
    let mut mapping = HashMap::new();

    for (idx, header) in headers.iter().enumerate() {
        let h = header.trim().to_lowercase().replace(['_', '-'], " ");

        match h.as_str() {
            "name" | "tag" | "tag name" | "sensor" | "sensor name" | "key" | "data source"
            | "tagname" | "point" | "point name" => {
                mapping.insert("tag_name".to_string(), idx);
            }
            "value" | "val" | "reading" | "measurement" | "data" | "pv" => {
                mapping.insert("value".to_string(), idx);
            }
            "timestamp" | "time" | "ts" | "datetime" | "date" | "date time" => {
                mapping.insert("timestamp".to_string(), idx);
            }
            "device" | "device id" | "deviceid" | "device_name" | "plc" => {
                mapping.insert("device_id".to_string(), idx);
            }
            _ => {}
        }
    }

    mapping
}

fn parse_timestamp(s: &str) -> Result<i64, String> {
    if let Ok(dt) =
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Iso8601::DEFAULT)
    {
        return Ok(dt.unix_timestamp());
    }
    if let Ok(dt) =
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Iso8601::DATE)
    {
        return Ok(dt.unix_timestamp());
    }
    if let Ok(ts) = s.parse::<i64>() {
        return Ok(ts);
    }
    if let Ok(ts_ms) = s.parse::<f64>() {
        return Ok(ts_ms as i64 / 1000);
    }

    Err(format!("Cannot parse timestamp: '{}'", s))
}

pub async fn import_csv(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Query(params): Query<ImportQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let dry_run = params.dry_run.as_deref() == Some("true");
    let mode = params.mode.as_deref().unwrap_or("merge");

    if mode != "merge" {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "mode must be 'merge'; to replace readings, delete the range first with DELETE /api/v1/telemetry"
            })),
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
    let original_name = file_name.unwrap_or_else(|| "data.csv".to_string());

    if !original_name.to_lowercase().ends_with(".csv") {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Only CSV files are supported"})),
        )
            .into_response());
    }

    let content = String::from_utf8_lossy(&bytes);
    let delimiter = match params.delimiter.as_deref() {
        Some("\\t") => b'\t',
        Some(";") => b';',
        _ => b',',
    };

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(content.as_bytes());

    let headers: Vec<String> = reader.headers()?.iter().map(|h| h.to_string()).collect();

    let column_mapping = match &params.column_mapping {
        Some(json_str) => {
            let provided: HashMap<String, usize> = serde_json::from_str(json_str)
                .map_err(|e| AppError(anyhow::anyhow!("Invalid column_mapping JSON: {}", e)))?;
            // Validate indices are within header bounds
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
        None => auto_detect_columns(&headers),
    };

    if !column_mapping.contains_key("tag_name") {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Could not auto-detect tag/name column. Ensure your CSV has a column named 'name', 'tag', 'sensor', or 'key'.",
                "detected_columns": headers,
                "expected": ["name/tag/sensor/key", "value", "timestamp (optional)", "device (optional)"],
            })),
        ).into_response());
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

    for (row_num, record) in reader.records().enumerate() {
        let record = match record {
            Ok(r) => r,
            Err(e) => {
                errors.push(json!({
                    "row": row_num + 2,
                    "error": format!("CSV parse error: {}", e),
                }));
                total_rows += 1;
                continue;
            }
        };

        total_rows += 1;

        let tag_name = match record.get(tag_name_idx) {
            Some(v) if !v.trim().is_empty() => v.trim().to_string(),
            _ => {
                errors.push(json!({
                    "row": row_num + 2,
                    "error": "Empty tag name",
                }));
                skipped += 1;
                continue;
            }
        };

        let value = match value_idx.and_then(|i| record.get(i)) {
            Some(v) => match v.trim().parse::<f64>() {
                Ok(f) => f,
                Err(_) => {
                    errors.push(json!({
                        "row": row_num + 2,
                        "tag": tag_name,
                        "error": format!("Invalid value: '{}'", v.trim()),
                    }));
                    skipped += 1;
                    continue;
                }
            },
            None => {
                errors.push(json!({
                    "row": row_num + 2,
                    "tag": tag_name,
                    "error": "No value column detected",
                }));
                skipped += 1;
                continue;
            }
        };

        let device_id = device_idx
            .and_then(|i| record.get(i))
            .map(|v| v.trim().to_string())
            .unwrap_or_else(|| "default".to_string());

        let timestamp = match timestamp_idx.and_then(|i| record.get(i)) {
            Some(v) if !v.trim().is_empty() => match parse_timestamp(v.trim()) {
                Ok(ts) => ts,
                Err(e) => {
                    errors.push(json!({
                        "row": row_num + 2,
                        "tag": tag_name,
                        "error": e,
                    }));
                    skipped += 1;
                    continue;
                }
            },
            _ => crate::timeutil::now_secs(),
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
        action: "csv_data_import".to_string(),
        username: auth.username.clone(),
        resource: format!("import:{}", original_name),
        details: Some(format!(
            "total={}, imported={}, skipped={}, errors={}, dry_run={}",
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
        })),
    )
        .into_response())
}

// Public wrappers for reuse by excel_import
pub fn auto_detect_columns_pub(headers: &[String]) -> HashMap<String, usize> {
    auto_detect_columns(headers)
}

pub fn parse_timestamp_pub(s: &str) -> Result<i64, String> {
    parse_timestamp(s)
}
