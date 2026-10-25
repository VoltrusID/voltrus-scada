//! The console's network side: one actor on its own tokio thread owns the
//! session (REST login, the WebSocket tag stream, reconnects) and reports to
//! the window through [`Event`]s.
//!
//! The actor never waits on REST calls other than signing in: acknowledging,
//! shelving, alarm refreshes and history run as separate tasks, so tag
//! updates keep flowing while the server answers.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::api::{Alarm, DataSourceSummary, HistorySeries, VoltrusApi};
use crate::ws_types::{ClientMessage, ServerMessage, TagUpdateEntry};

/// What the window asks for.
#[derive(Debug, Clone)]
pub enum Command {
    SignIn {
        url: String,
        username: String,
        password: String,
    },
    SignOut,
    /// Replace the signed-in user's password.
    ChangePassword {
        current: String,
        new: String,
    },
    Subscribe(Vec<String>),
    /// Write `value` to a tag. The console knows tags by `key`; the server
    /// addresses writes by the data source's numeric `id`.
    Write {
        key: String,
        id: i64,
        value: f64,
    },
    Acknowledge(i64),
    AcknowledgeAll,
    Shelve {
        alarm: i64,
        seconds: u64,
    },
    Unshelve(i64),
    RefreshAlarms,
    /// Load history for every tag over `range` (`30m`, `1h`, `6h`, `24h`, `7d`).
    History(String),
    Shutdown,
}

/// The connection to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    SignedOut,
    SigningIn,
    Live,
    Reconnecting {
        attempt: u32,
        retry_in_secs: u64,
        reason: String,
    },
    Failed(String),
}

impl Link {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SignedOut => "Signed out",
            Self::SigningIn => "Signing in",
            Self::Live => "Live",
            Self::Reconnecting { .. } => "Reconnecting",
            Self::Failed(_) => "Offline",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RaisedAlarm {
    pub alarm_id: i64,
    pub tag_id: String,
    pub severity: String,
    pub message: String,
    pub value: f64,
    pub ts: i64,
}

/// What the window hears back.
#[derive(Debug, Clone)]
pub enum Event {
    Link(Link),
    SignedIn {
        username: String,
        role: String,
        server: String,
        /// The server wants a new password before the operator works.
        must_change_password: bool,
    },
    SignInFailed(String),
    PasswordChanged(Result<(), String>),
    Tags(Vec<DataSourceSummary>),
    Updates(Vec<TagUpdateEntry>),
    Alarms(Vec<Alarm>),
    AlarmRaised(RaisedAlarm),
    History {
        range: String,
        series: HashMap<String, HistorySeries>,
    },
    Written {
        tag: String,
        result: Result<String, String>,
    },
    /// Outcome of an alarm action, for a toast.
    Done {
        action: String,
        result: Result<(), String>,
    },
}

/// Cheap handle for sending commands to the actor.
#[derive(Clone, Debug)]
pub struct Handle {
    commands: mpsc::UnboundedSender<Command>,
}

impl Handle {
    pub fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
    }
}

/// Start the actor thread. Events arrive on the returned receiver.
pub fn start() -> std::io::Result<(Handle, async_channel::Receiver<Event>)> {
    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let (event_tx, event_rx) = async_channel::unbounded();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("scada-io")
        .enable_all()
        .build()?;
    std::thread::Builder::new()
        .name("scada-runtime".into())
        .spawn(move || runtime.block_on(actor(command_rx, Out(event_tx))))?;
    Ok((
        Handle {
            commands: command_tx,
        },
        event_rx,
    ))
}

#[derive(Clone)]
struct Out(async_channel::Sender<Event>);

impl Out {
    fn send(&self, event: Event) {
        let _ = self.0.try_send(event);
    }
}

enum Input {
    Command(Command),
    Socket(Socket),
    Retry(u64),
    /// A write the server has not confirmed in time.
    WriteTimeout(String),
    /// Time to sign in again before the session token expires.
    Renew,
}

enum Socket {
    /// Open and authenticated; the sender writes to the socket.
    Open(u64, mpsc::Sender<ClientMessage>),
    Message(u64, ServerMessage),
    Closed(u64, String),
}

struct Session {
    api: VoltrusApi,
    username: String,
    /// Kept in memory only, to sign in again when the token expires.
    password: String,
    socket: Option<mpsc::Sender<ClientMessage>>,
    /// Identifies the current socket; events from older ones are ignored.
    generation: u64,
    failures: u32,
}

