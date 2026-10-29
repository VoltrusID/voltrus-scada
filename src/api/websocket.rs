use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use super::AppState;
use crate::ws_manager::WsFrame;
use crate::ws_types::{ClientMessage, ServerMessage};

/// GET /api/v1/ws -- WebSocket upgrade endpoint.
///
/// The client must send an `Auth` message (MessagePack-encoded `ClientMessage::Auth`)
/// within 30 seconds of connecting. After successful authentication the connection
/// is registered with `WsManager` and the client can subscribe to tags and issue
/// tag writes.
#[utoipa::path(
    get,
    path = "/api/v1/ws",
    tag = "Stream",
    responses(
        (status = 101, description = "Switching protocols: MessagePack frames in both directions"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    jar: axum_extra::extract::cookie::CookieJar,
) -> impl IntoResponse {
    let cookie_token = jar.get("voltrus_token").map(|c| c.value().to_string());
    ws.on_upgrade(move |socket| handle_socket(socket, state, cookie_token))
}

// -- Socket handler --------------------------------------------------------

async fn handle_socket(socket: WebSocket, state: AppState, cookie_token: Option<String>) {
    let (mut sink, mut stream) = socket.split();

    // 1. Obtain token: prefer cookie from upgrade request, fall back to Auth message.
    let token = if let Some(t) = cookie_token {
        t
    } else {
        // No cookie – wait for a MessagePack Auth message (non-browser clients).
        let first_msg: Message = match timeout(Duration::from_secs(30), stream.next()).await {
            Ok(Some(Ok(msg))) => msg,
            Ok(Some(Err(e))) => {
                log_receive_error("error receiving first message", &e);
                return;
            }
            Ok(None) | Err(_) => {
                tracing::info!("WS: client disconnected before authentication");
                return;
            }
        };

        let data = match first_msg.into_data() {
            d if !d.is_empty() => d,
            _ => {
                let _ = send_msg(
                    &mut sink,
                    &ServerMessage::AuthError {
                        message: "First message must be a binary Auth message".to_string(),
                    },
                )
                .await;
                return;
            }
        };

        let client_msg: ClientMessage = match rmp_serde::from_slice(&data) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("WS: failed to decode first message: {}", e);
                let _ = send_msg(
                    &mut sink,
                    &ServerMessage::AuthError {
                        message: format!("Invalid MessagePack: {}", e),
                    },
                )
                .await;
                return;
            }
        };

        match client_msg {
            ClientMessage::Auth { token } => token,
            _ => {
                let _ = send_msg(
                    &mut sink,
                    &ServerMessage::AuthError {
                        message: "First message must be Auth".to_string(),
                    },
                )
                .await;
                return;
            }
        }
    };

    // 3. Validate the token and re-read the user's current role.
    let claims = match crate::api::auth_middleware::verify_session(&state, &token).await {
        Some(c) => c,
        None => {
            let _ = send_msg(
                &mut sink,
                &ServerMessage::AuthError {
                    message: "Invalid or expired token".to_string(),
                },
            )
            .await;
            return;
        }
    };

    let username = claims.username.clone();
    let role = claims.role.clone();

    // 4. Determine permissions from role.
    let permissions = role_to_permissions(&role);

    // 5. Generate a unique connection ID.
    let conn_id = format!(
        "ws-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );

    tracing::info!("WS: user={} role={} conn={}", username, role, conn_id);

    // 6. Register with WsManager and get the receiver for outgoing messages.
    let mut rx = state
        .ws_manager
        .connect(conn_id.clone(), username.clone(), permissions.clone());

    // 7. Send AuthOk.
    if let Err(e) = send_msg(
        &mut sink,
        &ServerMessage::AuthOk {
            user: username.clone(),
            permissions: permissions.clone(),
        },
    )
    .await
    {
        tracing::warn!("WS: failed to send AuthOk: {}", e);
        state.ws_manager.disconnect(&conn_id);
        return;
    }

    // 8. Main loop: select between outgoing WsManager messages and
    //    incoming WebSocket frames from the client.
    let conn_id_loop = conn_id.clone();
    let ws_manager_loop = state.ws_manager.clone();
    let username_loop = username.clone();
    let role_loop = role.clone();
    let state_loop = state.clone();

    loop {
        tokio::select! {
            // Outgoing message from WsManager broadcast channel
            msg = rx.recv() => {
                match msg {
                    Some(frame) => {
                        let result = match frame {
                            WsFrame::Binary(bytes) => {
                                sink.send(Message::Binary(bytes)).await
                            }
                            WsFrame::Message(server_msg) => {
                                send_msg(&mut sink, &server_msg).await
                            }
                        };
                        if result.is_err() {
                            break;
                        }
                    }
                    None => {
                        // Channel closed -- manager dropped
                        break;
                    }
                }
            }
            // Incoming WebSocket frame from client
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Binary(data))) => {
                        let client_msg = match rmp_serde::from_slice::<ClientMessage>(&data) {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!("WS: decode error: {}", e);
                                continue;
                            }
                        };
                        handle_client_message(
                            &client_msg,
                            &conn_id_loop,
                            &ws_manager_loop,
                            &state_loop,
                            &username_loop,
                            &role_loop,
                        ).await;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sink.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        break;
                    }
                    Some(Ok(Message::Text(_))) => {
                        tracing::debug!(
                            "WS: ignoring text frame (expected binary MessagePack)"
                        );
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong responses are handled automatically by tungstenite
                    }
                    Some(Err(e)) => {
                        log_receive_error("recv error", &e);
                        break;
                    }
                }
            }
        }
    }

    ws_manager_loop.disconnect(&conn_id_loop);
    tracing::info!("WS: disconnected conn={}", conn_id);
}

