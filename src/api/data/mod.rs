mod evaluate;
mod handlers;
mod helpers;

#[cfg(test)]
mod tests;

use axum::Json;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState, generators};
use crate::expression;
use crate::models::*;

/// Metric name for a data source key.
///
/// Keys may be device-qualified (`plts_1.voltage`) to keep WS tag ids unique
/// across devices, but cache/telemetry metrics are stored under the bare
/// register name, scoped by `device_id`. Strip the device prefix when
/// present; bare keys pass through unchanged.
pub(crate) fn metric_name<'a>(key: &'a str, device_id: &str) -> &'a str {
    let prefix = format!("{device_id}.");
    key.strip_prefix(&prefix).unwrap_or(key)
}

pub(crate) use evaluate::*;
pub use handlers::*;
