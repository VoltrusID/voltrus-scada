use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;

use crate::analytics::{self, custom};
use crate::api::{AppError, AppState};
use crate::models::{
    CreateCustomMetricRequest, CustomMetric, CustomMetricValue, UpdateCustomMetricRequest,
};

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct AggregateQuery {
    pub device_id: String,
    pub metric: String,
    pub from: i64,
    pub to: i64,
    #[serde(default = "default_interval")]
    pub interval: i64,
}

fn default_interval() -> i64 {
    3600
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/aggregate",
    tag = "Analytics",
    params(AggregateQuery),
    responses(
        (status = 200, description = "Aggregated metrics", body = Vec<(i64, analytics::AggregatedMetric)>),
        (status = 403, description = "Requires Starter tier"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn get_aggregate(
    State(state): State<AppState>,
    Query(q): Query<AggregateQuery>,
) -> Result<Json<Vec<(i64, analytics::AggregatedMetric)>>, AppError> {
    let results = analytics::aggregate_metrics(
        &state.storage,
        &q.device_id,
        &q.metric,
        q.from,
        q.to,
        q.interval,
    )
    .await?;

    Ok(Json(results))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct CompareQuery {
    pub device_id: String,
    pub metric: String,
    pub period1_from: i64,
    pub period1_to: i64,
    pub period2_from: i64,
    pub period2_to: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/compare",
    tag = "Analytics",
    params(CompareQuery),
    responses(
        (status = 200, description = "Period comparison", body = analytics::ComparisonResult),
        (status = 403, description = "Requires Starter tier"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn get_compare(
    State(state): State<AppState>,
    Query(q): Query<CompareQuery>,
) -> Result<Json<analytics::ComparisonResult>, AppError> {
    let result = analytics::compare_periods(
        &state.storage,
        &q.device_id,
        &q.metric,
        (q.period1_from, q.period1_to),
        (q.period2_from, q.period2_to),
    )
    .await?;

    Ok(Json(result))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SpcQuery {
    pub device_id: String,
    pub metric: String,
    pub from: i64,
    pub to: i64,
    #[serde(default = "default_subgroup")]
    pub subgroup_size: usize,
    #[serde(default)]
    pub chart_type: Option<String>,
}

fn default_subgroup() -> usize {
    5
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/spc",
    tag = "Analytics",
    params(SpcQuery),
    responses(
        (status = 200, description = "SPC chart data"),
        (status = 403, description = "Requires Starter tier"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn get_spc(
    State(state): State<AppState>,
    Query(q): Query<SpcQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let rows = state
        .storage
        .get_metric_values_for_range(&q.device_id, &q.metric, q.from, q.to)
        .await?;

    let values: Vec<f64> = rows.iter().map(|(_, v)| *v).collect();
    let timestamps: Vec<i64> = rows.iter().map(|(t, _)| *t).collect();

    if values.is_empty() {
        return Ok(Json(serde_json::json!({"error": "no data"})));
    }

    let chart_type = q.chart_type.as_deref().unwrap_or("xbar_r");
    let result = match chart_type {
        "cusum" => {
            let result =
                analytics::spc::spc_calculations::cusum(&values, 0.0, 0.5, 4.0, &timestamps)
                    .map_err(|e| AppError(anyhow::anyhow!(e)))?;
            serde_json::to_value(result).unwrap_or_default()
        }
        "ewma" => {
            let result = analytics::spc::spc_calculations::ewma(&values, 0.0, 0.2, &timestamps)
                .map_err(|e| AppError(anyhow::anyhow!(e)))?;
            serde_json::to_value(result).unwrap_or_default()
        }
        _ => {
            let (xbar, r) =
                analytics::spc::spc_calculations::xbar_r(&values, q.subgroup_size, &timestamps)
                    .map_err(|e| AppError(anyhow::anyhow!(e)))?;
            serde_json::json!({"xbar": xbar, "r": r})
        }
    };

    Ok(Json(result))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct HistogramQuery {
    pub device_id: String,
    pub metric: String,
    pub from: i64,
    pub to: i64,
    #[serde(default = "default_bins")]
    pub bins: u32,
    pub lsl: Option<f64>,
    pub usl: Option<f64>,
}

fn default_bins() -> u32 {
    20
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/histogram",
    tag = "Analytics",
    params(HistogramQuery),
    responses(
        (status = 200, description = "Histogram data"),
        (status = 403, description = "Requires Starter tier"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn get_histogram(
    State(state): State<AppState>,
    Query(q): Query<HistogramQuery>,
) -> Result<Json<crate::models::HistogramResult>, AppError> {
    let rows = state
        .storage
        .get_metric_values_for_range(&q.device_id, &q.metric, q.from, q.to)
        .await?;

    let values: Vec<f64> = rows.iter().map(|(_, v)| *v).collect();

    let result = analytics::histogram::compute_histogram(&values, q.bins, q.lsl, q.usl)
        .map_err(|e| AppError(anyhow::anyhow!(e)))?;

    Ok(Json(result))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct RunChartQuery {
    pub device_id: String,
    pub metric: String,
    pub from: i64,
    pub to: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/run-chart",
    tag = "Analytics",
    params(RunChartQuery),
    responses(
        (status = 200, description = "Run chart data"),
        (status = 403, description = "Requires Starter tier"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn get_run_chart(
    State(state): State<AppState>,
    Query(q): Query<RunChartQuery>,
) -> Result<Json<crate::models::RunChartResult>, AppError> {
    let rows = state
        .storage
        .get_metric_values_for_range(&q.device_id, &q.metric, q.from, q.to)
        .await?;

    let values: Vec<f64> = rows.iter().map(|(_, v)| *v).collect();
    let timestamps: Vec<i64> = rows.iter().map(|(t, _)| *t).collect();

    let result = analytics::runchart::compute_run_chart(&values, &timestamps)
        .map_err(|e| AppError(anyhow::anyhow!(e)))?;

    Ok(Json(result))
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/custom-metrics",
    tag = "Analytics",
    responses(
        (status = 200, description = "List custom metrics", body = Vec<CustomMetric>),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn list_custom_metrics(
    State(state): State<AppState>,
) -> Result<Json<Vec<CustomMetric>>, AppError> {
    let metrics = state.storage.list_custom_metrics().await?;
    Ok(Json(metrics))
}

#[utoipa::path(
    post,
    path = "/api/v1/analytics/custom-metrics",
    tag = "Analytics",
    request_body = CreateCustomMetricRequest,
    responses(
        (status = 200, description = "Created custom metric", body = CustomMetric),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn create_custom_metric(
    State(state): State<AppState>,
    Json(body): Json<CreateCustomMetricRequest>,
) -> Result<Json<CustomMetric>, AppError> {
    let tag_refs = custom::extract_tag_refs(&body.formula);
    let tag_refs_json = serde_json::to_string(&tag_refs).unwrap_or_else(|_| "[]".to_string());

    let validation_error = custom::validate_formula(&body.formula).err();

    let metric = state
        .storage
        .create_custom_metric(
            &body.name,
            &body.description,
            &body.formula,
            body.unit.as_deref(),
            &tag_refs_json,
            validation_error.as_deref(),
        )
        .await?;

    Ok(Json(metric))
}

#[utoipa::path(
    put,
    path = "/api/v1/analytics/custom-metrics/{id}",
    tag = "Analytics",
    request_body = UpdateCustomMetricRequest,
    responses(
        (status = 200, description = "Updated custom metric"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn update_custom_metric(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateCustomMetricRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // If formula changed, revalidate
    let (tag_refs, validation_error) = if let Some(ref formula) = body.formula {
        let refs = custom::extract_tag_refs(formula);
        let refs_json = serde_json::to_string(&refs).unwrap_or_else(|_| "[]".to_string());
        let err = custom::validate_formula(formula).err();
        (Some(refs_json), err)
    } else {
        (None, None)
    };

    state
        .storage
        .update_custom_metric(
            id,
            body.name.as_deref(),
            body.description.as_deref(),
            body.formula.as_deref(),
            body.unit.as_ref().map(|o| o.as_deref()),
            tag_refs.as_deref(),
            Some(validation_error.as_deref()),
        )
        .await?;

    Ok(Json(serde_json::json!({"ok": true})))
}

#[utoipa::path(
    delete,
    path = "/api/v1/analytics/custom-metrics/{id}",
    tag = "Analytics",
    responses(
        (status = 200, description = "Deleted custom metric"),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn delete_custom_metric(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.storage.delete_custom_metric(id).await?;
    Ok(Json(serde_json::json!({"ok": true})))
}

#[utoipa::path(
    get,
    path = "/api/v1/analytics/custom-metrics/evaluate",
    tag = "Analytics",
    responses(
        (status = 200, description = "Evaluated all custom metrics", body = Vec<CustomMetricValue>),
    ),
    security(("cookie_auth" = [])),
)]
pub async fn evaluate_all_custom_metrics(
    State(state): State<AppState>,
) -> Result<Json<Vec<CustomMetricValue>>, AppError> {
    let results = custom::evaluate_all_metrics(&state.storage, &state.cache).await?;
    Ok(Json(results))
}