// -- Client message dispatch -----------------------------------------------

async fn handle_client_message(
    msg: &ClientMessage,
    conn_id: &str,
    ws_manager: &Arc<crate::ws_manager::WsManager>,
    state: &AppState,
    username: &str,
    role: &str,
) {
    match msg {
        ClientMessage::Subscribe { tag_ids } => {
            ws_manager.subscribe(conn_id, tag_ids);
            tracing::debug!("WS: user={} subscribed to {} tags", username, tag_ids.len());
        }

        ClientMessage::Unsubscribe { tag_ids } => {
            ws_manager.unsubscribe(conn_id, tag_ids);
            tracing::debug!(
                "WS: user={} unsubscribed from {} tags",
                username,
                tag_ids.len()
            );
        }

        ClientMessage::TagWrite {
            tag_id,
            value,
            command_id,
            reason,
        } => {
            let cmd_id = command_id.clone().unwrap_or_else(next_command_id);

            // Permission check: only admins can write via WebSocket
            if role != "admin" {
                tracing::warn!(
                    "WS: user={} (role={}) denied tag write to {}",
                    username,
                    role,
                    tag_id
                );
                reject_write(ws_manager, conn_id, cmd_id, "Only admins can write tags");
                return;
            }

            handle_tag_write(
                state,
                conn_id,
                username,
                tag_id,
                *value,
                reason.clone(),
                cmd_id,
            )
            .await;
        }

        ClientMessage::Ping => {
            // Application-level ping -- WS protocol ping/pong handled separately.
        }

        ClientMessage::Auth { .. } => {
            tracing::debug!("WS: ignoring duplicate Auth message");
        }
    }
}

// -- Tag write handling: through the command queue -------------------------

