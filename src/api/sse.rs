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
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::stream::{self, Stream};

use super::auth_middleware::ApiKeyAuth;
use super::websocket::WsPrincipal;
use super::{AppState, AuthUser};
use crate::ws_manager::{WsFrame, WsManager};
use crate::ws_types::ServerMessage;

/// How often an open stream re-reads its user or key (as the WebSocket does).
#[cfg(not(test))]
const RECHECK_INTERVAL: Duration = super::websocket::RECHECK_INTERVAL;
#[cfg(test)]
const RECHECK_INTERVAL: Duration = Duration::from_millis(200);

/// Who holds the stream open: a local user or an API key (re-read like the
/// WebSocket's principal), or a sign-in provider's bearer token (asked of the
/// add-on edition again).
enum Principal {
    Local(WsPrincipal),
    Bearer(String),
}

impl Principal {
    fn of(user: &AuthUser, key: Option<&ApiKeyAuth>, headers: &HeaderMap) -> Self {
        if let Some(key) = key {
            return Principal::Local(WsPrincipal::ApiKey(key.id));
        }
        let bearer = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let has_cookie = axum_extra::extract::CookieJar::from_headers(headers)
            .get("voltrus_token")
            .is_some_and(|c| !c.value().is_empty());
        match bearer {
            Some(token) if !has_cookie => Principal::Bearer(token.to_string()),
            _ => Principal::Local(WsPrincipal::User(user.username.clone())),
        }
    }

    /// Whether the principal may still read the stream: not disabled or
    /// deleted, key not revoked or expired, no password change pending.
    /// Fails closed on a storage error.
    async fn still_allowed(&self, state: &AppState) -> bool {
        match self {
            Principal::Local(p) => p.current_role(state).await.is_some(),
            Principal::Bearer(token) => {
                let Some(identity) = state.ext.authenticate_bearer(token).await else {
                    return false;
                };
                match state.storage.get_user_by_username(&identity.username).await {
                    Ok(user) => !user.is_some_and(|u| u.disabled),
                    Err(_) => false,
                }
            }
        }
    }
}

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
    key: Option<Extension<ApiKeyAuth>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let principal = Principal::of(&user, key.as_ref().map(|Extension(k)| k), &headers);
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

    // The principal is read again every RECHECK_INTERVAL: a disabled user, a
    // revoked key or a pending password change ends the stream with an
    // AuthError event, as the WebSocket closes.
    let recheck = tokio::time::interval_at(
        tokio::time::Instant::now() + RECHECK_INTERVAL,
        RECHECK_INTERVAL,
    );
    struct Live {
        rx: tokio::sync::mpsc::Receiver<WsFrame>,
        registration: Registration,
        recheck: tokio::time::Interval,
        principal: Principal,
        state: AppState,
        ended: bool,
    }
    let live = Live {
        rx,
        registration,
        recheck,
        principal,
        state,
        ended: false,
    };
    let live = stream::unfold(live, |mut live| async move {
        if live.ended {
            return None;
        }
        loop {
            tokio::select! {
                _ = live.recheck.tick() => {
                    if !live.principal.still_allowed(&live.state).await {
                        tracing::info!(
                            "SSE: access gone, closing conn={}",
                            live.registration.conn_id
                        );
                        live.ended = true;
                        let bye = ServerMessage::AuthError {
                            message: "Access revoked".to_string(),
                        };
                        return to_event(&bye).map(|e| (Ok::<_, Infallible>(e), live));
                    }
                }
                frame = live.rx.recv() => {
                    if let Some(event) = frame_to_event(frame?) {
                        return Some((Ok::<_, Infallible>(event), live));
                    }
                }
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
                data_quality: None,
                ts_ms: None,
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
            signature_required: None,
        });
        let ack = next_chunk(&mut body).await;
        assert!(
            ack.contains("event: CommandAck") && ack.contains(r#""command_id":"c1""#),
            "{ack}"
        );

        drop(body);
        assert_eq!(manager.connection_count(), 0, "the client left");
    }

    #[tokio::test]
    async fn stream_ends_when_the_user_is_disabled() {
        let state = seeded_admin_app_state().await;
        let manager = state.ws_manager.clone();
        let storage = state.storage.clone();
        let hash = crate::api::auth::hash_password("password-123");
        storage
            .create_user("vic", &hash, "", "viewer")
            .await
            .unwrap();
        let app = crate::api::create_router(state);
        let cookie = format!(
            "voltrus_token={}",
            login_cookie(&app, "vic", "password-123").await
        );

        let resp = app.oneshot(stream_request(Some(&cookie))).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let mut body = resp.into_body().into_data_stream();
        let hello = next_chunk(&mut body).await;
        assert!(hello.contains("event: AuthOk"), "{hello}");

        let vic = storage.get_user_by_username("vic").await.unwrap().unwrap();
        storage.set_user_disabled(vic.id, true).await.unwrap();

        let bye = next_chunk(&mut body).await;
        assert!(bye.contains("event: AuthError"), "{bye}");
        let end = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .expect("the stream ends");
        assert!(end.is_none(), "no more events after AuthError");
        drop(body);
        assert_eq!(manager.connection_count(), 0);
    }
}
