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

use super::auth_middleware::{ApiKeyAuth, AuthCredential};
use super::websocket::{Grant, SessionTie, WsPrincipal};
use super::{AppState, AuthUser};
use crate::ws_manager::{WsFrame, WsManager};
use crate::ws_types::ServerMessage;

/// How often an open stream re-reads its user or key (as the WebSocket does).
#[cfg(not(test))]
const RECHECK_INTERVAL: Duration = super::websocket::RECHECK_INTERVAL;
#[cfg(test)]
const RECHECK_INTERVAL: Duration = Duration::from_millis(200);

/// Who holds the stream open: a local user or an API key (re-read like the
/// WebSocket's principal, with the role and session it was granted), or a
/// sign-in provider's bearer token (asked of the add-on edition again).
enum Principal {
    Local(WsPrincipal, Grant),
    Bearer { token: String, role: String },
}

impl Principal {
    /// The principal of the credential the middleware authenticated with
    /// ([`AuthCredential`]), not merely of one the request also carries.
    async fn of(
        state: &AppState,
        user: &AuthUser,
        credential: AuthCredential,
        key: Option<&ApiKeyAuth>,
        headers: &HeaderMap,
    ) -> Self {
        let role = user.role.clone();
        match (credential, key) {
            (AuthCredential::ApiKey, Some(key)) => Principal::Local(
                WsPrincipal::ApiKey(key.id),
                Grant {
                    role,
                    session: SessionTie::None,
                },
            ),
            (AuthCredential::Bearer, _) => Principal::Bearer {
                token: headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.strip_prefix("Bearer "))
                    .unwrap_or_default()
                    .to_string(),
                role,
            },
            _ => {
                let jar = axum_extra::extract::CookieJar::from_headers(headers);
                let claims = jar
                    .get("voltrus_token")
                    .and_then(|c| crate::api::auth::verify_token(c.value(), &state.auth_secret));
                let session = match claims {
                    Some(claims) => {
                        SessionTie::of(
                            state,
                            &claims,
                            jar.get("voltrus_refresh").map(|c| c.value()),
                        )
                        .await
                    }
                    // Authenticated as a session, so the cookie verified a
                    // moment ago; fail closed if it no longer does.
                    None => SessionTie::Expires { exp_ms: 0 },
                };
                Principal::Local(
                    WsPrincipal::User(user.username.clone()),
                    Grant { role, session },
                )
            }
        }
    }

    /// Whether the principal may still read the stream: not disabled or
    /// deleted, key not revoked or expired, no password change pending, the
    /// same role as granted, and a person's session still open. Fails
    /// closed on a storage error.
    async fn still_allowed(&self, state: &AppState) -> bool {
        match self {
            Principal::Local(p, grant) => p.still_granted(state, grant).await,
            Principal::Bearer { token, role } => {
                let Some(identity) = state.ext.authenticate_bearer(token).await else {
                    return false;
                };
                if &identity.role != role {
                    return false;
                }
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
        // Collaboration needs the WebSocket; an event stream never joins a session.
        ServerMessage::DesignJoined { .. } => "DesignJoined",
        ServerMessage::DesignSync { .. } => "DesignSync",
        ServerMessage::DesignAwareness { .. } => "DesignAwareness",
        ServerMessage::DesignError { .. } => "DesignError",
        ServerMessage::DesignRejected { .. } => "DesignRejected",
        ServerMessage::DesignFlushed { .. } => "DesignFlushed",
        ServerMessage::DesignSessions { .. } => "DesignSessions",
        ServerMessage::DesignComment { .. } => "DesignComment",
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
    credential: Option<Extension<AuthCredential>>,
    key: Option<Extension<ApiKeyAuth>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let credential = credential.map_or(AuthCredential::Session, |Extension(c)| c);
    let principal = Principal::of(
        &state,
        &user,
        credential,
        key.as_ref().map(|Extension(k)| k),
        &headers,
    )
    .await;
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

    /// Sign in and return the cookie header with the access and refresh tokens.
    async fn session_cookies(app: &axum::Router, username: &str, password: &str) -> String {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"username": username, "password": password}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let access = extract_cookie_value(resp.headers(), "voltrus_token").unwrap();
        let refresh = extract_cookie_value(resp.headers(), "voltrus_refresh").unwrap();
        format!("voltrus_token={access}; voltrus_refresh={refresh}")
    }

    async fn expect_auth_error_then_end(
        body: &mut (impl Stream<Item = Result<bytes::Bytes, axum::Error>> + Unpin),
    ) {
        let bye = next_chunk(body).await;
        assert!(bye.contains("event: AuthError"), "{bye}");
        let end = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .expect("the stream ends");
        assert!(end.is_none());
    }

    #[tokio::test]
    async fn stream_ends_when_the_session_is_revoked_or_the_role_changes() {
        let state = seeded_admin_app_state().await;
        let storage = state.storage.clone();
        let hash = crate::api::auth::hash_password("password-123");
        storage
            .create_user("vic", &hash, "", "operator")
            .await
            .unwrap();
        let app = crate::api::create_router(state);

        // A password change elsewhere (every other refresh token revoked).
        let cookies = session_cookies(&app, "vic", "password-123").await;
        let resp = app
            .clone()
            .oneshot(stream_request(Some(&cookies)))
            .await
            .unwrap();
        let mut body = resp.into_body().into_data_stream();
        assert!(next_chunk(&mut body).await.contains("AuthOk"));
        storage.revoke_all_user_refresh_tokens("vic").await.unwrap();
        expect_auth_error_then_end(&mut body).await;

        // A role change: the stream goes, the client reconnects with the new
        // permissions.
        let cookies = session_cookies(&app, "vic", "password-123").await;
        let resp = app
            .clone()
            .oneshot(stream_request(Some(&cookies)))
            .await
            .unwrap();
        let mut body = resp.into_body().into_data_stream();
        assert!(next_chunk(&mut body).await.contains("AuthOk"));
        let vic = storage.get_user_by_username("vic").await.unwrap().unwrap();
        storage.update_user_role(vic.id, "viewer").await.unwrap();
        expect_auth_error_then_end(&mut body).await;
    }

    /// An add-on whose identity provider vouches for any bearer token "ok-<name>".
    struct FakeBearer;

    impl crate::edition::Extension for FakeBearer {
        fn authenticate_bearer<'a>(
            &'a self,
            token: &'a str,
        ) -> crate::edition::BoxFuture<'a, Option<crate::edition::ExternalIdentity>> {
            Box::pin(async move {
                token
                    .strip_prefix("ok-")
                    .map(|name| crate::edition::ExternalIdentity {
                        username: name.to_string(),
                        role: "viewer".to_string(),
                    })
            })
        }
    }

    /// The credential that authenticated counts: a bearer token next to a
    /// stale session cookie keeps the stream open.
    #[tokio::test]
    async fn a_bearer_stream_with_a_stale_cookie_stays_open() {
        let mut state = seeded_admin_app_state().await;
        state.ext = std::sync::Arc::new(FakeBearer);
        let manager = state.ws_manager.clone();
        let app = crate::api::create_router(state);
        let req = Request::builder()
            .method("GET")
            .uri("/api/v1/stream")
            .header("authorization", "Bearer ok-ana")
            .header("cookie", "voltrus_token=stale.token")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let mut body = resp.into_body().into_data_stream();
        assert!(next_chunk(&mut body).await.contains("AuthOk"));
        // Several rechecks later the stream still carries updates.
        tokio::time::sleep(RECHECK_INTERVAL * 4).await;
        manager.broadcast_command_ack(ServerMessage::CommandAck {
            command_id: "c9".into(),
            status: "acked".into(),
            device_response: None,
            latency_ms: None,
            signature_required: None,
        });
        let ack = next_chunk(&mut body).await;
        assert!(ack.contains("c9"), "{ack}");
    }
}