/// A command id for a write whose client did not choose one.
fn next_command_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "ws-{}-{}",
        crate::timeutil::now_millis(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// Answer a refused write to the connection that asked for it.
fn reject_write(
    ws_manager: &crate::ws_manager::WsManager,
    conn_id: &str,
    command_id: String,
    reason: &str,
) {
    ws_manager.send_to(
        conn_id,
        ServerMessage::CommandAck {
            command_id,
            status: "rejected".to_string(),
            device_response: Some(reason.to_string()),
            latency_ms: None,
        },
    );
}

/// A write over the WebSocket takes the same road as `POST /tags/{id}/write`:
/// the tag's write policy is checked and the command is queued with an audit
/// entry naming the user. The queue processor delivers it and broadcasts the
/// `sent`, `acked` or `failed` acknowledgements under the same command id; the
/// requester is told right away only when the write is refused (`rejected`, with the
/// reason); otherwise its first `CommandAck` for the command id is the final outcome
/// (`acked`, `failed` or `expired`), the intermediate `sent` and `queued` going to the
/// other connections only.
async fn handle_tag_write(
    state: &AppState,
    conn_id: &str,
    username: &str,
    tag_id: &str,
    value: f64,
    reason: Option<String>,
    cmd_id: String,
) {
    if !crate::node_role::is_active() {
        reject_write(
            &state.ws_manager,
            conn_id,
            cmd_id,
            crate::node_role::NOT_ACTIVE_MESSAGE,
        );
        return;
    }
    let data_source_id: i64 = match tag_id.parse() {
        Ok(id) => id,
        Err(_) => {
            tracing::warn!("WS: tag_id '{}' is not numeric, skipping write", tag_id);
            reject_write(
                &state.ws_manager,
                conn_id,
                cmd_id,
                "Tag id must be the numeric data source id",
            );
            return;
        }
    };

    let submission = super::commands::TagWriteSubmission {
        tag_id: data_source_id,
        value,
        policy: "immediate".to_string(),
        scheduled_at: None,
        reason,
        command_id: Some(cmd_id.clone()),
    };
    // Registered before the command exists: the queue may answer within milliseconds.
    state.ws_manager.expect_final_ack(&cmd_id, conn_id);
    match super::commands::submit_tag_write(state, username, submission).await {
        Ok(cmd) => {
            // No answer yet: the requester's first `CommandAck` for this id is the
            // queue's final one (`acked`, `failed`, `expired`), as it was the device's
            // answer when writes went straight to the device.
            tracing::info!(
                "WS: user={} queued write {}={value} as command {}",
                username,
                data_source_id,
                cmd.id
            );
        }
        Err(refusal) => {
            state.ws_manager.forget_command(&cmd_id);
            let message = refusal.message();
            tracing::warn!(
                "WS: user={} write to data source {} refused: {}",
                username,
                data_source_id,
                message
            );
            reject_write(&state.ws_manager, conn_id, cmd_id, &message);
        }
    }
}

// -- Helpers ----------------------------------------------------------------

/// Map a user role to a list of permission strings.
pub(super) fn role_to_permissions(role: &str) -> Vec<String> {
    match role {
        "admin" => vec!["read".to_string(), "write".to_string(), "admin".to_string()],
        "api" => vec!["read".to_string(), "write".to_string()],
        _ => vec!["read".to_string()],
    }
}

/// Encode a `ServerMessage` as MessagePack and send it as a binary WS frame.
async fn send_msg(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMessage,
) -> Result<(), axum::Error> {
    let bytes = rmp_serde::to_vec_named(msg)
        .map_err(|e| axum::Error::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)))?;
    sink.send(Message::Binary(bytes.into())).await
}

