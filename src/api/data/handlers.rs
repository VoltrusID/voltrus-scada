use super::*;

/// GET /api/data?screen=X — fetch data sources, generate values using generators
/// For source_type == "modbus": read from cache, evaluate expression if present
#[utoipa::path(
    get,
    path = "/api/v1/data",
    responses(
        (status = 200, body = Vec<DataSourceData>),
    ),
    tag = "Data",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_data(
    State(state): State<AppState>,
    Query(params): Query<DataQuery>,
) -> Result<impl IntoResponse, AppError> {
    let screen_id = params.screen.as_deref();
    let data_sources = state.storage.list_data_sources(screen_id).await?;
    let cache_data = state.cache.get_all().await;
    let now = crate::timeutil::now_secs() as f64;
    let result = evaluate::evaluate_data_sources(&data_sources, &cache_data, now);
    Ok(Json(result))
}

/// GET /api/history?range=30m&screen=X&sensor=key1,key2&agg=sampling
/// For modbus sources: reads real telemetry data from DB
/// For non-modbus sources: generates synthetic data via generators
/// `sensor` (comma-separated tag keys) and `device` narrow the answer to those tags.
/// `agg` is `sampling` (default), `avg` or `minmax`: the last two bucket the series into
/// `interval` and `minmax` adds `min` and `max` arrays next to `values`.
/// `data` is keyed by tag key, and keys are not unique (two devices may share a register
/// name), so the later source overwrites the earlier one; `keyed_by=id` keys it by data
/// source id (the `id` of each `sensors` entry) instead, which is always unique.
/// Returns compact format: each sensor has {values: [f64], start: unix_ms, interval: ms, count: usize}
#[utoipa::path(
    get,
    path = "/api/v1/history",
    responses(
        (status = 200, body = HistoryResponse),
    ),
    tag = "Data",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_history(
    State(state): State<AppState>,
    Query(params): Query<DataQuery>,
) -> Result<impl IntoResponse, AppError> {
    let range_str = params.range.as_deref().unwrap_or("30m");
    let detail = params.detail.as_deref();
    let agg = params.agg.as_deref().unwrap_or("sampling");
    let screen_id = params.screen.as_deref();
    let keyed_by_id = params.keyed_by.as_deref() == Some("id");
    let data_sources = history::filter_sources(
        state.storage.list_data_sources(screen_id).await?,
        params.sensor.as_deref(),
        params.device.as_deref().or(params.device_id.as_deref()),
    );

    let (start, end) = generators::parse_range(range_str);
    let interval_ms = generators::interval_for_range(range_str, detail);

    // Collect unique device_ids for modbus sources to batch-query
    let modbus_device_ids: Vec<String> = data_sources
        .iter()
        .filter(|ds| ds.source_type == "modbus")
        .map(|ds| ds.device_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let now = crate::timeutil::now_secs();
    let raw_keep = state.retention_config.raw_window_secs();
    let hourly_keep = state.retention_config.hourly_window_secs();
    let raw_cutoff = now - raw_keep;
    let hourly_cutoff = raw_cutoff - hourly_keep;
    let min_interval_secs = (interval_ms / 1000).max(1) as i64;

    // Query all telemetry for modbus devices across all tiers
    let mut device_data: std::collections::HashMap<
        String,
        Vec<(i64, std::collections::HashMap<String, f64>)>,
    > = std::collections::HashMap::new();
    for device_id in &modbus_device_ids {
        let data = state
            .storage
            .query_device_metrics_merged(
                device_id,
                start,
                end,
                raw_cutoff,
                hourly_cutoff,
                min_interval_secs,
            )
            .await?;
        device_data.insert(device_id.clone(), data);
    }

    let mut data_source_summaries: Vec<DataSourceSummary> = Vec::new();
    let mut data_map = serde_json::Map::new();

    let sub_samples = if agg == "sampling" {
        1
    } else {
        (interval_ms / 1000).max(1) as usize
    };

    for data_source in &data_sources {
        let series_key = if keyed_by_id {
            data_source.id.to_string()
        } else {
            data_source.key.clone()
        };
        data_source_summaries.push(DataSourceSummary {
            id: data_source.id,
            key: data_source.key.clone(),
            name: data_source.name.clone(),
            unit: data_source.unit.clone(),
            color: data_source.color.clone(),
            warn_val: data_source.warn_val,
            crit_val: data_source.crit_val,
            decimals: data_source.decimals,
            source_type: data_source.source_type.clone(),
        });

        let (points, band) = if data_source.source_type == "modbus" {
            // Real data from telemetry DB
            let raw_points = device_data
                .get(&data_source.device_id)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);

            // Check if there's an expression to evaluate
            let expr: Option<String> =
                serde_json::from_str::<serde_json::Value>(&data_source.source_config)
                    .ok()
                    .and_then(|cfg| {
                        cfg.get("expression")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                    });

            let samples: Vec<DataPoint> = if let Some(ref expr_str) = expr {
                // Expression-based: evaluate at each timestamp
                raw_points
                    .iter()
                    .filter_map(
                        |(ts, metrics)| match expression::evaluate(expr_str, metrics) {
                            Ok(val) => Some(DataPoint {
                                timestamp: *ts,
                                value: val,
                            }),
                            Err(_) => None,
                        },
                    )
                    .collect::<Vec<_>>()
            } else {
                // Simple metric lookup
                raw_points
                    .iter()
                    .filter_map(|(ts, metrics)| {
                        metrics
                            .get(metric_name(&data_source.key, &data_source.device_id))
                            .map(|&v| DataPoint {
                                timestamp: *ts,
                                value: v,
                            })
                    })
                    .collect::<Vec<_>>()
            };

            match agg {
                "avg" | "minmax" => {
                    let pairs: Vec<(i64, f64)> =
                        samples.iter().map(|p| (p.timestamp, p.value)).collect();
                    history::series_from_buckets(
                        history::bucket_samples(&pairs, min_interval_secs),
                        agg == "minmax",
                    )
                }
                _ => (samples, None),
            }
        } else {
            // Synthetic data from generators
            let duration_secs = end - start;
            let num_points = ((duration_secs * 1000) / interval_ms as i64) as usize;
            let mut pts = Vec::with_capacity(num_points);
            let mut band = (agg == "minmax").then(|| {
                (
                    Vec::with_capacity(num_points),
                    Vec::with_capacity(num_points),
                )
            });
            for i in 0..num_points {
                let bucket_start = start + ((end - start) * i as i64 / num_points.max(1) as i64);
                match agg {
                    "avg" | "minmax" => {
                        let (avg, mn, mx) = generators::aggregate_bucket(
                            data_source,
                            bucket_start as f64,
                            sub_samples,
                        );
                        pts.push(DataPoint {
                            timestamp: bucket_start,
                            value: avg,
                        });
                        if let Some((lo, hi)) = band.as_mut() {
                            lo.push(mn);
                            hi.push(mx);
                        }
                    }
                    _ => {
                        let value = generators::generate_value(data_source, bucket_start as f64);
                        pts.push(DataPoint {
                            timestamp: bucket_start,
                            value: (value * 100.0).round() / 100.0,
                        });
                    }
                }
            }
            (pts, band)
        };

        // Convert points to compact format
        if points.is_empty() {
            let mut sensor_data = serde_json::Map::new();
            sensor_data.insert("values".into(), json!(Vec::<f64>::new()));
            if agg == "minmax" {
                sensor_data.insert("min".into(), json!(Vec::<f64>::new()));
                sensor_data.insert("max".into(), json!(Vec::<f64>::new()));
            }
            sensor_data.insert("start".into(), json!(start * 1000));
            sensor_data.insert("interval".into(), json!(interval_ms));
            sensor_data.insert("count".into(), json!(0));
            data_map.insert(series_key.clone(), json!(sensor_data));
        } else {
            let values: Vec<f64> = points.iter().map(|p| p.value).collect();
            let first_ts = points[0].timestamp;
            let last_ts = points[points.len() - 1].timestamp;
            let calc_interval = if points.len() > 1 {
                ((last_ts - first_ts) * 1000 / (points.len() as i64 - 1).max(1)) as u64
            } else {
                interval_ms
            };
            let mut sensor_data = serde_json::Map::new();
            sensor_data.insert("values".into(), json!(values));
            if let Some((lo, hi)) = &band {
                sensor_data.insert("min".into(), json!(lo));
                sensor_data.insert("max".into(), json!(hi));
            }
            sensor_data.insert("start".into(), json!(first_ts * 1000));
            sensor_data.insert("interval".into(), json!(calc_interval));
            sensor_data.insert("count".into(), json!(points.len()));
            data_map.insert(series_key.clone(), json!(sensor_data));
        }
    }

    Ok(Json(HistoryResponse {
        sensors: data_source_summaries,
        data: serde_json::Value::Object(data_map),
        range: range_str.to_string(),
        interval: interval_ms,
    }))
}

