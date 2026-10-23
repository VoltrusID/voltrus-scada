//! Background runtime: owns a dedicated tokio thread for REST + WebSocket I/O
//! and bridges events into the GPUI (smol) main thread.
//!
//! UI → runtime:  `RuntimeHandle::send(UiCmd)` (tokio unbounded channel).
//! runtime → UI:  `smol::channel::Receiver<RtEvent>` drained in a `cx.spawn` loop.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

use crate::api::{Alarm, DataSourceSummary, LoginOk, VoltrusApi};
use crate::ws_types::{ClientMessage, ServerMessage, TagUpdateEntry};

/// Commands from the UI thread.
#[derive(Debug)]
pub enum UiCmd {
    Login {
        url: String,
        username: String,
        password: String,
    },
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
    AckAlarm(i64),
    AckAll,
    RefreshAlarms,
    Shutdown,
}

#[derive(Debug, Clone)]
pub struct AlarmEvent {
    pub alarm_id: i64,
    pub tag_id: String,
    pub severity: String,
    pub message: String,
    pub value: f64,
    pub ts: i64,
}

/// Events to the UI thread.
#[derive(Debug)]
pub enum RtEvent {
    LoginResult(Result<LoginOk, String>),
    /// Human-readable connection state for the status bar.
    Conn(String),
    Sensors(Vec<DataSourceSummary>),
    Tags(Vec<TagUpdateEntry>),
    Alarm(AlarmEvent),
    Alarms(Vec<Alarm>),
    AckResult(Result<(), String>),
}

/// Cheap cloneable handle for views to send commands.
#[derive(Clone)]
pub struct RuntimeHandle {
    tx: mpsc::UnboundedSender<UiCmd>,
}

impl RuntimeHandle {
    pub fn send(&self, cmd: UiCmd) {
        let _ = self.tx.send(cmd);
    }
}

/// Start the runtime thread. Returns the UI-side handle and event receiver.
pub fn start() -> (RuntimeHandle, mpsc::UnboundedReceiver<RtEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (evt_tx, evt_rx) = mpsc::unbounded_channel::<RtEvent>();

    std::thread::Builder::new()
        .name("voltrus-runtime".into())
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(actor(cmd_rx, evt_tx));
        })
        .expect("spawn runtime thread");

    (RuntimeHandle { tx: cmd_tx }, evt_rx)
}

/// Messages from the WS task / reconnect timers back to the actor loop.
enum Ev {
    Cmd(UiCmd),
    Ws(WsFrom),
    Retry,
}

enum WsFrom {
    /// Socket open + authenticated; sender writes client messages to the socket.
    Open(mpsc::Sender<ClientMessage>),
    Msg(ServerMessage),
    Closed,
}

struct Actor {
    api: Option<VoltrusApi>,
    creds: Option<(String, String)>, // username, password — session-only, for re-login
    subscribed: Vec<String>,
    ws_tx: Option<mpsc::Sender<ClientMessage>>,
    failures: u32,
    evt_tx: mpsc::UnboundedSender<RtEvent>,
}

fn ws_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let scheme = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else {
        base.replacen("http://", "ws://", 1)
    };
    format!("{scheme}/api/v1/ws")
}

