//! API handlers for external integration & BI connectors (v0.39.0).

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::{AppError, AppState};
use crate::integration::bi::{self, BiQuery};
use crate::integration::egress::{self, CreateEgressJobRequest, UpdateEgressJobRequest};

// ── Prometheus ──

pub async fn prometheus_metrics(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let metrics =
        crate::integration::prometheus::render_metrics(&state.cache, &state.storage).await;
    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        metrics,
    ))
}

// ── BI query ──

pub async fn bi_query(
    State(state): State<AppState>,
    Query(params): Query<BiQuery>,
) -> Result<impl IntoResponse, AppError> {
    let format = params.format.clone().unwrap_or_else(|| "json".to_string());
    let result = bi::execute_bi_query(&state.storage, &params).await?;
    match format.as_str() {
        "csv" => {
            let csv = bi::render_bi_csv(&result.data);
            Ok((
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, "text/csv".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        "attachment; filename=\"voltrus_bi_export.csv\"".to_string(),
                    ),
                ],
                csv,
            )
                .into_response())
        }
        _ => Ok(Json(result).into_response()),
    }
}

pub async fn grafana_query(
    State(state): State<AppState>,
    Query(params): Query<BiQuery>,
) -> Result<impl IntoResponse, AppError> {
    let result = bi::execute_grafana_query(&state.storage, &params).await?;
    Ok(Json(result))
}

pub async fn grafana_search(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let sources = state.storage.list_data_sources(None).await?;
    let targets: Vec<String> = sources
        .iter()
        .map(|s| format!("{}:{}", s.device_id, s.key))
        .collect();
    Ok(Json(json!(targets)))
}

/// Base URL clients should use to reach this server. A wildcard bind address
/// (`0.0.0.0`, `::`) is not routable, so the machine hostname stands in for it;
/// a specific bind address is used as is.
fn connection_base_url(bind_host: &str, port: u16, hostname: &str) -> String {
    let host = match bind_host.trim() {
        "" | "0.0.0.0" | "::" | "[::]" => hostname,
        h => h,
    };
    if host.contains(':') && !host.starts_with('[') {
        format!("http://[{}]:{}", host, port)
    } else {
        format!("http://{}:{}", host, port)
    }
}

pub async fn connection_info(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let hostname = sysinfo::System::host_name().unwrap_or_else(|| "localhost".to_string());
    let (bind_host, port) = match crate::config::load_config(&state.config_path) {
        Ok(cfg) => (cfg.server_host, cfg.server_port),
        Err(_) => ("0.0.0.0".to_string(), 3000),
    };
    let base_url = connection_base_url(&bind_host, port, &hostname);
    Ok(Json(json!({
        "rest_api": { "base_url": format!("{}/api/v1", base_url), "authentication": "Cookie or API Key (X-API-Key header)" },
        "prometheus": { "enabled": true, "endpoint": format!("{}/metrics", base_url), "metrics": ["voltrus_tag_value", "voltrus_device_up", "voltrus_alarms_active"] },
        "grafana": { "query_endpoint": format!("{}/api/v1/integration/grafana/query", base_url), "search_endpoint": format!("{}/api/v1/integration/grafana/search", base_url) },
        "power_bi": { "connector": "Web connector (Get Data > Web)", "url": format!("{}/api/v1/integration/bi/query?format=json", base_url) },
        "sample_queries": {
            "last_hour": format!("{}/api/v1/integration/bi/query?from=now-1h&to=now", base_url),
            "csv_export": format!("{}/api/v1/integration/bi/query?from=now-24h&to=now&format=csv", base_url),
        }
    })))
}

// ── Egress jobs CRUD ──

pub async fn list_egress_jobs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let jobs = state.storage.list_egress_jobs().await?;
    Ok(Json(jobs))
}

