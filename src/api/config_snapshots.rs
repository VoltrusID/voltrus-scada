use crate::api::*;
use crate::models::*;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

pub async fn list_snapshots(
    state: State<AppState>,
) -> Result<Json<Vec<ConfigSnapshot>>, StatusCode> {
    state
        .storage
        .list_config_snapshots(50)
        .await
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

pub async fn get_snapshot(
    state: State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ConfigSnapshot>, StatusCode> {
    state
        .storage
        .get_config_snapshot(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn create_snapshot(
    state: State<AppState>,
    Json(req): Json<CreateSnapshotRequest>,
) -> Result<(StatusCode, Json<ConfigSnapshot>), StatusCode> {
    let id = state
        .storage
        .create_config_snapshot(
            req.label.as_deref(),
            Some("Manual snapshot"),
            "manual",
            None,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let snapshot = state
        .storage
        .get_config_snapshot(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::CREATED, Json(snapshot)))
}

pub async fn diff_snapshots(
    state: State<AppState>,
    Path((from_id, to_id)): Path<(i64, i64)>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let from = state
        .storage
        .get_config_snapshot(from_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let to = state
        .storage
        .get_config_snapshot(to_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;

    let from_data: serde_json::Value =
        serde_json::from_str(&from.snapshot_data).unwrap_or_default();
    let to_data: serde_json::Value = serde_json::from_str(&to.snapshot_data).unwrap_or_default();

    let diff = compute_diff(&from_data, &to_data);
    Ok(Json(diff))
}

pub async fn rollback(
    state: State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, StatusCode> {
    // Create a pre-rollback snapshot
    let _ = state
        .storage
        .create_config_snapshot(
            Some(&format!("pre-rollback: snapshot #{}", id)),
            Some(&format!(
                "Auto-snapshot before rollback to snapshot #{}",
                id
            )),
            "restore",
            None,
        )
        .await;

    let rolled_back = state
        .storage
        .rollback_to_snapshot(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if rolled_back {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

pub fn compute_diff_public(from: &serde_json::Value, to: &serde_json::Value) -> serde_json::Value {
    compute_diff(from, to)
}

fn compute_diff(from: &serde_json::Value, to: &serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    let from_map = match from.as_object() {
        Some(m) => m,
        None => return serde_json::json!({}),
    };
    let to_map = match to.as_object() {
        Some(m) => m,
        None => return serde_json::json!({}),
    };

    for table in from_map.keys().chain(to_map.keys()) {
        let from_rows = from_map
            .get(table)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let to_rows = to_map
            .get(table)
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let from_ids: Vec<String> = from_rows
            .iter()
            .filter_map(|r| r.get("id").map(|v| v.to_string()))
            .collect();
        let to_ids: Vec<String> = to_rows
            .iter()
            .filter_map(|r| r.get("id").map(|v| v.to_string()))
            .collect();

        let added: Vec<_> = to_rows
            .iter()
            .filter(|r| {
                r.get("id")
                    .map(|v| !from_ids.contains(&v.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let removed: Vec<_> = from_rows
            .iter()
            .filter(|r| {
                r.get("id")
                    .map(|v| !to_ids.contains(&v.to_string()))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let changed: Vec<_> = to_rows
            .iter()
            .filter_map(|to_r| {
                let id = to_r.get("id")?.to_string();
                let from_r = from_rows
                    .iter()
                    .find(|r| r.get("id").map(|v| *v == id).unwrap_or(false))?;
                if from_r != to_r {
                    Some(serde_json::json!({"id": id, "from": from_r, "to": to_r}))
                } else {
                    None
                }
            })
            .collect();

        if !added.is_empty() || !removed.is_empty() || !changed.is_empty() {
            result.insert(
                table.clone(),
                serde_json::json!({
                    "added": added,
                    "removed": removed,
                    "changed": changed,
                }),
            );
        }
    }
    serde_json::Value::Object(result)
}