fn backoff(failures: u32) -> Duration {
    match failures {
        0 => Duration::from_secs(2),
        1 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

async fn actor(mut cmd_rx: mpsc::UnboundedReceiver<UiCmd>, evt_tx: mpsc::UnboundedSender<RtEvent>) {
    let mut state = Actor {
        api: None,
        creds: None,
        subscribed: Vec::new(),
        ws_tx: None,
        failures: 0,
        evt_tx: evt_tx.clone(),
    };

    // Merge UI commands, WS task messages and retry timers into one stream.
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Ev>();
    let cmd_fwd = ev_tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = cmd_rx.recv().await {
            if cmd_fwd.send(Ev::Cmd(cmd)).is_err() {
                break;
            }
        }
    });

    while let Some(ev) = ev_rx.recv().await {
        match ev {
            Ev::Cmd(UiCmd::Login {
                url,
                username,
                password,
            }) => {
                let _ = state.evt_tx.send(RtEvent::Conn("logging in…".into()));
                let mut api = VoltrusApi::new(&url);
                match api.login(&username, &password).await {
                    Ok(login) => {
                        state.creds = Some((username, password));
                        state.failures = 0;
                        state.subscribed.clear();
                        let _ = state.evt_tx.send(RtEvent::LoginResult(Ok(login)));
                        let _ = state.evt_tx.send(RtEvent::Conn("connected".into()));
                        // Initial data: sensors + alarms.
                        if let Ok(sensors) = api.data_sources().await {
                            let _ = state.evt_tx.send(RtEvent::Sensors(sensors));
                        }
                        if let Ok(alarms) = api.alarms("active", 100).await {
                            let _ = state.evt_tx.send(RtEvent::Alarms(alarms));
                        }
                        let base = api.base.clone();
                        let token = api.token.clone().unwrap_or_default();
                        state.api = Some(api);
                        spawn_ws(ev_tx.clone(), ws_url(&base), token);
                    }
                    Err(e) => {
                        let _ = state.evt_tx.send(RtEvent::LoginResult(Err(e.to_string())));
                        let _ = state.evt_tx.send(RtEvent::Conn("offline".into()));
                    }
                }
            }
            Ev::Cmd(UiCmd::Subscribe(keys)) => {
                for k in &keys {
                    if !state.subscribed.contains(k) {
                        state.subscribed.push(k.clone());
                    }
                }
                try_send_ws(&state.ws_tx, ClientMessage::Subscribe { tag_ids: keys });
            }
            Ev::Cmd(UiCmd::Unsubscribe(keys)) => {
                state.subscribed.retain(|k| !keys.contains(k));
                try_send_ws(&state.ws_tx, ClientMessage::Unsubscribe { tag_ids: keys });
            }
            Ev::Cmd(UiCmd::AckAlarm(id)) => {
                if let Some(api) = &state.api {
                    let res = api.ack_alarm(id).await.map_err(|e| e.to_string());
                    let _ = state.evt_tx.send(RtEvent::AckResult(res));
                    if let Ok(alarms) = api.alarms("active", 100).await {
                        let _ = state.evt_tx.send(RtEvent::Alarms(alarms));
                    }
                }
            }
            Ev::Cmd(UiCmd::AckAll) => {
                if let Some(api) = &state.api {
                    let res = api.ack_all().await.map_err(|e| e.to_string());
                    let _ = state.evt_tx.send(RtEvent::AckResult(res));
                    if let Ok(alarms) = api.alarms("active", 100).await {
                        let _ = state.evt_tx.send(RtEvent::Alarms(alarms));
                    }
                }
            }
            Ev::Cmd(UiCmd::RefreshAlarms) => {
                if let Some(api) = &state.api {
                    if let Ok(alarms) = api.alarms("active", 100).await {
                        let _ = state.evt_tx.send(RtEvent::Alarms(alarms));
                    }
                }
            }
            Ev::Cmd(UiCmd::Shutdown) => break,
            Ev::Ws(WsFrom::Open(tx)) => {
                state.ws_tx = Some(tx.clone());
                state.failures = 0;
                let _ = state.evt_tx.send(RtEvent::Conn("live".into()));
                // (Re)subscribe everything after (re)connect.
                if !state.subscribed.is_empty() {
                    try_send_ws(
                        &Some(tx),
                        ClientMessage::Subscribe {
                            tag_ids: state.subscribed.clone(),
                        },
                    );
                }
            }
            Ev::Ws(WsFrom::Msg(msg)) => match msg {
                ServerMessage::AuthOk { .. } => {}
                ServerMessage::AuthError { message } => {
                    let _ = state
                        .evt_tx
                        .send(RtEvent::Conn(format!("auth error: {message}")));
                }
                ServerMessage::TagUpdate {
                    tag_id,
                    value,
                    quality,
                    ts,
                    simulated,
                } => {
                    let _ = state.evt_tx.send(RtEvent::Tags(vec![TagUpdateEntry {
                        tag_id,
                        value,
                        quality,
                        ts,
                        simulated,
                    }]));
                }
                ServerMessage::TagUpdateBatch { updates } => {
                    let _ = state.evt_tx.send(RtEvent::Tags(updates));
                }
                ServerMessage::Alarm {
                    alarm_id,
                    tag_id,
                    severity,
                    message,
                    value,
                    ts,
                } => {
                    let _ = state.evt_tx.send(RtEvent::Alarm(AlarmEvent {
                        alarm_id,
                        tag_id,
                        severity,
                        message,
                        value,
                        ts,
                    }));
                    if let Some(api) = &state.api {
                        if let Ok(alarms) = api.alarms("active", 100).await {
                            let _ = state.evt_tx.send(RtEvent::Alarms(alarms));
                        }
                    }
                }
                ServerMessage::Pong
                | ServerMessage::CommandAck { .. }
                | ServerMessage::DeviceStatus { .. }
                | ServerMessage::HaStatus { .. } => {}
            },
            Ev::Ws(WsFrom::Closed) => {
                state.ws_tx = None;
                if state.creds.is_some() {
                    state.failures += 1;
                    let _ = state.evt_tx.send(RtEvent::Conn(format!(
                        "reconnecting in {}s",
                        backoff(state.failures).as_secs()
                    )));
                    let delay = backoff(state.failures);
                    let ev_tx2 = ev_tx.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        let _ = ev_tx2.send(Ev::Retry);
                    });
                } else {
                    let _ = state.evt_tx.send(RtEvent::Conn("closed".into()));
                }
            }
            Ev::Retry => {
                // Re-login (token may have expired) then reconnect.
                if let (Some(api), Some((user, pass))) = (&mut state.api, state.creds.clone()) {
                    if api.login(&user, &pass).await.is_err() {
                        let _ = state.evt_tx.send(RtEvent::Conn("relogin failed".into()));
                    }
                    let base = api.base.clone();
                    let token = api.token.clone().unwrap_or_default();
                    spawn_ws(ev_tx.clone(), ws_url(&base), token);
                }
            }
        }
    }
}