/// GET /api/export/csv?sensor=X&range=30m — generate CSV from history data
#[utoipa::path(
    get,
    path = "/api/v1/export/csv",
    responses(
        (status = 200, description = "CSV file download"),
    ),
    tag = "Data",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn export_csv(
    State(state): State<AppState>,
    Query(params): Query<DataQuery>,
) -> Result<impl IntoResponse, AppError> {
    let range_str = params.range.as_deref().unwrap_or("30m");
    let sensor_key = params.sensor.as_deref().unwrap_or("");

    // Find data sources matching the key
    let all_data_sources = state.storage.list_data_sources(None).await?;
    let data_sources: Vec<&DataSource> = if sensor_key.is_empty() {
        all_data_sources.iter().collect()
    } else {
        all_data_sources
            .iter()
            .filter(|s| s.key == sensor_key)
            .collect()
    };

    let (start, end) = generators::parse_range(range_str);
    let interval_ms = generators::interval_for_range(range_str, params.detail.as_deref());
    let duration_secs = end - start;
    let num_points = ((duration_secs * 1000) / interval_ms as i64) as usize;

    // Build CSV
    let mut csv = String::from("timestamp");
    for data_source in &data_sources {
        csv.push_str(&format!(",{}", data_source.key));
    }
    csv.push('\n');

    for i in 0..num_points {
        let t = start + ((end - start) * i as i64 / num_points.max(1) as i64);
        csv.push_str(&format!("{}", t));
        for data_source in &data_sources {
            let value = generators::generate_value(data_source, t as f64);
            csv.push_str(&format!(",{:.2}", value));
        }
        csv.push('\n');
    }

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv".to_string()),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"export.csv\"".to_string(),
            ),
        ],
        csv,
    ))
}

