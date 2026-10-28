use super::*;
use http::StatusCode;
use serde_json::json;

async fn admin_app() -> (Router, String, AppState) {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie, state)
}

async fn sim_state(app: &Router, cookie: &str, id: &str) -> serde_json::Value {
    let (status, body) = send_request(
        app,
        "GET",
        &format!("/api/v1/devices/{id}/simulate"),
        None,
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

/// Put `plc-1` into simulation with a custom config, as the UI does before go-live.
async fn simulated_device(app: &Router, cookie: &str, storage: &Storage) {
    storage.create_device("plc-1", "PLC 1", 0).await.unwrap();
    let (status, _) = send_request(
        app,
        "POST",
        "/api/v1/devices/plc-1/simulate",
        None,
        Some(cookie),
        Some(json!({"simulation_mode": true, "sim_config": "{\"power\":5}"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn rollback_restores_snapshot_and_returns_to_simulation() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;

    let (status, live) =
        send_request(&app, "POST", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None)
            .await;
    assert_eq!(status, StatusCode::OK, "{live}");
    assert_eq!(live["auto_rollback"], false);
    assert_eq!(sim_state(&app, &cookie, "plc-1").await["simulation_mode"], false);

    // A config edit made while live is part of what the rollback discards.
    state.storage.create_device("plc-2", "Added live", 1).await.unwrap();

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/devices/plc-1/go-live/rollback",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let sim = sim_state(&app, &cookie, "plc-1").await;
    assert_eq!(sim["simulation_mode"], true);
    assert_eq!(sim["sim_config"], "{\"power\":5}");
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/devices/plc-2/simulate",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "edit after go-live was not reverted");

    // The record is consumed: nothing left to roll back, status is inactive.
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/devices/plc-1/go-live/rollback",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, st) = send_request(&app, "GET", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None).await;
    assert_eq!(st["active"], false);

    // The pre-rollback state stays recoverable.
    let snaps = state.storage.list_config_snapshots(50).await.unwrap();
    assert!(snaps.iter().any(|s| s.trigger == "restore"));
}

#[tokio::test]
async fn rollback_without_go_live_and_go_live_unknown_device_are_404() {
    let (app, cookie, state) = admin_app().await;
    state.storage.create_device("plc-1", "PLC 1", 0).await.unwrap();

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/devices/plc-1/go-live/rollback",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) =
        send_request(&app, "POST", "/api/v1/devices/nope/go-live", None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A refused go-live must not leave a snapshot behind or a record.
    assert!(state.storage.list_config_snapshots(10).await.unwrap().is_empty());
}

#[tokio::test]
async fn rollback_with_deleted_snapshot_is_410_and_changes_nothing() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;
    let (_, live) =
        send_request(&app, "POST", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None)
            .await;
    let snap = live["snapshot_id"].as_i64().unwrap();
    assert!(state.storage.delete_config_snapshot(snap).await.unwrap());

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/devices/plc-1/go-live/rollback",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(sim_state(&app, &cookie, "plc-1").await["simulation_mode"], false);
}

#[tokio::test]
async fn status_reports_verifying_connected_and_failed() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;
    let status_uri = "/api/v1/devices/plc-1/go-live";

    let (_, st) = send_request(&app, "GET", status_uri, None, Some(&cookie), None).await;
    assert_eq!(st["active"], false);

    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/settings/go-live",
        None,
        Some(&cookie),
        Some(json!({"grace_secs": 30, "auto_rollback": false})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    send_request(&app, "POST", status_uri, None, Some(&cookie), None).await;

    let (_, st) = send_request(&app, "GET", status_uri, None, Some(&cookie), None).await;
    assert_eq!(st["state"], "verifying");
    assert_eq!(st["grace_secs"], 30);

    state.cache.update("plc-1", Default::default(), false).await;
    let (_, st) = send_request(&app, "GET", status_uri, None, Some(&cookie), None).await;
    assert_eq!(st["state"], "connected");

    // Grace window elapsed with the device still offline.
    state.cache.set_status("plc-1", crate::models::DeviceStatus::Offline).await;
    let record = json!({
        "snapshot_id": st["snapshot_id"],
        "went_live_at": crate::timeutil::now_secs() - 120,
        "grace_secs": 30,
        "auto_rollback": false,
    });
    state
        .storage
        .upsert_setting("go_live:plc-1", &record.to_string())
        .await
        .unwrap();
    let (_, st) = send_request(&app, "GET", status_uri, None, Some(&cookie), None).await;
    assert_eq!(st["state"], "failed");
    assert_eq!(st["auto_rollback"], false);
}

#[tokio::test]
async fn go_live_settings_default_off_and_clamp() {
    let (app, cookie, _) = admin_app().await;
    let (_, s) = send_request(&app, "GET", "/api/v1/settings/go-live", None, Some(&cookie), None).await;
    assert_eq!(s, json!({"grace_secs": 60, "auto_rollback": false}));
    let (_, s) = send_request(
        &app,
        "PUT",
        "/api/v1/settings/go-live",
        None,
        Some(&cookie),
        Some(json!({"grace_secs": 1, "auto_rollback": true})),
    )
    .await;
    assert_eq!(s, json!({"grace_secs": 5, "auto_rollback": true}));
}

#[tokio::test]
async fn auto_rollback_fires_only_when_enabled_and_device_never_connects() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;

    send_request(
        &app,
        "PUT",
        "/api/v1/settings/go-live",
        None,
        Some(&cookie),
        Some(json!({"grace_secs": 5, "auto_rollback": true})),
    )
    .await;
    send_request(&app, "POST", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None).await;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if sim_state(&app, &cookie, "plc-1").await["simulation_mode"] == true {
            return;
        }
    }
    panic!("device was not rolled back to simulation");
}

#[tokio::test]
async fn offline_device_stays_live_when_auto_rollback_is_off() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;
    send_request(
        &app,
        "PUT",
        "/api/v1/settings/go-live",
        None,
        Some(&cookie),
        Some(json!({"grace_secs": 5, "auto_rollback": false})),
    )
    .await;
    send_request(&app, "POST", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None).await;
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    assert_eq!(sim_state(&app, &cookie, "plc-1").await["simulation_mode"], false);
}

#[tokio::test]
async fn auto_rollback_spares_a_device_that_connected() {
    let (app, cookie, state) = admin_app().await;
    simulated_device(&app, &cookie, &state.storage).await;
    send_request(
        &app,
        "PUT",
        "/api/v1/settings/go-live",
        None,
        Some(&cookie),
        Some(json!({"grace_secs": 5, "auto_rollback": true})),
    )
    .await;
    send_request(&app, "POST", "/api/v1/devices/plc-1/go-live", None, Some(&cookie), None).await;
    state.cache.update("plc-1", Default::default(), false).await;
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    assert_eq!(sim_state(&app, &cookie, "plc-1").await["simulation_mode"], false);
}