fn try_send_ws(ws_tx: &Option<mpsc::Sender<ClientMessage>>, msg: ClientMessage) {
    if let Some(tx) = ws_tx {
        let _ = tx.try_send(msg);
    }
}

/// Spawn a WS connection task. Emits `Open` (with a command sender) once the
/// socket is up and the auth frame is written, `Msg` for every decoded server
/// message, and `Closed` when it drops. The socket's writer half lives inside
/// the task; the actor talks to it through the `mpsc::Sender` from `Open`.
fn spawn_ws(ev_tx: mpsc::UnboundedSender<Ev>, url: String, token: String) {
    tokio::spawn(async move {
        let (ws_tx, mut ws_rx) = mpsc::channel::<ClientMessage>(16);
        let ev_tx_closed = ev_tx.clone();

        let conn = tokio::spawn(async move {
            let ws = match tokio_tungstenite::connect_async(url.as_str()).await {
                Ok((ws, _)) => ws,
                Err(e) => {
                    eprintln!("WS connect to {url} failed: {e}");
                    return;
                }
            };
            let (mut sink, mut stream) = ws.split();

            let auth =
                rmp_serde::to_vec_named(&ClientMessage::Auth { token }).expect("encode auth");
            if sink
                .send(tokio_tungstenite::tungstenite::Message::Binary(auth.into()))
                .await
                .is_err()
            {
                return;
            }
            let _ = ev_tx.send(Ev::Ws(WsFrom::Open(ws_tx)));

            // Writer task owns the sink; the reader loop below only reads.
            let writer = tokio::spawn(async move {
                while let Some(msg) = ws_rx.recv().await {
                    // Server decodes with from_slice; named-map form is the
                    // universally accepted encoding (matches its own send_msg).
                    let bytes = rmp_serde::to_vec_named(&msg).expect("encode client msg");
                    if sink
                        .send(tokio_tungstenite::tungstenite::Message::Binary(
                            bytes.into(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            });

            while let Some(frame) = stream.next().await {
                match frame {
                    Ok(tokio_tungstenite::tungstenite::Message::Binary(data)) => {
                        match rmp_serde::from_slice::<ServerMessage>(&data) {
                            Ok(msg) => {
                                if ev_tx.send(Ev::Ws(WsFrom::Msg(msg))).is_err() {
                                    break;
                                }
                            }
                            Err(e) => eprintln!("WS decode error: {e}"),
                        }
                    }
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            writer.abort();
        });

        let _ = conn.await;
        let _ = ev_tx_closed.send(Ev::Ws(WsFrom::Closed));
    });
}