pub fn ws_url(base: &str) -> String {
    let base = base.trim_end_matches('/');
    let scheme = if base.starts_with("https://") {
        base.replacen("https://", "wss://", 1)
    } else {
        base.replacen("http://", "ws://", 1)
    };
    format!("{scheme}/api/v1/ws")
}

/// Signing in again this often keeps the 15-minute session token fresh for
/// REST calls.
const RENEW_EVERY: Duration = Duration::from_secs(10 * 60);

/// How long a write may wait for the server's acknowledgement.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

pub fn backoff(failures: u32) -> Duration {
    match failures {
        0 | 1 => Duration::from_secs(2),
        2 => Duration::from_secs(5),
        _ => Duration::from_secs(10),
    }
}

async fn actor(mut commands: mpsc::UnboundedReceiver<Command>, out: Out) {
    let (input_tx, mut inputs) = mpsc::unbounded_channel::<Input>();
    let forward = input_tx.clone();
    tokio::spawn(async move {
        while let Some(command) = commands.recv().await {
            if forward.send(Input::Command(command)).is_err() {
                break;
            }
        }
    });
    // The server's tokens last 15 minutes; sign in again well before.
    let renew = input_tx.clone();
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(RENEW_EVERY);
        ticks.tick().await;
        loop {
            ticks.tick().await;
            if renew.send(Input::Renew).is_err() {
                break;
            }
        }
    });

    let mut session: Option<Session> = None;
    let mut subscribed: Vec<String> = Vec::new();
    let mut pending_writes: HashMap<String, String> = HashMap::new();
    let mut next_write: u64 = 0;
    // The server sends every command acknowledgement to every client, so
    // this console's ids carry its process id.
    let write_prefix = format!("native-{}", std::process::id());
    // Never reused, so a socket from a signed-out session cannot pass for
    // the next session's.
    let mut generations: u64 = 0;
    let refreshing = Arc::new(AtomicBool::new(false));

    while let Some(input) = inputs.recv().await {
        match input {
            Input::Command(Command::SignIn {
                url,
                username,
                password,
            }) => {
                out.send(Event::Link(Link::SigningIn));
                let mut api = VoltrusApi::new(&url);
                match api.login(&username, &password).await {
                    Ok(login) => {
                        out.send(Event::SignedIn {
                            username: login.username,
                            role: login.role,
                            server: api.base.clone(),
                            must_change_password: login.must_change_password,
                        });
                        subscribed.clear();
                        generations += 1;
                        let generation = generations;
                        let token = api.token.clone().unwrap_or_default();
                        spawn_socket(input_tx.clone(), generation, ws_url(&api.base), token);
                        load_tags(&api, &out);
                        refresh_alarms(&api, &out, &refreshing);
                        session = Some(Session {
                            api,
                            username,
                            password,
                            socket: None,
                            generation,
                            failures: 0,
                        });
                    }
                    Err(error) => {
                        out.send(Event::SignInFailed(error.to_string()));
                        out.send(Event::Link(Link::SignedOut));
                    }
                }
            }
            Input::Command(Command::ChangePassword { current, new }) => {
                let Some(s) = session.as_mut() else {
                    out.send(Event::PasswordChanged(Err("Not signed in".into())));
                    continue;
                };
                // The console does not ask again for the password it just
                // signed in with: an empty `current` means the session's.
                let current = if current.is_empty() {
                    s.password.clone()
                } else {
                    current
                };
                match s.api.change_password(&current, &new).await {
                    Ok(()) => {
                        // Reconnects sign in again with the new password.
                        s.password = new;
                        out.send(Event::PasswordChanged(Ok(())));
                    }
                    Err(error) => out.send(Event::PasswordChanged(Err(error.to_string()))),
                }
            }
            Input::Command(Command::SignOut) => {
                if let Some(old) = session.take() {
                    // Dropping the sender closes the socket's writer; bump the
                    // generation so its Closed event is ignored.
                    drop(old.socket);
                }
                subscribed.clear();
                pending_writes.clear();
                out.send(Event::Link(Link::SignedOut));
            }
            Input::Command(Command::Subscribe(tags)) => {
                for tag in &tags {
                    if !subscribed.contains(tag) {
                        subscribed.push(tag.clone());
                    }
                }
                if let Some(socket) = session.as_ref().and_then(|s| s.socket.as_ref()) {
                    let _ = socket.try_send(ClientMessage::Subscribe { tag_ids: tags });
                }
            }
            Input::Command(Command::Write { key, id, value }) => {
                let socket = session.as_ref().and_then(|s| s.socket.as_ref());
                match socket {
                    Some(socket) => {
                        next_write += 1;
                        let command_id = format!("{write_prefix}-{next_write}");
                        pending_writes.insert(command_id.clone(), key.clone());
                        let message = ClientMessage::TagWrite {
                            tag_id: id.to_string(),
                            value,
                            command_id: Some(command_id.clone()),
                        };
                        if socket.try_send(message).is_err() {
                            pending_writes.remove(&command_id);
                            out.send(Event::Written {
                                tag: key,
                                result: Err("The connection is busy; try again".into()),
                            });
                        } else {
                            // The server stays silent when it refuses a write.
                            let timeout = input_tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(WRITE_TIMEOUT).await;
                                let _ = timeout.send(Input::WriteTimeout(command_id));
                            });
                        }
                    }
                    None => out.send(Event::Written {
                        tag: key,
                        result: Err("Not connected".into()),
                    }),
                }
            }
            Input::WriteTimeout(command_id) => {
                if let Some(key) = pending_writes.remove(&command_id) {
                    out.send(Event::Written {
                        tag: key,
                        result: Err(
                            "No answer from the server. Writing needs the admin role and a tag the server lets you write.".into(),
                        ),
                    });
                }
            }
            Input::Renew => {
                if let Some(s) = session.as_mut() {
                    // On failure the old token serves until the socket drops,
                    // and reconnecting signs in again.
                    let _ = s.api.login(&s.username, &s.password).await;
                }
            }
            Input::Command(Command::Acknowledge(id)) => {
                if let Some(s) = &session {
                    let api = s.api.clone();
                    run_action(
                        &out,
                        &refreshing,
                        api,
                        format!("Acknowledge alarm {id}"),
                        move |api| Box::pin(async move { api.ack_alarm(id).await }),
                    );
                }
            }
            Input::Command(Command::AcknowledgeAll) => {
                if let Some(s) = &session {
                    let api = s.api.clone();
                    run_action(
                        &out,
                        &refreshing,
                        api,
                        "Acknowledge all alarms".into(),
                        |api| Box::pin(async move { api.ack_all().await }),
                    );
                }
            }
            Input::Command(Command::Shelve { alarm, seconds }) => {
                if let Some(s) = &session {
                    let api = s.api.clone();
                    run_action(
                        &out,
                        &refreshing,
                        api,
                        format!("Shelve alarm {alarm}"),
                        move |api| {
                            Box::pin(async move {
                                api.shelve_alarm(
                                    alarm,
                                    seconds,
                                    "Shelved from the operator console",
                                )
                                .await
                            })
                        },
                    );
                }
            }
            Input::Command(Command::Unshelve(id)) => {
                if let Some(s) = &session {
                    let api = s.api.clone();
                    run_action(
                        &out,
                        &refreshing,
                        api,
                        format!("Unshelve alarm {id}"),
                        move |api| Box::pin(async move { api.unshelve_alarm(id).await }),
                    );
                }
            }
            Input::Command(Command::RefreshAlarms) => {
                if let Some(s) = &session {
                    refresh_alarms(&s.api, &out, &refreshing);
                }
            }
            Input::Command(Command::History(range)) => {
                if let Some(s) = &session {
                    let api = s.api.clone();
                    let out = out.clone();
                    tokio::spawn(async move {
                        match api.history(&range).await {
                            Ok(series) => out.send(Event::History { range, series }),
                            Err(error) => out.send(Event::Done {
                                action: format!("Load {range} of history"),
                                result: Err(error.to_string()),
                            }),
                        }
                    });
                }
            }
            Input::Command(Command::Shutdown) => break,

            Input::Socket(Socket::Open(generation, sender)) => {
                let Some(s) = session.as_mut().filter(|s| s.generation == generation) else {
                    continue;
                };
                s.failures = 0;
                if !subscribed.is_empty() {
                    let _ = sender.try_send(ClientMessage::Subscribe {
                        tag_ids: subscribed.clone(),
                    });
                }
                s.socket = Some(sender);
                out.send(Event::Link(Link::Live));
            }
            Input::Socket(Socket::Message(generation, message)) => {
                let Some(s) = session.as_ref().filter(|s| s.generation == generation) else {
                    continue;
                };
                match message {
                    ServerMessage::TagUpdate {
                        tag_id,
                        value,
                        quality,
                        ts,
                        simulated,
                    } => out.send(Event::Updates(vec![TagUpdateEntry {
                        tag_id,
                        value,
                        quality,
                        ts,
                        simulated,
                    }])),
                    ServerMessage::TagUpdateBatch { updates } => out.send(Event::Updates(updates)),
                    ServerMessage::Alarm {
                        alarm_id,
                        tag_id,
                        severity,
                        message,
                        value,
                        ts,
                    } => {
                        out.send(Event::AlarmRaised(RaisedAlarm {
                            alarm_id,
                            tag_id,
                            severity,
                            message,
                            value,
                            ts,
                        }));
                        refresh_alarms(&s.api, &out, &refreshing);
                    }
                    ServerMessage::CommandAck {
                        command_id,
                        status,
                        device_response,
                        latency_ms,
                    } => {
                        if let Some(tag) = pending_writes.remove(&command_id) {
                            let ok = matches!(
                                status.as_str(),
                                "ok" | "success" | "acked" | "confirmed" | "written"
                            );
                            let detail = match (device_response, latency_ms) {
                                (Some(response), Some(ms)) => {
                                    format!("{status}: {response} ({ms} ms)")
                                }
                                (Some(response), None) => format!("{status}: {response}"),
                                (None, Some(ms)) => format!("{status} ({ms} ms)"),
                                (None, None) => status.clone(),
                            };
                            out.send(Event::Written {
                                tag,
                                result: if ok { Ok(detail) } else { Err(detail) },
                            });
                        }
                    }
                    ServerMessage::AuthError { message } => {
                        out.send(Event::Link(Link::Failed(format!(
                            "Not authorised: {message}"
                        ))));
                    }
                    ServerMessage::AuthOk { .. }
                    | ServerMessage::Pong
                    | ServerMessage::DeviceStatus { .. }
                    | ServerMessage::HaStatus { .. } => {}
                }
            }
            Input::Socket(Socket::Closed(generation, reason)) => {
                let Some(s) = session.as_mut().filter(|s| s.generation == generation) else {
                    continue;
                };
                s.socket = None;
                s.failures += 1;
                for (_, tag) in pending_writes.drain() {
                    out.send(Event::Written {
                        tag,
                        result: Err("The connection dropped before the device answered".into()),
                    });
                }
                let wait = backoff(s.failures);
                out.send(Event::Link(Link::Reconnecting {
                    attempt: s.failures,
                    retry_in_secs: wait.as_secs(),
                    reason,
                }));
                let retry = input_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(wait).await;
                    let _ = retry.send(Input::Retry(generation));
                });
            }
            Input::Retry(generation) => {
                let Some(s) = session.as_mut().filter(|s| s.generation == generation) else {
                    continue;
                };
                // The token may have expired while the socket was down.
                if let Err(error) = s.api.login(&s.username, &s.password).await {
                    out.send(Event::Link(Link::Reconnecting {
                        attempt: s.failures,
                        retry_in_secs: backoff(s.failures).as_secs(),
                        reason: format!("Sign-in failed: {error}"),
                    }));
                }
                generations += 1;
                s.generation = generations;
                let token = s.api.token.clone().unwrap_or_default();
                spawn_socket(input_tx.clone(), s.generation, ws_url(&s.api.base), token);
            }
        }
    }
}