pub async fn create_egress_job(
    State(state): State<AppState>,
    Json(req): Json<CreateEgressJobRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_schedule(&req.schedule)?;
    if req.format != "csv" && req.format != "parquet" {
        return Err(AppError(anyhow::anyhow!(
            "format must be 'csv' or 'parquet'"
        )));
    }
    if req.destination.is_empty() {
        return Err(AppError(anyhow::anyhow!("destination is required")));
    }
    if !req.destination.starts_with('/')
        && !req.destination.starts_with("./")
        && !req.destination.starts_with("s3://")
    {
        return Err(AppError(anyhow::anyhow!(
            "destination must be an absolute path, relative path (./), or S3 URL (s3://bucket/prefix/)"
        )));
    }
    egress::parse_time_range(&req.time_range).map_err(AppError)?;
    let id = state
        .storage
        .create_egress_job(
            &req.name,
            req.enabled,
            &req.schedule,
            &req.format,
            &req.destination,
            &req.device_ids,
            &req.time_range,
            req.compression,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": req.name, "schedule": req.schedule, "format": req.format, "destination": req.destination, "enabled": req.enabled }),
        ),
    ))
}

pub async fn get_egress_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    match state.storage.get_egress_job(id).await? {
        Some(job) => Ok(Json(job)),
        None => Err(AppError(anyhow::anyhow!("Egress job not found"))),
    }
}

pub async fn update_egress_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateEgressJobRequest>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(ref schedule) = req.schedule {
        validate_schedule(schedule)?;
    }
    if let Some(ref format) = req.format
        && format != "csv"
        && format != "parquet"
    {
        return Err(AppError(anyhow::anyhow!(
            "format must be 'csv' or 'parquet'"
        )));
    }
    if let Some(ref time_range) = req.time_range {
        egress::parse_time_range(time_range).map_err(AppError)?;
    }
    state
        .storage
        .update_egress_job(
            id,
            req.name.as_deref(),
            req.enabled,
            req.schedule.as_deref(),
            req.format.as_deref(),
            req.destination.as_deref(),
            req.device_ids.as_deref(),
            req.time_range.as_deref(),
            req.compression,
        )
        .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_egress_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.delete_egress_job(id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn run_egress_job(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let job = state
        .storage
        .get_egress_job(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Egress job not found")))?;
    let storage = state.storage.clone();
    let job_name = job.name.clone();
    let job_id = job.id;
    let response_name = job.name.clone();
    tokio::spawn(async move {
        let now = crate::timeutil::now_secs();
        match egress::execute_egress_job(&storage, &job).await {
            Ok(()) => {
                let _ = storage
                    .update_egress_job_status(job_id, Some(now), Some("success".into()), None)
                    .await;
            }
            Err(e) => {
                tracing::error!("Manual egress job '{}' failed: {}", job_name, e);
                let _ = storage
                    .update_egress_job_status(
                        job_id,
                        Some(now),
                        Some("error".into()),
                        Some(e.to_string()),
                    )
                    .await;
            }
        }
    });
    Ok(Json(
        json!({ "ok": true, "message": format!("Egress job '{}' triggered", response_name) }),
    ))
}

// ── Outbound webhooks ──

#[derive(Debug, Clone, Deserialize)]
pub struct CreateOutboundWebhookRequest {
    pub name: String,
    pub url: String,
    #[serde(default = "default_alarm_events")]
    pub events: String,
    #[serde(default = "default_all_filter")]
    pub device_filter: String,
    #[serde(default)]
    pub headers: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_retry")]
    pub retry_count: u32,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}
fn default_alarm_events() -> String {
    r#"["alarm"]"#.to_string()
}
fn default_all_filter() -> String {
    r#""all""#.to_string()
}
fn default_true() -> bool {
    true
}
fn default_retry() -> u32 {
    3
}
fn default_timeout() -> u64 {
    10000
}

pub async fn list_outbound_webhooks(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let webhooks = state.storage.list_outbound_webhooks().await?;
    Ok(Json(webhooks))
}

pub async fn create_outbound_webhook(
    State(state): State<AppState>,
    Json(req): Json<CreateOutboundWebhookRequest>,
) -> Result<impl IntoResponse, AppError> {
    if !req.url.starts_with("http://") && !req.url.starts_with("https://") {
        return Err(AppError(anyhow::anyhow!(
            "URL must start with http:// or https://"
        )));
    }
    let events: Vec<String> = serde_json::from_str(&req.events).map_err(|_| {
        AppError(anyhow::anyhow!(
            "events must be a JSON array of event types"
        ))
    })?;
    for event in &events {
        if event != "alarm" && event != "tag_change" {
            return Err(AppError(anyhow::anyhow!(
                "event type must be \"alarm\" or \"tag_change\""
            )));
        }
    }
    let id = state
        .storage
        .create_outbound_webhook(
            &req.name,
            &req.url,
            &req.events,
            &req.device_filter,
            &req.headers,
            req.enabled,
            req.retry_count,
            req.timeout_ms,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({ "id": id, "name": req.name, "url": req.url, "events": req.events, "enabled": req.enabled }),
        ),
    ))
}

