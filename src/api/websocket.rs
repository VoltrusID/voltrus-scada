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
        (status = 200, description = "Server-sent events stream"),
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
                tracing::warn!("WS: error receiving first message: {}", e);
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
                        tracing::warn!("WS: recv error: {}", e);
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
        } => {
            let cmd_id = command_id
                .clone()
                .unwrap_or_else(|| format!("direct-{}", crate::timeutil::now_secs()));

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

            handle_tag_write(state, conn_id, username, tag_id, *value, cmd_id).await;
        }

        ClientMessage::Ping => {
            // Application-level ping -- WS protocol ping/pong handled separately.
        }

        ClientMessage::Auth { .. } => {
            tracing::debug!("WS: ignoring duplicate Auth message");
        }
    }
}

// -- Tag write handling: direct write, no queue ----------------------------

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

async fn handle_tag_write(
    state: &AppState,
    conn_id: &str,
    username: &str,
    tag_id: &str,
    value: f64,
    cmd_id: String,
) {
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

    // Look up data source to get device_id and source_config
    let ds = match state.storage.get_data_source_by_id(data_source_id).await {
        Ok(Some(ds)) => ds,
        _ => {
            tracing::warn!("WS: data source {} not found", data_source_id);
            reject_write(&state.ws_manager, conn_id, cmd_id, "Data source not found");
            return;
        }
    };

    // Check write policy
    let writable = state
        .storage
        .get_write_policy(data_source_id)
        .await
        .ok()
        .flatten()
        .map(|p| p.writable)
        .unwrap_or(false);

    if !writable {
        tracing::warn!("WS: data source {} is not writable", data_source_id);
        reject_write(&state.ws_manager, conn_id, cmd_id, "Tag is not writable");
        return;
    }

    let cache_device_id = ds.device_id.clone();
    // Cache metrics live under the bare register name (see data::metric_name).
    let metric_key = crate::api::data::metric_name(&ds.key, &ds.device_id).to_string();

    // Execute the write directly based on protocol
    let result = execute_direct_write(state, &ds, value).await;

    tracing::info!(
        "WS: user={} direct write {}={value} → {} ({}ms)",
        username,
        ds.name,
        if result.success { "OK" } else { "FAIL" },
        result.latency_ms,
    );

    // Only a written value reaches the cache, and through it every client via
    // the data broadcaster pipeline; after a failed write the device still
    // holds the old value.
    if result.success {
        state
            .cache
            .set_metric_value(&cache_device_id, &metric_key, value)
            .await;
    }

    // The result goes to the requesting connection only.
    state.ws_manager.send_to(
        conn_id,
        crate::ws_types::ServerMessage::CommandAck {
            command_id: cmd_id,
            status: if result.success {
                "acked".to_string()
            } else {
                "failed".to_string()
            },
            device_response: result.device_response.or(result.error),
            latency_ms: Some(result.latency_ms),
        },
    );
}

/// Execute a write directly to the device, bypassing the command queue.
async fn execute_direct_write(
    state: &AppState,
    ds: &crate::models::DataSource,
    value: f64,
) -> crate::models::WriteResult {
    let start = std::time::Instant::now();
    let fail = |msg: &str| crate::models::WriteResult {
        success: false,
        device_response: None,
        error: Some(msg.to_string()),
        latency_ms: start.elapsed().as_millis() as u64,
    };

    match ds.source_type.as_str() {
        #[cfg(feature = "modbus")]
        "modbus" => {
            // Parse register address
            let address = match parse_modbus_address(&ds.source_config) {
                Some(a) => a,
                None => return fail("Could not parse Modbus register address"),
            };

            // Get device config
            let device = match state.storage.get_modbus_device(&ds.device_id).await {
                Ok(Some(d)) => d,
                _ => return fail("Modbus device not found"),
            };

            let config = crate::models::DeviceConfig {
                id: device.id.clone(),
                ip: device.ip.clone(),
                port: device.port,
                unit_id: device.unit_id,
                interval_ms: device.interval_ms,
                endianness: match device.endianness.as_str() {
                    "little" => crate::models::Endianness::Little,
                    "swapped" => crate::models::Endianness::Swapped,
                    _ => crate::models::Endianness::Big,
                },
                registers: vec![],
                readback: None,
                network: None,
            };

            let register = crate::models::RegisterConfig {
                name: "unknown".into(),
                address,
                reg_type: crate::models::RegisterType::Uint16,
                scale: 1.0,
                register_area: "holding".into(),
                zero_based: false,
            };
            crate::modbus::write_tag(&state.modbus_pool, &config, &register, value).await
        }
        _ => fail("Direct write not supported for this protocol"),
    }
}

/// Parse Modbus register address from source_config JSON.
#[cfg(feature = "modbus")]
fn parse_modbus_address(source_config: &str) -> Option<u16> {
    let config: serde_json::Value = serde_json::from_str(source_config).ok()?;
    let convert = |addr: u16| -> u16 {
        if addr >= 40001 {
            addr - 40001
        } else if addr >= 30001 {
            addr - 30001
        } else {
            addr
        }
    };
    config
        .get("address")
        .and_then(|v| v.as_u64().map(|a| convert(a as u16)))
        .or_else(|| {
            config
                .get("register")
                .and_then(|v| v.as_u64().map(|a| convert(a as u16)))
        })
}

// -- Helpers ----------------------------------------------------------------

/// Map a user role to a list of permission strings.
fn role_to_permissions(role: &str) -> Vec<String> {
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

    #[tokio::test]
    async fn failed_device_write_answers_the_requester_and_leaves_the_cache() {
        let (state, mut me, mut other, id) = setup(true).await;
        dispatch(&state, "admin", write_msg(&id.to_string(), "c1")).await;

        let (command_id, status, reason) = only_ack(&mut me);
        assert_eq!((command_id.as_str(), status.as_str()), ("c1", "failed"));
        assert_eq!(reason.as_deref(), Some("Modbus device not found"));
        assert!(other.try_recv().is_err(), "the result is not broadcast");
        assert_eq!(
            state.cache.get_metric_value("temp").await,
            Some(1.0),
            "the value that was not written is not shown"
        );
    }
}
