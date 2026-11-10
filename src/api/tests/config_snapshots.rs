use super::*;
use http::StatusCode;

/// SCADA-15 S11: rollback used to `DELETE FROM alarms` and restore the
/// snapshot's rows, losing every alarm raised since the snapshot.
#[tokio::test]
async fn test_rollback_keeps_alarm_history() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let older = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "Before snapshot", 0.0)
        .await
        .unwrap();
    let (status, snap) = send_request(
        &app,
        "POST",
        "/api/v1/config/snapshots",
        None,
        Some(&cookie),
        Some(serde_json::json!({"label": "baseline"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let snap_id = snap["id"].as_i64().unwrap();
    storage.acknowledge_alarm(older, "admin", 0).await.unwrap();
    let newer = storage
        .create_alarm("tag2", "critical", 95.0, 90.0, "After snapshot", 0.0)
        .await
        .unwrap();

    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/config/snapshots/{snap_id}/rollback"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let alarms = storage.list_alarms(None, 100).await.unwrap();
    let ids: Vec<i64> = alarms.iter().map(|a| a.id).collect();
    assert!(
        ids.contains(&older) && ids.contains(&newer),
        "alarms after rollback: {ids:?}"
    );
    let older_row = alarms.iter().find(|a| a.id == older).unwrap();
    assert!(older_row.acknowledged_at.is_some(), "acknowledgement kept");
}

#[tokio::test]
async fn test_delete_snapshot() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (_, snap) = send_request(
        &app,
        "POST",
        "/api/v1/config/snapshots",
        None,
        Some(&cookie),
        Some(serde_json::json!({"label": "to delete"})),
    )
    .await;
    let uri = format!("/api/v1/config/snapshots/{}", snap["id"].as_i64().unwrap());

    let (status, _) = send_request(&app, "DELETE", &uri, None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send_request(&app, "GET", &uri, None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send_request(&app, "DELETE", &uri, None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// `recipes.values` is an SQL keyword: a snapshot holding a recipe must still
/// roll back, and bring the recipe back as it was.
#[tokio::test]
async fn test_rollback_restores_recipes() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, recipe) = send_request(
        &app,
        "POST",
        "/api/v1/recipes",
        None,
        Some(&cookie),
        Some(serde_json::json!({
            "name": "summer", "device_id": "plc-1", "values": "{\"sp\": 42.0}",
        })),
    )
    .await;
    assert!(status.is_success(), "{status} {recipe}");
    let (status, snap) = send_request(
        &app,
        "POST",
        "/api/v1/config/snapshots",
        None,
        Some(&cookie),
        Some(serde_json::json!({"label": "with a recipe"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let snap_id = snap["id"].as_i64().unwrap();

    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/config/snapshots/{snap_id}/rollback"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, listed) = send_request(&app, "GET", "/api/v1/recipes", None, Some(&cookie), None).await;
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["name"], "summer");
    assert_eq!(listed[0]["values"], recipe["values"]);
}
