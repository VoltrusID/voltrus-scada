use crate::storage::Storage;

pub async fn log_run(
    storage: &Storage,
    flow_id: &str,
    duration_ms: u64,
    success: bool,
    error: Option<String>,
    output: Option<serde_json::Value>,
) {
    if let Err(e) = storage
        .insert_flow_run(flow_id, duration_ms, success, error, output)
        .await
    {
        tracing::warn!("Failed to log flow run: {}", e);
    }
}
