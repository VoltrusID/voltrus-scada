use super::*;
use crate::ws_manager::WsFrame;
use crate::ws_types::ServerMessage;
use http::StatusCode;
use tokio::sync::mpsc::Receiver;

/// Every `AlarmUpdate` queued on the connection so far, as (alarm_id, event, state, actor).
fn updates(rx: &mut Receiver<WsFrame>) -> Vec<(i64, String, String, Option<String>)> {
    let mut out = Vec::new();
    while let Ok(frame) = rx.try_recv() {
        let WsFrame::Binary(bytes) = frame else {
            continue;
        };
        if let ServerMessage::AlarmUpdate {
            alarm_id,
            event,
            state,
            actor,
            ..
        } = rmp_serde::from_slice(&bytes).unwrap()
        {
            out.push((alarm_id, event, state, actor));
        }
    }
    out
}

/// SCADA-17 S9: acknowledge, shelve and unshelve changed the alarm without a
/// WebSocket message; only the raise was pushed.
#[tokio::test]
async fn alarm_changes_are_pushed_over_the_websocket() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let mut rx = state
        .ws_manager
        .connect("c1".to_string(), "admin".to_string(), vec![]);
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let a = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    let b = storage
        .create_alarm("tag2", "critical", 95.0, 90.0, "High", 0.0)
        .await
        .unwrap();

    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/alarms/{a}/acknowledge"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        updates(&mut rx),
        [(
            a,
            "acknowledged".to_string(),
            "acknowledged".to_string(),
            Some("admin".to_string())
        )]
    );

    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/alarms/{b}/shelf"),
        None,
        Some(&cookie),
        Some(serde_json::json!({"duration_secs": 600, "reason": "Sensor swap"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        updates(&mut rx),
        [(
            b,
            "shelved".to_string(),
            "shelved".to_string(),
            Some("admin".to_string())
        )]
    );

    let (status, _) = send_request(
        &app,
        "POST",
        &format!("/api/v1/alarms/{b}/unshelf"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        updates(&mut rx),
        [(b, "unshelved".to_string(), "active".to_string(), None)]
    );
}

/// Acknowledge-all pushes one update per alarm it acknowledged.
#[tokio::test]
async fn acknowledge_all_pushes_each_alarm() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let mut rx = state
        .ws_manager
        .connect("c1".to_string(), "admin".to_string(), vec![]);
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let a = storage
        .create_alarm("tag1", "warning", 75.0, 70.0, "High", 0.0)
        .await
        .unwrap();
    let b = storage
        .create_alarm("tag2", "info", 5.0, 1.0, "High", 0.0)
        .await
        .unwrap();

    let (status, _) = send_request(&app, "POST", "/api/v1/alarms", None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    let mut pushed: Vec<i64> = updates(&mut rx).into_iter().map(|u| u.0).collect();
    pushed.sort_unstable();
    assert_eq!(pushed, [a, b]);
}

/// An unknown alarm id changes nothing, so nothing is pushed.
#[tokio::test]
async fn an_unknown_alarm_pushes_nothing() {
    let state = seeded_admin_app_state().await;
    let mut rx = state
        .ws_manager
        .connect("c1".to_string(), "admin".to_string(), vec![]);
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/alarms/999/acknowledge",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(updates(&mut rx).is_empty());
}

/// A tag whose alarm was raised six times in an hour is reported as chattering
/// in alarm analytics, and the tag editor's suggestion asks for a longer
/// on-delay.
#[tokio::test]
async fn chattering_alarms_are_counted_and_get_a_longer_on_delay() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(serde_json::json!({"key": "turbidity", "name": "Turbidity", "min_val": 0, "max_val": 50, "warn_val": 18})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().unwrap();

    // No history and no alarms: the analog default.
    let (status, s) = send_request(
        &app,
        "GET",
        &format!("/api/v1/data-sources/{id}/alarm-suggestion"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{s}");
    assert_eq!(
        s["on_delay_secs"],
        crate::alarm_chatter::DEFAULT_ANALOG_ON_DELAY_SECS
    );
    assert_eq!(s["chattering"], false);

    for _ in 0..6 {
        storage
            .create_alarm("turbidity", "warning", 18.2, 18.0, "high", 0.5)
            .await
            .unwrap();
    }

    let (_, s) = send_request(
        &app,
        "GET",
        &format!("/api/v1/data-sources/{id}/alarm-suggestion"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(s["alarms_last_hour"], 6, "{s}");
    assert_eq!(s["max_alarms_per_hour"], 6);
    assert_eq!(s["chattering"], true);
    assert_eq!(
        s["on_delay_secs"],
        crate::alarm_chatter::CHATTERING_ON_DELAY_SECS
    );

    let (status, list) = send_request(
        &app,
        "GET",
        "/api/v1/alarms/analytics",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["source_key"] == "turbidity")
        .unwrap();
    assert_eq!(row["trigger_count"], 6);
    assert_eq!(row["max_per_hour"], 6);
    assert_eq!(row["chattering"], true);

    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/data-sources/99999/alarm-suggestion",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