pub async fn delete_outbound_webhook(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.delete_outbound_webhook(id).await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn test_outbound_webhook(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    let webhook = state
        .storage
        .get_outbound_webhook(id)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("Outbound webhook not found")))?;
    let url = webhook
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let headers_str = webhook
        .get("headers")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timeout = webhook
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(10000);
    let test_payload = json!({ "event": "test", "message": "Test webhook from Voltrus", "timestamp": crate::timeutil::now_secs(), "source": "voltrus_webhook_test" });
    let client = reqwest::Client::new();
    let mut request = client
        .post(&url)
        .json(&test_payload)
        .timeout(std::time::Duration::from_millis(timeout));
    if !headers_str.is_empty()
        && let Ok(headers) =
            serde_json::from_str::<std::collections::HashMap<String, String>>(&headers_str)
    {
        for (k, v) in &headers {
            request = request.header(k, v);
        }
    }
    match request.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            Ok(Json(
                json!({ "ok": true, "status": status, "response": body }),
            ))
        }
        Err(e) => Ok(Json(json!({ "ok": false, "error": e.to_string() }))),
    }
}

fn validate_schedule(schedule: &str) -> Result<(), AppError> {
    match schedule {
        "hourly" | "daily" | "weekly" => Ok(()),
        s if s.starts_with("every_") => {
            let rest = s.strip_prefix("every_").unwrap();
            if let Some(m) = rest.strip_suffix('m') {
                m.parse::<u32>()
                    .map_err(|_| AppError(anyhow::anyhow!("Invalid schedule number")))?;
            } else if let Some(h) = rest.strip_suffix('h') {
                h.parse::<u32>()
                    .map_err(|_| AppError(anyhow::anyhow!("Invalid schedule number")))?;
            } else if let Some(d) = rest.strip_suffix('d') {
                d.parse::<u32>()
                    .map_err(|_| AppError(anyhow::anyhow!("Invalid schedule number")))?;
            } else {
                return Err(AppError(anyhow::anyhow!(
                    "Schedule must be 'hourly', 'daily', 'weekly', or 'every_Nm'/'every_Nh'/'every_Nd'"
                )));
            }
            Ok(())
        }
        _ => Err(AppError(anyhow::anyhow!(
            "Schedule must be 'hourly', 'daily', 'weekly', or 'every_Nm'/'every_Nh'/'every_Nd'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::connection_base_url;

    #[test]
    fn base_url_uses_configured_port_and_hostname_for_wildcard_bind() {
        assert_eq!(
            connection_base_url("0.0.0.0", 3000, "scada-1"),
            "http://scada-1:3000"
        );
        assert_eq!(connection_base_url("::", 9000, "scada-1"), "http://scada-1:9000");
    }

    #[test]
    fn base_url_keeps_specific_bind_address() {
        assert_eq!(
            connection_base_url("10.1.2.3", 8443, "scada-1"),
            "http://10.1.2.3:8443"
        );
        assert_eq!(connection_base_url("fd00::5", 3000, "x"), "http://[fd00::5]:3000");
    }
}
