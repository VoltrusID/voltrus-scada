#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Analytics types ──

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ControlLimits {
    pub ucl: f64,
    pub cl: f64,
    pub lcl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SpcPoint {
    pub index: usize,
    pub value: f64,
    pub timestamp: i64,
    pub subgroup_values: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SpcResult {
    pub chart_type: String,
    pub data_points: Vec<SpcPoint>,
    pub control_limits: ControlLimits,
    pub violations: Vec<usize>,
    pub in_control_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HistogramBin {
    pub lower: f64,
    pub upper: f64,
    pub count: u64,
    pub frequency: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CurvePoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct DistributionStats {
    pub mean: f64,
    pub stddev: f64,
    pub skewness: f64,
    pub kurtosis: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CapabilityIndices {
    pub cp: f64,
    pub cpk: f64,
    pub pp: f64,
    pub ppk: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct HistogramResult {
    pub bins: Vec<HistogramBin>,
    pub normal_curve: Vec<CurvePoint>,
    pub stats: DistributionStats,
    pub capability: Option<CapabilityIndices>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunChartPoint {
    pub value: f64,
    pub timestamp: i64,
    pub above_median: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Signal {
    pub rule: String,
    pub start_idx: usize,
    pub end_idx: usize,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RunChartResult {
    pub points: Vec<RunChartPoint>,
    pub median: f64,
    pub signals: Vec<Signal>,
    pub runs_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CustomMetric {
    pub id: i64,
    pub name: String,
    pub description: String,
    pub formula: String,
    pub unit: Option<String>,
    pub tag_refs: String,
    pub validation_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CustomMetricValue {
    pub id: i64,
    pub name: String,
    pub value: f64,
    pub unit: Option<String>,
    pub computed_at: i64,
    pub tag_values: HashMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct CreateCustomMetricRequest {
    pub name: String,
    pub description: String,
    pub formula: String,
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct UpdateCustomMetricRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub formula: Option<String>,
    pub unit: Option<Option<String>>,
}
