//! A session whose user must change their password (a generated or reset
//! password, or one the edition reported expired) may only change it and
//! sign out: everything else answers 403 `password_change_required`.

use super::*;
use crate::api::shared_accounts::PublicDemo;
use crate::ws_types::{ClientMessage, ServerMessage};
use http::StatusCode;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PW: &str = "first-password-1";
const NEW_PW: &str = "second-password-2";

async fn flagged_user(state: &AppState, username: &str, role: &str) {
    state
        .storage
        .create_user(username, &crate::api::auth::hash_password(PW), "", role)
        .await
        .unwrap();
    state
        .storage
        .set_must_change_password(username, true)
        .await
        .unwrap();
}

async fn cookie(app: &Router, username: &str, password: &str) -> String {
    format!(
        "voltrus_token={}",
        login_cookie(app, username, password).await
    )
}

async fn get(app: &Router, uri: &str, cookie: &str) -> (StatusCode, Value) {
    send_request(app, "GET", uri, None, Some(cookie), None).await
}

fn assert_gated(status: StatusCode, body: &Value) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["code"], "password_change_required", "{body}");
    assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
}

/// Serve the router on a local port.
async fn serve(state: AppState) -> std::net::SocketAddr {
    let app = create_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    addr
}

/// Open `/api/v1/ws` with an optional cookie. Returns the response status
/// and, for a 101, the open stream.
async fn ws_upgrade(
    addr: std::net::SocketAddr,
    cookie: Option<&str>,
) -> (u16, tokio::net::TcpStream) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "GET /api/v1/ws HTTP/1.1\r\nHost: {addr}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
    );
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, stream)
}

/// Send one masked binary frame.
async fn ws_send(stream: &mut tokio::net::TcpStream, message: &ClientMessage) {
    let payload = rmp_serde::to_vec_named(message).unwrap();
    let mut frame = vec![0x82_u8];
    if payload.len() < 126 {
        frame.push(0x80 | payload.len() as u8);
    } else {
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    let mask = [1_u8, 2, 3, 4];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    stream.write_all(&frame).await.unwrap();
}

/// Read the first binary frame the server sends.
async fn ws_receive(stream: &mut tokio::net::TcpStream) -> ServerMessage {
    loop {
        let mut head = [0_u8; 2];
        stream.read_exact(&mut head).await.unwrap();
        let len = match head[1] & 0x7f {
            126 => {
                let mut ext = [0_u8; 2];
                stream.read_exact(&mut ext).await.unwrap();
                u16::from_be_bytes(ext) as usize
            }
            127 => {
                let mut ext = [0_u8; 8];
                stream.read_exact(&mut ext).await.unwrap();
                u64::from_be_bytes(ext) as usize
            }
            n => n as usize,
        };
        let mut payload = vec![0_u8; len];
        stream.read_exact(&mut payload).await.unwrap();
        if head[0] & 0x0f == 0x2 {
            return rmp_serde::from_slice(&payload).unwrap();
        }
    }
}

/// Authenticate a socket with an `Auth` message (the native console's way).
async fn ws_auth_message(addr: std::net::SocketAddr, token: &str) -> ServerMessage {
    let (status, mut stream) = ws_upgrade(addr, None).await;
    assert_eq!(status, 101);
    ws_send(
        &mut stream,
        &ClientMessage::Auth {
            token: token.to_string(),
        },
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), ws_receive(&mut stream))
        .await
        .expect("an answer to Auth")
}

#[tokio::test]
async fn a_session_that_must_change_its_password_may_only_change_it() {
    let state = seeded_admin_app_state().await;
    flagged_user(&state, "ops", "operator").await;
    let app = create_router(state.clone());
    let addr = serve(state.clone()).await;

    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/auth/login",
        None,
        None,
        Some(json!({"username": "ops", "password": PW})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["must_change_password"], true);
    let ops = cookie(&app, "ops", PW).await;
    let token = ops.trim_start_matches("voltrus_token=").to_string();

    // Normal calls of every tier, the SSE stream and MCP are refused.
    for uri in [
        "/api/v1/screens",
        "/api/v1/alarms/summary",
        "/api/v1/stream",
    ] {
        let (status, body) = get(&app, uri, &ops).await;
        assert_gated(status, &body);
    }
    let (status, body) = send_request(
        &app,
        "POST",
        "/mcp",
        None,
        Some(&ops),
        Some(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})),
    )
    .await;
    assert_gated(status, &body);

    // The WebSocket: refused at the upgrade with the cookie, and with an
    // AuthError when the token comes in the Auth message.
    let (status, _) = ws_upgrade(addr, Some(&ops)).await;
    assert_eq!(status, 403);
    match ws_auth_message(addr, &token).await {
        ServerMessage::AuthError { message } => {
            assert!(message.starts_with("password_change_required"), "{message}")
        }
        other => panic!("expected AuthError, got {other:?}"),
    }

    // What the change-password screen needs still works.
    let (status, me) = get(&app, "/api/v1/auth/me", &ops).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["authenticated"], true);
    assert_eq!(me["must_change_password"], true);
    assert_eq!(me["password_expired"], false);
    let (status, _) = get(&app, "/api/v1/version", &ops).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/auth/change-password",
        None,
        Some(&ops),
        Some(json!({"old_password": PW, "new_password": NEW_PW})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The same session works now, the socket too.
    let (status, body) = get(&app, "/api/v1/screens", &ops).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, me) = get(&app, "/api/v1/auth/me", &ops).await;
    assert_eq!(me["must_change_password"], false);
    let (status, _) = ws_upgrade(addr, Some(&ops)).await;
    assert_eq!(status, 101);
    assert!(matches!(
        ws_auth_message(addr, &token).await,
        ServerMessage::AuthOk { .. }
    ));
}

#[tokio::test]
async fn a_gated_session_can_sign_out() {
    let state = seeded_admin_app_state().await;
    flagged_user(&state, "ops", "operator").await;
    let app = create_router(state);
    let ops = cookie(&app, "ops", PW).await;
    let (status, _) =
        send_request(&app, "POST", "/api/v1/auth/logout", None, Some(&ops), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_api_key_is_not_held_to_its_creators_password_change() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let admin = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, key) = send_request(
        &app,
        "POST",
        "/api/v1/api-keys",
        None,
        Some(&admin),
        Some(json!({"name": "scraper", "role": "api"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{key}");
    let key = key["key"].as_str().unwrap().to_string();

    state
        .storage
        .set_must_change_password("admin", true)
        .await
        .unwrap();
    let (status, body) = get(&app, "/api/v1/screens", &admin).await;
    assert_gated(status, &body);
    let (status, body) = send_request(&app, "GET", "/api/v1/screens", Some(&key), None, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn a_shared_demo_account_is_never_held() {
    let mut state = seeded_admin_app_state().await;
    flagged_user(&state, "guest", "viewer").await;
    state.public_demo = Arc::new(PublicDemo {
        shared_accounts: vec!["guest".to_string()],
        login_hint: None,
    });
    let app = create_router(state.clone());
    let addr = serve(state).await;
    let guest = cookie(&app, "guest", PW).await;

    let (status, body) = get(&app, "/api/v1/screens", &guest).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, me) = get(&app, "/api/v1/auth/me", &guest).await;
    assert_eq!(me["must_change_password"], false);
    let (status, _) = ws_upgrade(addr, Some(&guest)).await;
    assert_eq!(status, 101);
}
