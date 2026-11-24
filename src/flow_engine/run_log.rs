use crate::storage::Storage;

pub async fn log_run(
    storage: &Storage,
    flow_id: &str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    output: Option<serde_json::Value>,
) {
    log_run_traced(storage, flow_id, duration_ms, success, error, output, None).await;
}

/// Record a run with its per-node trace (see `executor::truncate_trace`).
pub async fn log_run_traced(
    storage: &Storage,
    flow_id: &str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    output: Option<serde_json::Value>,
    trace: Option<serde_json::Value>,
) {
    if let Err(e) = storage
        .insert_flow_run_traced(flow_id, duration_ms, success, error, output, trace)
        .await
    {
        tracing::warn!("Failed to log flow run: {}", e);
    }
}
