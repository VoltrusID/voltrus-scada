//! Server-Sent Events fallback for the live data stream.
//!
//! Carries what the WebSocket (`websocket.rs`) carries to a read-only client, as
//! JSON: tag batches, command acknowledgements, device status, alarms and HA
//! status. It exists for networks whose proxies refuse WebSocket upgrades. It
//! sits behind the same authentication as the REST API (session cookie or API
//! key) and registers with the same [`WsManager`](crate::ws_manager::WsManager),
//! so it receives exactly the frames WebSocket clients do. It cannot write; tag
//! writes go through `POST /api/v1/tags/{id}/write`.

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::extract::{Extension, State};
use axum::http::{HeaderName, HeaderValue};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream};

use super::{AppState, AuthUser};
use crate::ws_manager::{WsFrame, WsManager};
use crate::ws_types::ServerMessage;

/// Removes the connection from the manager when the client goes away and the
/// response stream is dropped.
struct Registration {
    manager: Arc<WsManager>,
    conn_id: String,
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.manager.disconnect(&self.conn_id);
        tracing::info!("SSE: disconnected conn={}", self.conn_id);
    }
}

/// The SSE event name of a message: its `type` (the serde tag), which the client
/// also finds inside the JSON.
fn event_name(msg: &ServerMessage) -> &'static str {
    match msg {
        ServerMessage::AuthOk { .. } => "AuthOk",
        ServerMessage::AuthError { .. } => "AuthError",
        ServerMessage::TagUpdate { .. } => "TagUpdate",
        ServerMessage::TagUpdateBatch { .. } => "TagUpdateBatch",
        ServerMessage::CommandAck { .. } => "CommandAck",
        ServerMessage::DeviceStatus { .. } => "DeviceStatus",
        ServerMessage::Alarm { .. } => "Alarm",
        ServerMessage::AlarmUpdate { .. } => "AlarmUpdate",
        ServerMessage::Pong => "Pong",
        ServerMessage::HaStatus { .. } => "HaStatus",
    }
}

/// The SSE event for one message: named after its type, the message as JSON
/// (`type` field included) as data, so a client can feed it to the same handler as
/// a decoded WebSocket frame.
fn to_event(msg: &ServerMessage) -> Option<Event> {
    let json = serde_json::to_string(msg).ok()?;
    Some(Event::default().event(event_name(msg)).data(json))
}

fn frame_to_event(frame: WsFrame) -> Option<Event> {
    match frame {
        WsFrame::Message(msg) => to_event(&msg),
        // Broadcasts are encoded once as MessagePack for the WebSocket clients.
        WsFrame::Binary(bytes) => match rmp_serde::from_slice::<ServerMessage>(&bytes) {
            Ok(msg) => to_event(&msg),
            Err(e) => {
                tracing::warn!("SSE: dropping a frame that does not decode: {e}");
                None
            }
        },
    }
}

/// GET /api/v1/stream -- live updates as `text/event-stream`.
#[utoipa::path(
    get,
    path = "/api/v1/stream",
    tag = "Stream",
    responses(
        (status = 200, description = "Server-sent events: one event per live message, named after its type, with the message as JSON data", content_type = "text/event-stream"),
        (status = 401),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn sse_handler(
    State(state): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> impl IntoResponse {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let conn_id = format!("sse-{}", SEQ.fetch_add(1, Ordering::Relaxed));
    let permissions = super::websocket::role_to_permissions(&user.role);

    let rx = state
        .ws_manager
        .connect(conn_id.clone(), user.username.clone(), permissions.clone());
    tracing::info!(
        "SSE: user={} role={} conn={}",
        user.username,
        user.role,
        conn_id
    );
    let registration = Registration {
        manager: state.ws_manager.clone(),
        conn_id,
    };

    // Same greeting as the WebSocket's AuthOk, so the client knows the stream
    // is live even when a proxy holds back the response headers until data flows.
    let hello = ServerMessage::AuthOk {
        user: user.username,
        permissions,
    };
    let hello = stream::iter(to_event(&hello).map(Ok::<_, Infallible>));

    let live = stream::unfold((rx, registration), |(mut rx, registration)| async move {
        loop {
            let frame = rx.recv().await?;
            if let Some(event) = frame_to_event(frame) {
                return Some((Ok::<_, Infallible>(event), (rx, registration)));
            }
        }
    });

    let body: std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>> =
        Box::pin(futures_util::StreamExt::chain(hello, live));
    let sse = Sse::new(body).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    );
    (
        // Tell nginx-style proxies not to buffer the stream.
        [(
            HeaderName::from_static("x-accel-buffering"),
            HeaderValue::from_static("no"),
        )],
        sse,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::tests::*;
    use crate::ws_types::TagUpdateEntry;
    use axum::body::Body;
    use futures_util::StreamExt;
    use http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn next_chunk(
        stream: &mut (impl Stream<Item = Result<bytes::Bytes, axum::Error>> + Unpin),
    ) -> String {
        let chunk = tokio::time::timeout(Duration::from_secs(5), stream.next())
            .await
            .expect("an event within 5 s")
            .expect("the stream is open")
            .expect("no body error");
        String::from_utf8(chunk.to_vec()).unwrap()
    }

    fn stream_request(cookie: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method("GET").uri("/api/v1/stream");
        if let Some(c) = cookie {
            b = b.header("cookie", c);
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn stream_requires_authentication() {
        let app = test_app_with_seeded_admin().await;
        let resp = app.oneshot(stream_request(None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let resp = test_app_with_seeded_admin()
            .await
            .oneshot(stream_request(Some("voltrus_token=garbage")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn stream_greets_then_carries_live_updates_and_unregisters_on_drop() {
        let state = seeded_admin_app_state().await;
        let manager = state.ws_manager.clone();
        let app = crate::api::create_router(state);
        let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

        let resp = app.oneshot(stream_request(Some(&cookie))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()["content-type"].to_str().unwrap(),
            "text/event-stream"
        );
        assert_eq!(manager.connection_count(), 1);

        let mut body = resp.into_body().into_data_stream();
        let hello = next_chunk(&mut body).await;
        assert!(hello.contains("event: AuthOk"), "{hello}");
        assert!(hello.contains(r#""user":"admin""#), "{hello}");

        // A batch broadcast the way the data broadcaster sends it (MessagePack).
        manager.broadcast_preencoded(&ServerMessage::TagUpdateBatch {
            updates: vec![TagUpdateEntry {
                tag_id: "dev1.temp".into(),
                value: 21.5,
                quality: "good".into(),
                ts: 1_700_000_000,
                simulated: false,
            }],
        });
        let batch = next_chunk(&mut body).await;
        assert!(batch.contains("event: TagUpdateBatch"), "{batch}");
        let data = batch
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .expect("a data line");
        let json: serde_json::Value = serde_json::from_str(data).unwrap();
        assert_eq!(json["type"], "TagUpdateBatch");
        assert_eq!(json["updates"][0]["tag_id"], "dev1.temp");
        assert_eq!(json["updates"][0]["value"], 21.5);

        // And a per-message frame, as the command queue and alarm engine send.
        manager.broadcast_command_ack(ServerMessage::CommandAck {
            command_id: "c1".into(),
            status: "acked".into(),
            device_response: Some("OK".into()),
            latency_ms: Some(7),
        });
        let ack = next_chunk(&mut body).await;
        assert!(
            ack.contains("event: CommandAck") && ack.contains(r#""command_id":"c1""#),
            "{ack}"
        );

        drop(body);
        assert_eq!(manager.connection_count(), 0, "the client left");
    }
}
