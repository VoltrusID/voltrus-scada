//! SCADA-17 S1, S2: refusals and missing records answered 500.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn demo_app() -> (Router, String) {
    let state = seeded_admin_app_state().await;
    let hash = crate::api::auth::hash_password("pw");
    state
        .storage
        .create_user("ops", &hash, "", "demo")
        .await
        .unwrap();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", login_cookie(&app, "ops", "pw").await);
    (app, cookie)
}

/// S1: the OPC UA, S7, EtherNet/IP, DNP3, BACnet and Voltrus-PLC handlers
/// check for the admin role themselves; a demo user passes the router and the
/// refusal was a 500 `anyhow` error instead of a 403.
#[tokio::test]
async fn test_protocol_handlers_answer_403_to_a_non_admin() {
    let (app, demo) = demo_app().await;

    for path in [
        "/api/v1/opcua/devices",
        "/api/v1/s7/devices",
        "/api/v1/enip/devices",
        "/api/v1/dnp3/devices",
        "/api/v1/bacnet/devices",
        "/api/v1/voltrus-plc/devices",
    ] {
        let (status, body) = send_request(&app, "GET", path, None, Some(&demo), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "GET {path}: {body}");
        assert_eq!(body["error"], "Admin access required", "GET {path}");
    }

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/bacnet/discover",
        None,
        Some(&demo),
        Some(json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "POST bacnet/discover: {body}"
    );
}

/// S2: a command id that does not exist answered 500 "Command not found".
/// Cancelling a command that was already sent keeps its 400.
#[tokio::test]
async fn test_unknown_command_answers_404() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, body) = send_request(
        &app,
        "GET",
        "/api/v1/commands/no-such-command",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "Command not found");

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/commands/no-such-command/cancel",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "Command not found");
}
