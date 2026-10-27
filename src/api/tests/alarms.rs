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