fn load_tags(api: &VoltrusApi, out: &Out) {
    let api = api.clone();
    let out = out.clone();
    tokio::spawn(async move {
        match api.data_sources().await {
            Ok(tags) => out.send(Event::Tags(tags)),
            Err(error) => out.send(Event::Done {
                action: "Load tags".into(),
                result: Err(error.to_string()),
            }),
        }
    });
}

/// Fetch the open alarms unless a fetch is already running: an alarm storm
/// triggers one request at a time, not one per alarm.
///
/// The server's `active` list holds only unacknowledged alarms, so the
/// acknowledged and shelved ones come from their own lists; resolved alarms
/// are dropped.
fn refresh_alarms(api: &VoltrusApi, out: &Out, refreshing: &Arc<AtomicBool>) {
    if refreshing.swap(true, Ordering::AcqRel) {
        return;
    }
    let api = api.clone();
    let out = out.clone();
    let refreshing = refreshing.clone();
    tokio::spawn(async move {
        let (active, acknowledged, shelved) = tokio::join!(
            api.alarms("active", 200),
            api.alarms("acknowledged", 200),
            api.alarms("shelved", 200),
        );
        if let (Ok(active), Ok(acknowledged), Ok(shelved)) = (active, acknowledged, shelved) {
            out.send(Event::Alarms(open_alarms([active, acknowledged, shelved])));
        }
        refreshing.store(false, Ordering::Release);
    });
}