/// GET /api/v1/data/export — programmatic data export with filtering, aggregation, pagination.
///
/// Query parameters:
/// - `from` / `to` — time range filter (ISO 8601 or unix timestamp)
/// - `device` — filter by device ID
/// - `metric` — filter by metric name
/// - `aggregation` — avg, min, max, count, sum (default: raw/no aggregation)
/// - `interval` — aggregation window (1m, 5m, 15m, 1h, 1d)
/// - `limit` / `offset` — pagination (default limit: 1000, max: 10000)
/// - `format` — json (default), csv
#[utoipa::path(
    get,
    path = "/api/v1/data/export",
    responses(
        (status = 200, description = "JSON export, or CSV when format=csv", body = ExportResponse),
    ),
    tag = "Data",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn export_data(
    State(state): State<AppState>,
    Query(params): Query<ExportQuery>,
) -> Result<axum::response::Response, AppError> {
    // Parse time range
    let from_ts = params
        .from
        .as_deref()
        .map(helpers::parse_time_param)
        .transpose()
        .map_err(|e| AppError(anyhow::anyhow!(e)))?;

    let to_ts = params
        .to
        .as_deref()
        .map(helpers::parse_time_param)
        .transpose()
        .map_err(|e| AppError(anyhow::anyhow!(e)))?;

    // Validate aggregation
    let agg = params.aggregation.as_deref().unwrap_or("");
    if !agg.is_empty() && !helpers::validate_aggregation(agg) {
        return Err(AppError(anyhow::anyhow!(
            "Invalid aggregation '{}'. Must be one of: avg, min, max, count, sum",
            agg
        )));
    }
    let aggregation = if agg.is_empty() { None } else { Some(agg) };

    // Validate interval — required when aggregation is specified
    let intv = params.interval.as_deref().unwrap_or("");
    if aggregation.is_some() && intv.is_empty() {
        return Err(AppError(anyhow::anyhow!(
            "Interval is required when aggregation is specified. Use one of: 1m, 5m, 15m, 1h, 1d"
        )));
    }
    if !intv.is_empty() && !helpers::validate_interval(intv) {
        return Err(AppError(anyhow::anyhow!(
            "Invalid interval '{}'. Must be one of: 1m, 5m, 15m, 1h, 1d",
            intv
        )));
    }
    let interval = if intv.is_empty() { None } else { Some(intv) };

    // Pagination with defaults and clamping
    let limit = params
        .limit
        .unwrap_or(helpers::DEFAULT_EXPORT_LIMIT)
        .min(helpers::MAX_EXPORT_LIMIT);
    let offset = params.offset.unwrap_or(0);

    // Query data
    let (rows, total) = state
        .storage
        .query_telemetry_export(
            params.device.as_deref(),
            params.metric.as_deref(),
            from_ts,
            to_ts,
            aggregation,
            interval,
            limit,
            offset,
        )
        .await?;

    // Determine response format
    let format = params.format.as_deref().unwrap_or("json");

    // Build meta
    let meta = ExportMeta {
        total,
        limit,
        offset,
        aggregation: aggregation.unwrap_or("raw").to_string(),
        interval: interval.unwrap_or("none").to_string(),
    };

    match format {
        "csv" => {
            // Stream CSV — build as string (SQLite already bounded by limit)
            let mut csv = String::with_capacity(rows.len() * 80);
            csv.push_str("timestamp,device,metric,value\n");
            for (ts, device, metric, value) in &rows {
                csv.push_str(&format!("{ts},{device},{metric},{value}\n"));
            }
            Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"telemetry-export.csv\"".to_string(),
                    ),
                ],
                csv,
            )
                .into_response())
        }
        _ => {
            // JSON response
            let data: Vec<TelemetryExportRow> = rows
                .into_iter()
                .map(|(timestamp, device, metric, value)| TelemetryExportRow {
                    timestamp,
                    device,
                    metric,
                    value,
                })
                .collect();
            Ok(Json(ExportResponse { data, meta }).into_response())
        }
    }
}