/// A peer that vanished without saying goodbye: the TCP connection was reset or
/// broken, or closed without the WebSocket closing handshake. Browsers do this
/// whenever a tab is closed or a phone leaves the Wi-Fi, so it is not a fault.
/// Walks the error chain because the cause sits under axum's and tungstenite's
/// wrappers.
fn is_routine_disconnect(err: &(dyn std::error::Error + 'static)) -> bool {
    use std::io::ErrorKind;
    let mut current = Some(err);
    while let Some(e) = current {
        if let Some(io) = e.downcast_ref::<std::io::Error>()
            && matches!(
                io.kind(),
                ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
                    | ErrorKind::UnexpectedEof
                    | ErrorKind::NotConnected
            )
        {
            return true;
        }
        // tungstenite's `ProtocolError::ResetWithoutClosingHandshake`, and its
        // `ConnectionClosed` / `AlreadyClosed`, which are not public through axum.
        let text = e.to_string();
        if text.contains("without closing handshake")
            || text.contains("Connection closed normally")
            || text.contains("closed connection")
        {
            return true;
        }
        current = e.source();
    }
    false
}

/// Log a receive failure: a client that just went away at debug, anything else
/// (a malformed frame, an oversized message) at warn.
fn log_receive_error(what: &str, err: &axum::Error) {
    if is_routine_disconnect(err) {
        tracing::debug!("WS: client went away ({what}): {err}");
    } else {
        tracing::warn!("WS: {what}: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::seeded_admin_app_state;
    use crate::ws_manager::WsFrame;
    use std::collections::HashMap;
    use tokio::sync::mpsc::Receiver;

    fn write_msg(tag_id: &str, command_id: &str) -> ClientMessage {
        ClientMessage::TagWrite {
            tag_id: tag_id.to_string(),
            value: 42.0,
            command_id: Some(command_id.to_string()),
            reason: None,
        }
    }

    /// The only message in the queue: (command id, status, device response).
    fn only_ack(rx: &mut Receiver<WsFrame>) -> (String, String, Option<String>) {
        let ack = match rx.try_recv() {
            Ok(WsFrame::Message(ServerMessage::CommandAck {
                command_id,
                status,
                device_response,
                ..
            })) => (command_id, status, device_response),
            other => panic!("expected a CommandAck, got {other:?}"),
        };
        assert!(rx.try_recv().is_err(), "exactly one answer");
        ack
    }

    /// A write the server accepted sends the requester nothing yet.
    fn assert_accepted(rx: &mut Receiver<WsFrame>) {
        assert!(
            rx.try_recv().is_err(),
            "an accepted write is answered when the command is final"
        );
    }

    /// Two connected clients and a Modbus tag on a device the store does not know, so a
    /// permitted write fails. The tag's cache value starts at 1.0.
    async fn setup(writable: bool) -> (AppState, Receiver<WsFrame>, Receiver<WsFrame>, i64) {
        let state = seeded_admin_app_state().await;
        let requester = state
            .ws_manager
            .connect("me".into(), "admin".into(), vec![]);
        let other = state
            .ws_manager
            .connect("other".into(), "admin".into(), vec![]);
        let id = state
            .storage
            .create_data_source(
                "dev1.temp",
                "Temp",
                "",
                "#fff",
                0.0,
                100.0,
                0.0,
                0.0,
                0.0,
                0,
                1,
                "modbus",
                r#"{"address":40001}"#,
                "dev1",
                None,
                "line",
            )
            .await
            .unwrap();
        if writable {
            state
                .storage
                .update_write_policy(
                    id,
                    &crate::models::WritePolicyUpdate {
                        writable: Some(true),
                        write_min: None,
                        write_max: None,
                        write_max_rate: None,
                        write_requires_reason: None,
                        write_deadband: None,
                    },
                )
                .await
                .unwrap();
        }
        state
            .cache
            .update("dev1", HashMap::from([("temp".to_string(), 1.0)]), false)
            .await;
        (state, requester, other, id)
    }

    async fn dispatch(state: &AppState, role: &str, msg: ClientMessage) {
        handle_client_message(&msg, "me", &state.ws_manager, state, "admin", role).await;
    }

    #[tokio::test]
    async fn non_admin_write_is_rejected_to_the_requester_only() {
        let (state, mut me, mut other, id) = setup(true).await;
        dispatch(&state, "operator", write_msg(&id.to_string(), "c1")).await;

        let (command_id, status, reason) = only_ack(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c1", "rejected"));
        assert!(reason.unwrap().contains("admin"));
        assert!(other.try_recv().is_err(), "nobody else is told");
        assert_eq!(state.cache.get_metric_value("temp").await, Some(1.0));
    }

    #[tokio::test]
    async fn refused_writes_answer_the_requester() {
        let (state, mut me, mut other, id) = setup(false).await;

        // Non-numeric id.
        dispatch(&state, "admin", write_msg("not-a-number", "c1")).await;
        let (command_id, status, _) = only_ack(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c1", "rejected"));

        // Unknown data source.
        dispatch(&state, "admin", write_msg("9999", "c2")).await;
        let (command_id, status, _) = only_ack(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c2", "rejected"));

        // Not writable.
        dispatch(&state, "admin", write_msg(&id.to_string(), "c3")).await;
        let (command_id, status, reason) = only_ack(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c3", "rejected"));
        assert!(reason.unwrap().contains("not writable"));

        assert!(other.try_recv().is_err(), "nobody else is told");
        assert_eq!(state.cache.get_metric_value("temp").await, Some(1.0));
    }

    async fn set_policy(state: &AppState, id: i64, update: crate::models::WritePolicyUpdate) {
        state
            .storage
            .update_write_policy(id, &update)
            .await
            .unwrap();
    }

    fn writable_with(
        configure: impl FnOnce(&mut crate::models::WritePolicyUpdate),
    ) -> crate::models::WritePolicyUpdate {
        let mut update = crate::models::WritePolicyUpdate {
            writable: Some(true),
            write_min: None,
            write_max: None,
            write_max_rate: None,
            write_requires_reason: None,
            write_deadband: None,
        };
        configure(&mut update);
        update
    }

    async fn assert_rejected(
        state: &AppState,
        me: &mut Receiver<WsFrame>,
        id: i64,
        command_id: &str,
        expected: &str,
    ) {
        let (got_id, status, reason) = only_ack(me);
        assert_eq!((got_id.as_str(), status.as_str()), (command_id, "rejected"));
        assert!(
            reason.as_deref().unwrap().contains(expected),
            "{reason:?} should mention {expected:?}"
        );
        assert!(
            state
                .storage
                .get_command(command_id)
                .await
                .unwrap()
                .is_none(),
            "a refused write is not queued"
        );
        let _ = id;
    }

    #[tokio::test]
    async fn permitted_write_is_queued_and_audited() {
        let (state, mut me, mut other, id) = setup(true).await;
        set_policy(
            &state,
            id,
            writable_with(|p| p.write_requires_reason = Some(true)),
        )
        .await;

        let msg = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: 42.0,
            command_id: Some("c1".into()),
            reason: Some("raise setpoint for test run".into()),
        };
        dispatch(&state, "admin", msg).await;

        assert_accepted(&mut me);
        assert!(other.try_recv().is_err(), "nobody else is told");

        let cmd = state.storage.get_command("c1").await.unwrap().unwrap();
        assert_eq!(cmd.created_by, "admin");
        assert_eq!(cmd.status, "pending");
        assert_eq!(cmd.value, "42");
        assert_eq!(cmd.data_source_id, id);
        assert_eq!(cmd.device_ref, "dev1");

        let audit = state.storage.get_command_audit("c1").await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].new_status, "pending");
        assert_eq!(audit[0].changed_by, "admin");
        assert_eq!(
            audit[0].reason.as_deref(),
            Some("raise setpoint for test run")
        );

        assert_eq!(
            state.cache.get_metric_value("temp").await,
            Some(1.0),
            "the cache changes when the device acknowledges, not when the write is queued"
        );
    }

    #[tokio::test]
    async fn write_without_command_id_gets_a_generated_one() {
        let (state, mut me, _other, id) = setup(true).await;
        let msg = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: 5.0,
            command_id: None,
            reason: None,
        };
        dispatch(&state, "admin", msg).await;
        assert_accepted(&mut me);
        let commands = state
            .storage
            .list_commands(&crate::models::CommandQuery::default())
            .await
            .unwrap();
        assert_eq!(commands.len(), 1);
        assert!(commands[0].id.starts_with("ws-"), "{}", commands[0].id);
        assert_eq!(commands[0].created_by, "admin");
    }

    /// What the queue publishes for a command, as a broadcast.
    fn queue_says(state: &AppState, command_id: &str, status: &str, response: Option<&str>) {
        state
            .ws_manager
            .broadcast_command_ack(ServerMessage::CommandAck {
                command_id: command_id.to_string(),
                status: status.to_string(),
                device_response: response.map(str::to_string),
                latency_ms: None,
            });
    }

    fn ack_of(rx: &mut Receiver<WsFrame>) -> (String, String, Option<String>) {
        match rx.try_recv() {
            Ok(WsFrame::Binary(bytes)) => match rmp_serde::from_slice::<ServerMessage>(&bytes) {
                Ok(ServerMessage::CommandAck {
                    command_id,
                    status,
                    device_response,
                    ..
                }) => (command_id, status, device_response),
                other => panic!("expected a CommandAck, got {other:?}"),
            },
            other => panic!("expected a frame, got {other:?}"),
        }
    }

    /// The requester sees one acknowledgement of its command, the final one (clients of the
    /// direct write took the first as the outcome); the others see every step.
    #[tokio::test]
    async fn requester_gets_only_the_final_acknowledgement_others_get_every_step() {
        let (state, mut me, mut other, id) = setup(true).await;
        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        assert_accepted(&mut me);

        queue_says(&state, "c1", "sent", None);
        assert!(
            me.try_recv().is_err(),
            "`sent` is not the requester's outcome"
        );
        queue_says(&state, "c1", "queued", Some("device busy"));
        assert!(me.try_recv().is_err(), "nor is a retry");
        queue_says(&state, "c1", "acked", Some("OK"));
        let (command_id, status, response) = ack_of(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c1", "acked"));
        assert_eq!(response.as_deref(), Some("OK"));
        assert!(me.try_recv().is_err(), "exactly one answer");

        let steps: Vec<String> = (0..3).map(|_| ack_of(&mut other).1).collect();
        assert_eq!(steps, ["sent", "queued", "acked"]);
    }

    #[tokio::test]
    async fn a_failed_command_is_the_requesters_final_answer() {
        let (state, mut me, _other, id) = setup(true).await;
        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        queue_says(&state, "c1", "sent", None);
        queue_says(&state, "c1", "failed", Some("Modbus device not found"));
        let (_, status, response) = ack_of(&mut me);
        assert_eq!(status, "failed");
        assert_eq!(response.as_deref(), Some("Modbus device not found"));
        assert!(me.try_recv().is_err());
    }

    /// Commands from REST or another connection are not held back from anyone, and a
    /// disconnected requester leaves nothing behind.
    #[tokio::test]
    async fn only_the_submitting_connection_is_filtered() {
        let (state, mut me, mut other, id) = setup(true).await;
        queue_says(&state, "rest-1", "sent", None);
        assert_eq!(ack_of(&mut me).1, "sent");
        assert_eq!(ack_of(&mut other).1, "sent");

        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        assert!(state.ws_manager.disconnect("me"));
        queue_says(&state, "c1", "sent", None);
        assert_eq!(ack_of(&mut other).1, "sent");
        queue_says(&state, "c1", "acked", None);
        assert_eq!(ack_of(&mut other).1, "acked");
    }

    #[tokio::test]
    async fn write_outside_the_range_is_refused() {
        let (state, mut me, _other, id) = setup(true).await;
        set_policy(
            &state,
            id,
            writable_with(|p| {
                p.write_min = Some(Some(10.0));
                p.write_max = Some(Some(50.0));
            }),
        )
        .await;

        dispatch(&state, "admin", write_msg(&id.to_string(), "low")).await; // 42 is inside
        assert_accepted(&mut me);

        let too_high = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: 51.0,
            command_id: Some("high".into()),
            reason: None,
        };
        dispatch(&state, "admin", too_high).await;
        assert_rejected(&state, &mut me, id, "high", "above maximum").await;

        let too_low = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: 9.0,
            command_id: Some("low2".into()),
            reason: None,
        };
        dispatch(&state, "admin", too_low).await;
        assert_rejected(&state, &mut me, id, "low2", "below minimum").await;
    }

    #[tokio::test]
    async fn write_that_is_not_finite_is_refused() {
        let (state, mut me, _other, id) = setup(true).await;
        let msg = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: f64::NAN,
            command_id: Some("nan".into()),
            reason: None,
        };
        dispatch(&state, "admin", msg).await;
        assert_rejected(&state, &mut me, id, "nan", "Invalid numeric value").await;
    }

    #[tokio::test]
    async fn write_requiring_a_reason_is_refused_without_one() {
        let (state, mut me, _other, id) = setup(true).await;
        set_policy(
            &state,
            id,
            writable_with(|p| p.write_requires_reason = Some(true)),
        )
        .await;

        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        assert_rejected(&state, &mut me, id, "c1", "Reason is required").await;

        let blank = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value: 42.0,
            command_id: Some("c2".into()),
            reason: Some("   ".into()),
        };
        dispatch(&state, "admin", blank).await;
        assert_rejected(&state, &mut me, id, "c2", "Reason is required").await;
    }

    #[tokio::test]
    async fn write_within_the_deadband_is_refused() {
        let (state, mut me, _other, id) = setup(true).await;
        set_policy(
            &state,
            id,
            writable_with(|p| p.write_deadband = Some(Some(5.0))),
        )
        .await;
        state
            .cache
            .update("dev1", HashMap::from([("temp".to_string(), 40.0)]), false)
            .await;

        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await; // |42 - 40| <= 5
        assert_rejected(&state, &mut me, id, "c1", "deadband").await;
    }

    #[tokio::test]
    async fn write_over_the_rate_limit_is_refused() {
        let (state, mut me, _other, id) = setup(true).await;
        set_policy(
            &state,
            id,
            writable_with(|p| p.write_max_rate = Some(Some(1))),
        )
        .await;

        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        assert_accepted(&mut me);

        dispatch(&state, "admin", write_msg(&id.to_string(), "c2")).await;
        assert_rejected(&state, &mut me, id, "c2", "Rate limit").await;
    }

    #[tokio::test]
    async fn reusing_a_command_id_is_refused() {
        let (state, mut me, _other, id) = setup(true).await;
        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        assert_accepted(&mut me);

        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;
        let (_, status, reason) = only_ack(&mut me);
        assert_eq!(status, "rejected");
        assert!(reason.unwrap().contains("already in use"));
        assert_eq!(
            state.storage.get_command_audit("c1").await.unwrap().len(),
            1
        );
    }

    /// What the server's `recv()` returns when the client ends the connection
    /// in a given way, as `(is_routine_disconnect, error text)`. Uses a real
    /// socket so the error is the one tungstenite produces, not a hand-made one.
    async fn server_receive_error(
        client_action: impl AsyncFnOnce(&mut tokio::net::TcpStream),
    ) -> (bool, String) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (tx, rx) = tokio::sync::oneshot::channel::<(bool, String)>();
        let tx = Arc::new(std::sync::Mutex::new(Some(tx)));
        let app = axum::Router::new().route(
            "/ws",
            axum::routing::get(move |ws: WebSocketUpgrade| {
                let tx = tx.clone();
                async move {
                    ws.on_upgrade(move |mut socket| async move {
                        let verdict = match socket.recv().await {
                            Some(Err(e)) => (is_routine_disconnect(&e), e.to_string()),
                            other => (false, format!("no error: {other:?}")),
                        };
                        if let Some(tx) = tx.lock().unwrap().take() {
                            let _ = tx.send(verdict);
                        }
                    })
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        client
            .write_all(
                b"GET /ws HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
                  Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
            )
            .await
            .unwrap();
        let mut head = [0u8; 12];
        client.read_exact(&mut head).await.unwrap();
        assert_eq!(&head, b"HTTP/1.1 101", "upgrade accepted");

        client_action(&mut client).await;
        drop(client);

        let verdict = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("server noticed the disconnect")
            .unwrap();
        server.abort();
        verdict
    }

    #[tokio::test]
    async fn a_client_that_closes_the_tcp_stream_is_a_routine_disconnect() {
        let (routine, text) = server_receive_error(async |_client| {}).await;
        assert!(routine, "closed without a closing handshake: {text}");
    }

    #[tokio::test]
    async fn a_reset_connection_is_a_routine_disconnect() {
        let (routine, text) = server_receive_error(async |client| {
            client.set_zero_linger().unwrap();
        })
        .await;
        assert!(routine, "connection reset: {text}");
    }

    #[tokio::test]
    async fn a_protocol_violation_is_still_a_real_error() {
        use tokio::io::AsyncWriteExt;
        // An unmasked client frame: a client bug, not a vanished peer.
        let (routine, text) = server_receive_error(async |client| {
            client.write_all(&[0x82, 0x01, 0x41]).await.unwrap();
        })
        .await;
        assert!(!routine, "must stay a warning: {text}");
    }

    #[test]
    fn io_errors_wrapped_by_the_stack_are_classified_by_kind() {
        use std::io::{Error, ErrorKind};
        for kind in [
            ErrorKind::ConnectionReset,
            ErrorKind::BrokenPipe,
            ErrorKind::ConnectionAborted,
        ] {
            assert!(is_routine_disconnect(&axum::Error::new(Error::from(kind))));
        }
        for kind in [ErrorKind::PermissionDenied, ErrorKind::InvalidData] {
            assert!(!is_routine_disconnect(&axum::Error::new(Error::from(kind))));
        }
    }
}