/// The unresolved alarms of several lists, each once, in list order.
pub fn open_alarms(lists: [Vec<Alarm>; 3]) -> Vec<Alarm> {
    let mut seen = std::collections::HashSet::new();
    lists
        .into_iter()
        .flatten()
        .filter(|alarm| alarm.resolved_at.is_none() && alarm.state != "resolved")
        .filter(|alarm| seen.insert(alarm.id))
        .collect()
}

type ActionFuture = std::pin::Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

fn run_action(
    out: &Out,
    refreshing: &Arc<AtomicBool>,
    api: VoltrusApi,
    action: String,
    call: impl FnOnce(VoltrusApi) -> ActionFuture + Send + 'static,
) {
    let out = out.clone();
    let refreshing = refreshing.clone();
    tokio::spawn(async move {
        let result = call(api.clone()).await.map_err(|error| error.to_string());
        out.send(Event::Done { action, result });
        refresh_alarms(&api, &out, &refreshing);
    });
}

/// Connect, authenticate, then forward every server message tagged with
/// `generation` until the socket closes.
fn spawn_socket(inputs: mpsc::UnboundedSender<Input>, generation: u64, url: String, token: String) {
    tokio::spawn(async move {
        let reason = match tokio_tungstenite::connect_async(url.as_str()).await {
            Err(error) => format!("Cannot reach {url}: {error}"),
            Ok((socket, _)) => {
                let (mut sink, mut stream) = socket.split();
                let Ok(auth) = rmp_serde::to_vec_named(&ClientMessage::Auth { token }) else {
                    let _ = inputs.send(Input::Socket(Socket::Closed(
                        generation,
                        "Cannot encode sign-in".into(),
                    )));
                    return;
                };
                if sink.send(Message::Binary(auth.into())).await.is_err() {
                    "The server closed the connection".to_owned()
                } else {
                    let (sender, mut queue) = mpsc::channel::<ClientMessage>(64);
                    let _ = inputs.send(Input::Socket(Socket::Open(generation, sender)));
                    let writer = tokio::spawn(async move {
                        while let Some(message) = queue.recv().await {
                            // Named maps: the encoding the server itself sends.
                            let Ok(bytes) = rmp_serde::to_vec_named(&message) else {
                                continue;
                            };
                            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    });
                    let mut reason = "The server closed the connection".to_owned();
                    while let Some(frame) = stream.next().await {
                        match frame {
                            Ok(Message::Binary(data)) => {
                                if let Ok(message) = rmp_serde::from_slice::<ServerMessage>(&data)
                                    && inputs
                                        .send(Input::Socket(Socket::Message(generation, message)))
                                        .is_err()
                                {
                                    break;
                                }
                            }
                            Ok(Message::Close(_)) => break,
                            Err(error) => {
                                reason = error.to_string();
                                break;
                            }
                            Ok(_) => {}
                        }
                    }
                    writer.abort();
                    reason
                }
            }
        };
        let _ = inputs.send(Input::Socket(Socket::Closed(generation, reason)));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_follows_the_http_scheme() {
        assert_eq!(ws_url("http://plant:3100/"), "ws://plant:3100/api/v1/ws");
        assert_eq!(
            ws_url("https://scada.example"),
            "wss://scada.example/api/v1/ws"
        );
    }

    #[test]
    fn reconnects_back_off_and_cap() {
        assert_eq!(backoff(1), Duration::from_secs(2));
        assert_eq!(backoff(9), Duration::from_secs(10));
    }

    #[test]
    fn open_alarms_drop_resolved_and_repeats() {
        let alarm = |id: i64, state: &str, resolved_at: Option<i64>| Alarm {
            id,
            state: state.into(),
            resolved_at,
            ..Alarm::default()
        };
        let open = open_alarms([
            vec![alarm(1, "active", None)],
            vec![
                alarm(2, "acknowledged", None),
                alarm(3, "resolved", Some(9)),
            ],
            vec![alarm(4, "shelved", None), alarm(1, "active", None)],
        ]);
        let ids: Vec<i64> = open.iter().map(|alarm| alarm.id).collect();
        assert_eq!(ids, [1, 2, 4]);
    }
}
