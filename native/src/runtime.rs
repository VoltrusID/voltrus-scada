//! The console's network side: one actor on its own tokio thread owns the
//! session (REST sign-in, the WebSocket tag stream, reconnects) and reports to
//! the window through [`Event`]s.
//!
//! The actor waits only on the operator's own sign-in. Every other REST call
//! (tags, alarms, history, alarm actions, password changes, token renewals,
//! the sign-in before a reconnect and every generic [`Command::Call`]) runs
//! as its own task and comes back as an input tagged with its session, so
//! tag updates keep flowing while the server answers, and a signed-out
//! session's answers are dropped.
//!
//! When the server refuses the stored password (changed elsewhere, account
//! locked) the session ends instead of trying again: every wrong password
//! counts towards locking the account.
//!
//! # Generic REST calls
//!
//! A feature that needs an endpoint does not add a command or an event:
//! it sends `Command::Call { key, call }` with a [`Call`] built from the
//! [`VoltrusApi`] helpers, and gets `Event::Reply { key, reply }` back with
//! the decoded answer (decoded here, off the UI thread). The [`CallKey`]
//! names the feature (`"alarms.summary"`) and an optional argument, so the
//! console routes the reply to the page that asked.

use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::pin::Pin;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::api::{self, Alarm, DataSource, HistorySeries, Method, VoltrusApi};
use crate::stress::{self, StressFeed, StressOptions};
use crate::ws_types::{ClientMessage, ServerMessage, TagUpdateEntry};

/// Names a generic REST call so its answer finds the code that asked:
/// `"<page id>.<what>"` (the console routes by the part before the dot),
/// plus an argument such as an alarm id or a page number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallKey {
    pub name: &'static str,
    pub arg: i64,
}

impl CallKey {
    pub const fn new(name: &'static str) -> Self {
        Self { name, arg: 0 }
    }

    #[must_use]
    pub const fn with(self, arg: i64) -> Self {
        Self {
            name: self.name,
            arg,
        }
    }

    /// The part of the name before the first dot: the page id.
    pub fn scope(&self) -> &'static str {
        self.name.split('.').next().unwrap_or(self.name)
    }
}

type CallFuture = Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send>, String>> + Send>>;
type CallFn = Box<dyn FnOnce(VoltrusApi) -> CallFuture + Send>;

/// A REST call to run with the session's client. Build one with
/// [`Call::get`], [`Call::send`], [`Call::send_empty`] or, for anything
/// else, [`Call::new`].
pub struct Call(CallFn);

impl Call {
    /// Any async work with the session's client; its result is the reply.
    pub fn new<T, F, Fut>(call: F) -> Self
    where
        T: Send + 'static,
        F: FnOnce(VoltrusApi) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        Self(Box::new(move |api: VoltrusApi| -> CallFuture {
            Box::pin(async move {
                call(api)
                    .await
                    .map(|value| Box::new(value) as Box<dyn Any + Send>)
                    .map_err(|error| error.to_string())
            })
        }))
    }

    /// `GET path`, decoded as `T`.
    pub fn get<T: DeserializeOwned + Send + 'static>(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(move |api| async move { api.get_json::<T>(&path).await })
    }

    /// `method path` with a JSON body, the answer decoded as `T`.
    pub fn send<T: DeserializeOwned + Send + 'static>(
        method: Method,
        path: impl Into<String>,
        body: serde_json::Value,
    ) -> Self {
        let path = path.into();
        Self::new(move |api| async move { api.send_json::<_, T>(method, &path, &body).await })
    }

    /// `method path` without a body, the answer decoded as `T`
    /// (`serde::de::IgnoredAny` when only success matters).
    pub fn send_empty<T: DeserializeOwned + Send + 'static>(
        method: Method,
        path: impl Into<String>,
    ) -> Self {
        let path = path.into();
        Self::new(move |api| async move { api.send_empty::<T>(method, &path).await })
    }
}

/// The answer to a [`Command::Call`]: the decoded value, or why there is
/// none.
pub struct Reply(Result<Box<dyn Any + Send>, String>);

impl Reply {
    pub fn failed(error: impl Into<String>) -> Self {
        Self(Err(error.into()))
    }

    pub fn ok<T: Send + 'static>(value: T) -> Self {
        Self(Ok(Box::new(value)))
    }

    /// The value, if the call succeeded and answered a `T`.
    pub fn take<T: 'static>(self) -> Result<T, String> {
        let value = self.0?;
        value
            .downcast::<T>()
            .map(|value| *value)
            .map_err(|_| format!("the reply is not a {}", std::any::type_name::<T>()))
    }

    pub fn error(&self) -> Option<&str> {
        self.0.as_ref().err().map(String::as_str)
    }
}

impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Ok(_) => f.write_str("Reply(Ok(..))"),
            Err(error) => f.debug_tuple("Reply").field(&Err::<(), _>(error)).finish(),
        }
    }
}

/// What the window asks for.
pub enum Command {
    SignIn {
        url: String,
        username: String,
        password: String,
    },
    /// End the session: the server is told (`POST /auth/logout`, which
    /// revokes the refresh token) and the socket closes.
    SignOut,
    /// Replace the signed-in user's password. An empty `current` means the
    /// password the session signed in with.
    ChangePassword {
        current: String,
        new: String,
    },
    /// Stream these tag keys (WebSocket `Subscribe` takes keys).
    Subscribe(Vec<String>),
    /// Write `value` to the tag with this data source id.
    Write {
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
    /// A generic REST call for the session; answered by [`Event::Reply`]
    /// with the same key.
    Call {
        key: CallKey,
        call: Call,
    },
    /// End any session and feed a synthetic plant instead (see
    /// [`crate::stress`]); `SignOut` stops it.
    Stress(StressOptions),
    Shutdown,
    /// Fetch the tag list again (after a data source was added, changed,
    /// removed or reordered); answered by [`Event::Tags`] as at sign-in.
    RefreshTags,
}

/// Passwords stay out of logs.
impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SignIn { url, username, .. } => f
                .debug_struct("SignIn")
                .field("url", url)
                .field("username", username)
                .finish_non_exhaustive(),
            Self::SignOut => f.write_str("SignOut"),
            Self::ChangePassword { .. } => f.debug_struct("ChangePassword").finish_non_exhaustive(),
            Self::Subscribe(tags) => f.debug_tuple("Subscribe").field(tags).finish(),
            Self::Write { id, value } => f
                .debug_struct("Write")
                .field("id", id)
                .field("value", value)
                .finish(),
            Self::Acknowledge(id) => f.debug_tuple("Acknowledge").field(id).finish(),
            Self::AcknowledgeAll => f.write_str("AcknowledgeAll"),
            Self::Shelve { alarm, seconds } => f
                .debug_struct("Shelve")
                .field("alarm", alarm)
                .field("seconds", seconds)
                .finish(),
            Self::Unshelve(id) => f.debug_tuple("Unshelve").field(id).finish(),
            Self::RefreshAlarms => f.write_str("RefreshAlarms"),
            Self::History(range) => f.debug_tuple("History").field(range).finish(),
            Self::Call { key, .. } => f
                .debug_struct("Call")
                .field("key", key)
                .finish_non_exhaustive(),
            Self::Stress(options) => f.debug_tuple("Stress").field(options).finish(),
            Self::Shutdown => f.write_str("Shutdown"),
            Self::RefreshTags => f.write_str("RefreshTags"),
        }
    }
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
    /// The session ended: the server refused its credentials.
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
#[derive(Debug)]
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
    Tags(Vec<DataSource>),
    Updates(Vec<TagUpdateEntry>),
    Alarms(Vec<Alarm>),
    /// The open alarms could not be fetched: the list shown is stale.
    AlarmsFailed(String),
    AlarmRaised(RaisedAlarm),
    History {
        range: String,
        series: HashMap<String, HistorySeries>,
    },
    /// The answer to a write to the tag with this data source id.
    Written {
        tag: i64,
        result: Result<String, String>,
        /// The answer came after the write was reported unanswered.
        late: bool,
    },
    /// Outcome of an alarm action, for a toast.
    Done {
        action: String,
        result: Result<(), String>,
    },
    /// The answer to a [`Command::Call`].
    Reply {
        key: CallKey,
        reply: Reply,
    },
    /// A device's link to the server came up or went down.
    DeviceStatus {
        device_id: String,
        online: bool,
        latency_ms: Option<u64>,
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

    /// Events the window has not taken yet.
    fn backlog(&self) -> usize {
        self.0.len()
    }
}

enum Input {
    Command(Command),
    Socket(Socket),
    /// Time to reconnect after socket `u64` closed.
    Retry(u64),
    /// A write of session `u64` the server has not confirmed in time.
    WriteTimeout(u64, String),
    /// Time for session `u64` to sign in again before its token expires.
    Renew(u64),
    /// A REST answer for session `u64`.
    Rest(u64, Rest),
}

enum Socket {
    /// Open and authenticated; the sender writes to the socket.
    Open(u64, mpsc::Sender<ClientMessage>),
    Message(u64, ServerMessage),
    Closed(u64, String),
}

/// What a REST task brings back to the actor.
enum Rest {
    Tags(Result<Vec<DataSource>, String>),
    Alarms(Result<Vec<Alarm>, String>),
    History {
        range: String,
        result: Result<HashMap<String, HistorySeries>, String>,
    },
    Action {
        action: String,
        result: Result<(), String>,
    },
    PasswordChanged {
        new: String,
        result: Result<(), String>,
    },
    /// A renewal sign-in: the new token.
    Renewed(Result<String, SignInError>),
    /// The sign-in before reopening socket `socket`: the new token.
    Reconnect {
        socket: u64,
        result: Result<String, SignInError>,
    },
    Reply {
        key: CallKey,
        reply: Reply,
    },
}

#[derive(Debug, Clone)]
struct SignInError {
    message: String,
    /// The server refused the credentials (not merely unreachable).
    refused: bool,
}

impl SignInError {
    fn new(error: &anyhow::Error) -> Self {
        Self {
            message: error.to_string(),
            refused: api::refused(error),
        }
    }
}

/// Sign in again with a copy of the session's client; the new token.
async fn sign_in_again(
    mut api: VoltrusApi,
    username: String,
    password: String,
) -> Result<String, SignInError> {
    match api.login(&username, &password).await {
        Ok(_) => Ok(api.token.unwrap_or_default()),
        Err(error) => Err(SignInError::new(&error)),
    }
}

/// One alarm fetch at a time, so an alarm storm costs one request at a
/// time; a refresh asked for during a fetch runs once it ends, so a change
/// the running fetch may have missed is fetched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Refresh {
    running: bool,
    dirty: bool,
}

impl Refresh {
    /// A refresh is wanted; whether to start a fetch now.
    fn request(&mut self) -> bool {
        if self.running {
            self.dirty = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    /// The running fetch ended; whether to start another.
    fn finish(&mut self) -> bool {
        if self.dirty {
            self.dirty = false;
            true
        } else {
            self.running = false;
            false
        }
    }
}

/// Writes reported as unanswered whose late answer still counts.
const LATE_WRITES: usize = 32;

/// Writes awaiting the server's acknowledgement. The server sends every
/// acknowledgement to every client, so ids carry a random per-session nonce
/// that no other console shares.
#[derive(Debug)]
struct Writes {
    nonce: String,
    next: u64,
    /// Command id → data source id.
    pending: HashMap<String, i64>,
    /// Timed-out writes, oldest first.
    late: VecDeque<(String, i64)>,
}

impl Writes {
    fn new(nonce: String) -> Self {
        Self {
            nonce,
            next: 0,
            pending: HashMap::new(),
            late: VecDeque::new(),
        }
    }

    /// A new command id for a write to `tag`.
    fn issue(&mut self, tag: i64) -> String {
        self.next += 1;
        let id = format!("native-{}-{}", self.nonce, self.next);
        self.pending.insert(id.clone(), tag);
        id
    }

    /// Forget a write that never left.
    fn cancel(&mut self, id: &str) {
        self.pending.remove(id);
    }

    /// The tag of a write that waited too long, if it still waited.
    fn time_out(&mut self, id: &str) -> Option<i64> {
        let tag = self.pending.remove(id)?;
        if self.late.len() == LATE_WRITES {
            self.late.pop_front();
        }
        self.late.push_back((id.to_owned(), tag));
        Some(tag)
    }

    /// The tag of an acknowledged write of ours, and whether it had timed
    /// out.
    fn acknowledge(&mut self, id: &str) -> Option<(i64, bool)> {
        if let Some(tag) = self.pending.remove(id) {
            return Some((tag, false));
        }
        let position = self.late.iter().position(|(late, _)| late == id)?;
        self.late.remove(position).map(|(_, tag)| (tag, true))
    }

    /// The tags of every waiting write, when the connection drops.
    fn abandon(&mut self) -> Vec<i64> {
        self.pending.drain().map(|(_, tag)| tag).collect()
    }
}

/// A random id for this session's writes: std's per-process random hash
/// keys over the time and process id.
fn nonce() -> String {
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos()),
    );
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}

/// The password a change starts from: the console does not ask again for
/// the one it signed in with, so an empty `given` means the session's.
fn current_password(given: String, session: &str) -> String {
    if given.is_empty() {
        session.to_owned()
    } else {
        given
    }
}

struct Session {
    /// Never reused: answers for an older session are dropped.
    id: u64,
    api: VoltrusApi,
    username: String,
    /// Kept in memory only, to sign in again when the token expires.
    password: String,
    socket: Option<mpsc::Sender<ClientMessage>>,
    /// Identifies the current socket; events from older ones are ignored.
    generation: u64,
    /// The current socket's task. Dropping it (the session ends, or a new
    /// socket replaces it) closes the connection.
    reader: AbortOnDrop,
    failures: u32,
    alarms: Refresh,
    writes: Writes,
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

/// After a renewal the server did not answer, try again this soon: the
/// token still has minutes left.
const RENEW_RETRY: Duration = Duration::from_secs(30);

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
    let mut actor = Actor::new(out, input_tx);
    while let Some(input) = inputs.recv().await {
        if !actor.handle(input).await {
            break;
        }
    }
}

/// Send `input` to the actor after `delay`.
fn schedule(inputs: &mpsc::UnboundedSender<Input>, delay: Duration, input: Input) {
    let inputs = inputs.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let _ = inputs.send(input);
    });
}

/// Run a REST call for `session` and hand its answer to the actor.
fn spawn_rest(
    inputs: &mpsc::UnboundedSender<Input>,
    session: u64,
    task: impl Future<Output = Rest> + Send + 'static,
) {
    let inputs = inputs.clone();
    tokio::spawn(async move {
        let rest = task.await;
        let _ = inputs.send(Input::Rest(session, rest));
    });
}

/// Fetch the open alarms. The server's `active` list holds only
/// unacknowledged alarms, so the acknowledged and shelved ones come from
/// their own lists; resolved alarms are dropped.
fn fetch_alarms(inputs: &mpsc::UnboundedSender<Input>, session: &Session) {
    let api = session.api.clone();
    spawn_rest(inputs, session.id, async move {
        let (active, acknowledged, shelved) = tokio::join!(
            api.alarms("active", 200),
            api.alarms("acknowledged", 200),
            api.alarms("shelved", 200),
        );
        Rest::Alarms(match (active, acknowledged, shelved) {
            (Ok(active), Ok(acknowledged), Ok(shelved)) => {
                Ok(open_alarms([active, acknowledged, shelved]))
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error.to_string()),
        })
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

struct Actor {
    out: Out,
    inputs: mpsc::UnboundedSender<Input>,
    session: Option<Session>,
    /// The synthetic plant's feed while a stress test runs instead of a
    /// session; dropping it stops the feed.
    stress: Option<AbortOnDrop>,
    subscribed: Vec<String>,
    /// Session and socket ids. Never reused, so neither a signed-out
    /// session's answers nor an old socket's messages pass for current ones.
    ids: u64,
}

impl Actor {
    fn new(out: Out, inputs: mpsc::UnboundedSender<Input>) -> Self {
        Self {
            out,
            inputs,
            session: None,
            stress: None,
            subscribed: Vec::new(),
            ids: 0,
        }
    }

    fn next_id(&mut self) -> u64 {
        self.ids += 1;
        self.ids
    }

    /// Handle one input; false once the actor should stop.
    async fn handle(&mut self, input: Input) -> bool {
        match input {
            Input::Command(command) => return self.command(command).await,
            Input::Socket(socket) => self.socket(socket),
            Input::Retry(generation) => self.retry(generation),
            Input::WriteTimeout(session, id) => self.write_timeout(session, &id),
            Input::Renew(session) => self.renew(session),
            Input::Rest(session, rest) => {
                if self.session.as_ref().is_some_and(|s| s.id == session) {
                    self.rest(rest);
                }
            }
        }
        true
    }

    async fn command(&mut self, command: Command) -> bool {
        match command {
            Command::SignIn {
                url,
                username,
                password,
            } => {
                self.end_session();
                self.out.send(Event::Link(Link::SigningIn));
                let mut api = VoltrusApi::new(&url);
                match api.login(&username, &password).await {
                    Ok(login) => {
                        self.out.send(Event::SignedIn {
                            username: login.username,
                            role: login.role,
                            server: api.base.clone(),
                            must_change_password: login.must_change_password,
                        });
                        self.begin(api, username, password);
                    }
                    Err(error) => {
                        self.out.send(Event::SignInFailed(error.to_string()));
                        self.out.send(Event::Link(Link::SignedOut));
                    }
                }
            }
            Command::ChangePassword { current, new } => {
                let Some(s) = &self.session else {
                    self.out
                        .send(Event::PasswordChanged(Err("Not signed in".into())));
                    return true;
                };
                let current = current_password(current, &s.password);
                let api = s.api.clone();
                spawn_rest(&self.inputs, s.id, async move {
                    let result = api
                        .change_password(&current, &new)
                        .await
                        .map_err(|error| error.to_string());
                    Rest::PasswordChanged { new, result }
                });
            }
            Command::SignOut => {
                if let Some(s) = self.end_session() {
                    // Revoke the session on the server too; nobody waits
                    // for the answer.
                    let api = s.api;
                    tokio::spawn(async move {
                        let _ = api.logout().await;
                    });
                }
                self.out.send(Event::Link(Link::SignedOut));
            }
            Command::Subscribe(tags) => {
                for tag in &tags {
                    if !self.subscribed.contains(tag) {
                        self.subscribed.push(tag.clone());
                    }
                }
                if let Some(socket) = self.session.as_ref().and_then(|s| s.socket.as_ref()) {
                    let _ = socket.try_send(ClientMessage::Subscribe { tag_ids: tags });
                }
            }
            Command::Write { id, value } => self.write(id, value),
            Command::Acknowledge(id) => {
                self.action(format!("Acknowledge alarm {id}"), move |api| {
                    Box::pin(async move { api.ack_alarm(id).await })
                });
            }
            Command::AcknowledgeAll => {
                self.action("Acknowledge all alarms".into(), |api| {
                    Box::pin(async move { api.ack_all().await })
                });
            }
            Command::Shelve { alarm, seconds } => {
                self.action(format!("Shelve alarm {alarm}"), move |api| {
                    Box::pin(async move {
                        api.shelve_alarm(alarm, seconds, "Shelved from the operator console")
                            .await
                    })
                });
            }
            Command::Unshelve(id) => {
                self.action(format!("Unshelve alarm {id}"), move |api| {
                    Box::pin(async move { api.unshelve_alarm(id).await })
                });
            }
            Command::RefreshAlarms => self.refresh_alarms(),
            Command::History(range) => {
                if let Some(s) = &self.session {
                    let api = s.api.clone();
                    spawn_rest(&self.inputs, s.id, async move {
                        let result = api.history(&range).await.map_err(|error| error.to_string());
                        Rest::History { range, result }
                    });
                }
            }
            Command::Call { key, call } => match &self.session {
                Some(s) => {
                    let api = s.api.clone();
                    spawn_rest(&self.inputs, s.id, async move {
                        Rest::Reply {
                            key,
                            reply: Reply((call.0)(api).await),
                        }
                    });
                }
                None => self.out.send(Event::Reply {
                    key,
                    reply: Reply::failed(if self.stress.is_some() {
                        "A stress test has no server to ask"
                    } else {
                        "Not signed in"
                    }),
                }),
            },
            Command::Stress(options) => {
                self.end_session();
                self.stress = Some(spawn_stress(self.out.clone(), options));
            }
            Command::Shutdown => return false,
            Command::RefreshTags => {
                if let Some(s) = &self.session {
                    let api = s.api.clone();
                    spawn_rest(&self.inputs, s.id, async move {
                        Rest::Tags(api.data_sources().await.map_err(|error| error.to_string()))
                    });
                }
            }
        }
        true
    }

    /// Start a session after a successful sign-in.
    fn begin(&mut self, api: VoltrusApi, username: String, password: String) {
        self.subscribed.clear();
        let id = self.next_id();
        let generation = self.next_id();
        let token = api.token.clone().unwrap_or_default();
        let reader = spawn_socket(self.inputs.clone(), generation, ws_url(&api.base), token);
        let tags = api.clone();
        spawn_rest(&self.inputs, id, async move {
            Rest::Tags(tags.data_sources().await.map_err(|error| error.to_string()))
        });
        // The server's tokens last 15 minutes; sign in again well before.
        schedule(&self.inputs, RENEW_EVERY, Input::Renew(id));
        self.session = Some(Session {
            id,
            api,
            username,
            password,
            socket: None,
            generation,
            reader,
            failures: 0,
            alarms: Refresh::default(),
            writes: Writes::new(nonce()),
        });
        self.refresh_alarms();
    }

    /// Drop the session (its socket task is aborted and its answers
    /// ignored) or stop the stress feed.
    fn end_session(&mut self) -> Option<Session> {
        self.subscribed.clear();
        self.stress = None;
        self.session.take()
    }

    /// The server refused the session's credentials: end it, as signing in
    /// again with them would only count towards locking the account.
    fn refuse(&mut self, reason: &str) {
        if let Some(mut s) = self.end_session() {
            for tag in s.writes.abandon() {
                self.out.send(Event::Written {
                    tag,
                    result: Err("The session ended before the device answered".into()),
                    late: false,
                });
            }
        }
        let reason = reason.trim_end_matches('.');
        self.out.send(Event::Link(Link::Failed(format!(
            "Not authorised: {reason}. Sign in again."
        ))));
    }

    fn refresh_alarms(&mut self) {
        if let Some(s) = self.session.as_mut()
            && s.alarms.request()
        {
            fetch_alarms(&self.inputs, s);
        }
    }

    fn action(
        &self,
        action: String,
        call: impl FnOnce(VoltrusApi) -> ActionFuture + Send + 'static,
    ) {
        if let Some(s) = &self.session {
            let api = s.api.clone();
            spawn_rest(&self.inputs, s.id, async move {
                let result = call(api).await.map_err(|error| error.to_string());
                Rest::Action { action, result }
            });
        }
    }

    fn write(&mut self, id: i64, value: f64) {
        let Some(s) = self.session.as_mut() else {
            self.out.send(Event::Written {
                tag: id,
                result: Err("Not connected".into()),
                late: false,
            });
            return;
        };
        let Some(socket) = s.socket.as_ref() else {
            self.out.send(Event::Written {
                tag: id,
                result: Err("Not connected".into()),
                late: false,
            });
            return;
        };
        let command_id = s.writes.issue(id);
        let message = ClientMessage::TagWrite {
            tag_id: id.to_string(),
            value,
            command_id: Some(command_id.clone()),
        };
        if socket.try_send(message).is_err() {
            s.writes.cancel(&command_id);
            self.out.send(Event::Written {
                tag: id,
                result: Err("The connection is busy; try again".into()),
                late: false,
            });
        } else {
            // The server stays silent when it refuses a write.
            schedule(
                &self.inputs,
                WRITE_TIMEOUT,
                Input::WriteTimeout(s.id, command_id),
            );
        }
    }

    fn write_timeout(&mut self, session: u64, id: &str) {
        let Some(s) = self.session.as_mut().filter(|s| s.id == session) else {
            return;
        };
        if let Some(tag) = s.writes.time_out(id) {
            self.out.send(Event::Written {
                tag,
                result: Err(
                    "No answer from the server. Writing needs the admin role and a tag the server lets you write.".into(),
                ),
                late: false,
            });
        }
    }

    fn renew(&self, session: u64) {
        let Some(s) = self.session.as_ref().filter(|s| s.id == session) else {
            return;
        };
        let task = sign_in_again(s.api.clone(), s.username.clone(), s.password.clone());
        spawn_rest(&self.inputs, s.id, async move { Rest::Renewed(task.await) });
    }

    fn retry(&self, generation: u64) {
        let Some(s) = self
            .session
            .as_ref()
            .filter(|s| s.generation == generation && s.socket.is_none())
        else {
            return;
        };
        // The token may have expired while the socket was down.
        let task = sign_in_again(s.api.clone(), s.username.clone(), s.password.clone());
        spawn_rest(&self.inputs, s.id, async move {
            Rest::Reconnect {
                socket: generation,
                result: task.await,
            }
        });
    }

    /// A REST answer for the current session.
    fn rest(&mut self, rest: Rest) {
        match rest {
            Rest::Tags(Ok(tags)) => self.out.send(Event::Tags(tags)),
            Rest::Tags(Err(error)) => self.out.send(Event::Done {
                action: "Load tags".into(),
                result: Err(error),
            }),
            Rest::Alarms(result) => {
                if let Some(s) = self.session.as_mut()
                    && s.alarms.finish()
                {
                    fetch_alarms(&self.inputs, s);
                }
                self.out.send(match result {
                    Ok(alarms) => Event::Alarms(alarms),
                    Err(error) => Event::AlarmsFailed(error),
                });
            }
            Rest::History { range, result } => self.out.send(match result {
                Ok(series) => Event::History { range, series },
                Err(error) => Event::Done {
                    action: format!("Load {range} of history"),
                    result: Err(error),
                },
            }),
            Rest::Action { action, result } => {
                self.out.send(Event::Done { action, result });
                self.refresh_alarms();
            }
            Rest::PasswordChanged { new, result } => {
                if result.is_ok()
                    && let Some(s) = self.session.as_mut()
                {
                    // Renewals and reconnects sign in with the new password.
                    s.password = new;
                }
                self.out.send(Event::PasswordChanged(result));
            }
            Rest::Renewed(result) => self.renewed(result),
            Rest::Reconnect { socket, result } => self.reconnect(socket, result),
            Rest::Reply { key, reply } => self.out.send(Event::Reply { key, reply }),
        }
    }

    fn renewed(&mut self, result: Result<String, SignInError>) {
        match result {
            Err(error) if error.refused => self.refuse(&error.message),
            result => {
                let Some(s) = self.session.as_mut() else {
                    return;
                };
                let next = match result {
                    Ok(token) => {
                        s.api.token = Some(token);
                        RENEW_EVERY
                    }
                    // The old token serves a few more minutes.
                    Err(_) => RENEW_RETRY,
                };
                schedule(&self.inputs, next, Input::Renew(s.id));
            }
        }
    }

    fn reconnect(&mut self, socket: u64, result: Result<String, SignInError>) {
        if self
            .session
            .as_ref()
            .is_none_or(|s| s.generation != socket || s.socket.is_some())
        {
            return;
        }
        match result {
            Err(error) if error.refused => self.refuse(&error.message),
            Ok(token) => {
                let generation = self.next_id();
                let Some(s) = self.session.as_mut() else {
                    return;
                };
                s.api.token = Some(token.clone());
                s.generation = generation;
                // Replaces (and so aborts) the previous socket's task.
                s.reader =
                    spawn_socket(self.inputs.clone(), generation, ws_url(&s.api.base), token);
            }
            Err(error) => {
                let Some(s) = self.session.as_mut() else {
                    return;
                };
                s.failures += 1;
                let wait = backoff(s.failures);
                self.out.send(Event::Link(Link::Reconnecting {
                    attempt: s.failures,
                    retry_in_secs: wait.as_secs(),
                    reason: format!("Sign-in failed: {}", error.message),
                }));
                schedule(&self.inputs, wait, Input::Retry(socket));
            }
        }
    }

    fn socket(&mut self, socket: Socket) {
        match socket {
            Socket::Open(generation, sender) => {
                let Some(s) = self.session.as_mut().filter(|s| s.generation == generation) else {
                    return;
                };
                s.failures = 0;
                if !self.subscribed.is_empty() {
                    let _ = sender.try_send(ClientMessage::Subscribe {
                        tag_ids: self.subscribed.clone(),
                    });
                }
                s.socket = Some(sender);
                self.out.send(Event::Link(Link::Live));
            }
            Socket::Message(generation, message) => {
                if self
                    .session
                    .as_ref()
                    .is_some_and(|s| s.generation == generation)
                {
                    self.message(message);
                }
            }
            Socket::Closed(generation, reason) => {
                let Some(s) = self.session.as_mut().filter(|s| s.generation == generation) else {
                    return;
                };
                s.socket = None;
                s.failures += 1;
                for tag in s.writes.abandon() {
                    self.out.send(Event::Written {
                        tag,
                        result: Err("The connection dropped before the device answered".into()),
                        late: false,
                    });
                }
                let wait = backoff(s.failures);
                self.out.send(Event::Link(Link::Reconnecting {
                    attempt: s.failures,
                    retry_in_secs: wait.as_secs(),
                    reason,
                }));
                schedule(&self.inputs, wait, Input::Retry(generation));
            }
        }
    }

    /// A message from the current socket.
    fn message(&mut self, message: ServerMessage) {
        match message {
            ServerMessage::TagUpdate {
                tag_id,
                value,
                quality,
                ts,
                simulated,
            } => self.out.send(Event::Updates(vec![TagUpdateEntry {
                tag_id,
                value,
                quality,
                ts,
                simulated,
            }])),
            ServerMessage::TagUpdateBatch { updates } => self.out.send(Event::Updates(updates)),
            ServerMessage::Alarm {
                alarm_id,
                tag_id,
                severity,
                message,
                value,
                ts,
            } => {
                self.out.send(Event::AlarmRaised(RaisedAlarm {
                    alarm_id,
                    tag_id,
                    severity,
                    message,
                    value,
                    ts,
                }));
                self.refresh_alarms();
            }
            ServerMessage::CommandAck {
                command_id,
                status,
                device_response,
                latency_ms,
            } => {
                let Some((tag, late)) = self
                    .session
                    .as_mut()
                    .and_then(|s| s.writes.acknowledge(&command_id))
                else {
                    return;
                };
                let ok = matches!(
                    status.as_str(),
                    "ok" | "success" | "acked" | "confirmed" | "written"
                );
                let detail = match (device_response, latency_ms) {
                    (Some(response), Some(ms)) => format!("{status}: {response} ({ms} ms)"),
                    (Some(response), None) => format!("{status}: {response}"),
                    (None, Some(ms)) => format!("{status} ({ms} ms)"),
                    (None, None) => status.clone(),
                };
                self.out.send(Event::Written {
                    tag,
                    result: if ok { Ok(detail) } else { Err(detail) },
                    late,
                });
            }
            // The socket's token was refused: reconnecting with it would
            // fail the same way.
            ServerMessage::AuthError { message } => self.refuse(&message),
            ServerMessage::DeviceStatus {
                device_id,
                online,
                latency_ms,
            } => self.out.send(Event::DeviceStatus {
                device_id,
                online,
                latency_ms,
            }),
            ServerMessage::AuthOk { .. } | ServerMessage::Pong | ServerMessage::HaStatus { .. } => {
            }
        }
    }
}

/// A task that is aborted when this handle is dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Batches the window has not taken yet beyond which the stress feed skips
/// a tick rather than queue without bound: a console that cannot keep up
/// shows it as a lower applied rate, not as growing memory.
const STRESS_BACKLOG: usize = 64;

/// The user and server a stress test signs in as.
pub const STRESS_USER: &str = "stress";
pub const STRESS_SERVER: &str = "stress://synthetic-plant";

/// Act out a session with a synthetic plant: signed in, live, its tag list,
/// then a batch of updates every [`stress::TICK`] until dropped.
fn spawn_stress(out: Out, options: StressOptions) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
        let mut feed = StressFeed::new(options);
        out.send(Event::SignedIn {
            username: STRESS_USER.into(),
            role: "admin".into(),
            server: STRESS_SERVER.into(),
            must_change_password: false,
        });
        out.send(Event::Link(Link::Live));
        out.send(Event::Tags(feed.tags()));
        out.send(Event::Alarms(Vec::new()));
        let mut ticker = tokio::time::interval(stress::TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if out.backlog() > STRESS_BACKLOG {
                continue;
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| {
                    i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
                });
            out.send(Event::Updates(feed.next_batch(now)));
        }
    }))
}

/// Connect, authenticate, then forward every server message tagged with
/// `generation` until the socket closes or the returned handle is dropped.
fn spawn_socket(
    inputs: mpsc::UnboundedSender<Input>,
    generation: u64,
    url: String,
    token: String,
) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
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
                    // Stops with the reader, also when the reader is aborted.
                    let writer = AbortOnDrop(tokio::spawn(async move {
                        while let Some(message) = queue.recv().await {
                            // Named maps: the encoding the server itself sends.
                            let Ok(bytes) = rmp_serde::to_vec_named(&message) else {
                                continue;
                            };
                            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                                break;
                            }
                        }
                    }));
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
                    drop(writer);
                    reason
                }
            }
        };
        let _ = inputs.send(Input::Socket(Socket::Closed(generation, reason)));
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The actor with a session against a closed port: nothing it spawns
    /// answers, so every answer is one the test feeds in.
    struct Harness {
        runtime: tokio::runtime::Runtime,
        actor: Actor,
        events: async_channel::Receiver<Event>,
        inputs: mpsc::UnboundedReceiver<Input>,
    }

    impl Harness {
        fn new() -> Self {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            let (out, events) = async_channel::unbounded();
            let (inputs, receiver) = mpsc::unbounded_channel();
            Self {
                runtime,
                actor: Actor::new(Out(out), inputs),
                events,
                inputs: receiver,
            }
        }

        fn sign_in(&mut self) -> u64 {
            self.sign_in_at("http://127.0.0.1:9")
        }

        fn sign_in_at(&mut self, url: &str) -> u64 {
            let mut api = VoltrusApi::new(url);
            api.token = Some("token".into());
            let _entered = self.runtime.enter();
            self.actor.begin(api, "admin".into(), "secret".into());
            self.session().id
        }

        /// Let the spawned tasks run for a while.
        fn run_for(&self, duration: Duration) {
            // The timer must be made inside the runtime.
            self.runtime
                .block_on(async move { tokio::time::sleep(duration).await });
        }

        fn session(&self) -> &Session {
            self.actor.session.as_ref().expect("a session")
        }

        fn feed(&mut self, input: Input) {
            self.runtime.block_on(self.actor.handle(input));
        }

        fn socket(&mut self, socket: Socket) {
            self.feed(Input::Socket(socket));
        }

        fn events(&self) -> Vec<Event> {
            std::iter::from_fn(|| self.events.try_recv().ok()).collect()
        }
    }

    fn refused(message: &str) -> SignInError {
        SignInError {
            message: message.into(),
            refused: true,
        }
    }

    fn unreachable() -> SignInError {
        SignInError {
            message: "connect failed".into(),
            refused: false,
        }
    }

    fn is_failed(event: Option<&Event>, text: &str) -> bool {
        matches!(event, Some(Event::Link(Link::Failed(reason))) if reason.contains(text))
    }

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

    #[test]
    fn answers_for_an_ended_session_are_dropped() {
        let mut h = Harness::new();
        let first = h.sign_in();
        h.feed(Input::Command(Command::SignOut));
        let second = h.sign_in();
        assert_ne!(first, second);
        h.events();
        h.feed(Input::Rest(
            first,
            Rest::Tags(Ok(vec![DataSource::default()])),
        ));
        h.feed(Input::Rest(first, Rest::Alarms(Ok(vec![Alarm::default()]))));
        h.feed(Input::Rest(
            first,
            Rest::Action {
                action: "Acknowledge alarm 1".into(),
                result: Ok(()),
            },
        ));
        assert!(
            h.events().is_empty(),
            "the old session's answers are dropped"
        );
        assert_eq!(
            h.session().alarms,
            Refresh {
                running: true,
                dirty: false
            },
            "the old session's fetch does not hold up the new one's"
        );
        h.feed(Input::Rest(
            second,
            Rest::Tags(Ok(vec![DataSource::default()])),
        ));
        assert!(matches!(h.events().as_slice(), [Event::Tags(tags)] if tags.len() == 1));
    }

    /// Accepts connections until the WebSocket upgrade request arrives, then
    /// waits up to five seconds for the console to close that connection.
    /// Never answers, so the console's socket task waits on it forever.
    fn websocket_closed(listener: &std::net::TcpListener) -> bool {
        use std::io::Read;
        let mut others = Vec::new();
        loop {
            let Ok((mut connection, _)) = listener.accept() else {
                return false;
            };
            if connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .is_err()
            {
                return false;
            }
            let mut head = Vec::new();
            let mut byte = [0_u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                match connection.read(&mut byte) {
                    Ok(1) => head.push(byte[0]),
                    _ => return false,
                }
            }
            if head.starts_with(b"GET /api/v1/ws ") {
                // Ok only at end of stream; a timeout is an error.
                return connection.read_to_end(&mut Vec::new()).is_ok();
            }
            // A REST request: keep it open, unanswered.
            others.push(connection);
        }
    }

    #[test]
    fn signing_out_closes_the_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("listener");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let (report, closed) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = report.send(websocket_closed(&listener));
        });
        let mut h = Harness::new();
        h.sign_in_at(&url);
        // The socket task connects and waits for the upgrade answer.
        h.run_for(Duration::from_millis(300));
        h.feed(Input::Command(Command::SignOut));
        h.run_for(Duration::from_millis(300));
        assert_eq!(
            closed.recv_timeout(Duration::from_secs(10)),
            Ok(true),
            "signing out aborts the socket task, which closes the connection"
        );
    }

    #[test]
    fn messages_from_an_older_socket_are_ignored() {
        let mut h = Harness::new();
        let id = h.sign_in();
        let old = h.session().generation;
        let (sender, _queue) = mpsc::channel(4);
        h.socket(Socket::Open(old, sender));
        h.socket(Socket::Closed(old, "gone".into()));
        h.feed(Input::Rest(
            id,
            Rest::Reconnect {
                socket: old,
                result: Ok("fresh".into()),
            },
        ));
        let current = h.session().generation;
        assert_ne!(old, current);
        assert_eq!(h.session().api.token.as_deref(), Some("fresh"));
        h.events();
        let batch = || ServerMessage::TagUpdateBatch {
            updates: Vec::new(),
        };
        h.socket(Socket::Message(old, batch()));
        h.socket(Socket::Closed(old, "late close".into()));
        assert!(h.events().is_empty());
        h.socket(Socket::Message(current, batch()));
        assert!(matches!(h.events().as_slice(), [Event::Updates(_)]));
    }

    #[test]
    fn a_refresh_asked_for_during_a_fetch_runs_after_it() {
        let mut refresh = Refresh::default();
        assert!(refresh.request());
        assert!(!refresh.request());
        assert!(!refresh.request());
        assert!(
            refresh.finish(),
            "one more fetch for the requests meanwhile"
        );
        assert!(!refresh.finish());
        assert_eq!(refresh, Refresh::default());
    }

    #[test]
    fn the_actor_fetches_again_and_reports_a_failed_fetch() {
        let mut h = Harness::new();
        let id = h.sign_in();
        assert!(h.session().alarms.running, "signing in fetches the alarms");
        h.feed(Input::Command(Command::RefreshAlarms));
        assert!(h.session().alarms.dirty);
        h.feed(Input::Rest(id, Rest::Alarms(Ok(Vec::new()))));
        assert_eq!(
            h.session().alarms,
            Refresh {
                running: true,
                dirty: false
            }
        );
        h.feed(Input::Rest(id, Rest::Alarms(Err("HTTP 502".into()))));
        assert_eq!(h.session().alarms, Refresh::default());
        assert!(matches!(
            h.events().as_slice(),
            [Event::Alarms(_), Event::AlarmsFailed(error)] if error == "HTTP 502"
        ));
    }

    #[test]
    fn writes_match_only_their_own_acks_even_late_ones() {
        let mut writes = Writes::new("abc".into());
        let first = writes.issue(11);
        let second = writes.issue(12);
        assert_ne!(first, second);
        assert!(first.starts_with("native-abc-"));
        assert_eq!(writes.acknowledge("native-other-1"), None);
        assert_eq!(writes.time_out(&first), Some(11));
        assert_eq!(writes.time_out(&first), None, "reported once");
        assert_eq!(writes.acknowledge(&first), Some((11, true)));
        assert_eq!(writes.acknowledge(&first), None);
        assert_eq!(writes.abandon(), [12]);
        assert_eq!(writes.acknowledge(&second), None);
        assert_ne!(nonce(), nonce());
    }

    #[test]
    fn a_write_goes_out_on_the_socket_and_a_late_ack_corrects_its_timeout() {
        let mut h = Harness::new();
        let id = h.sign_in();
        let write = || Input::Command(Command::Write { id: 7, value: 1.5 });
        h.feed(write());
        let (sender, mut queue) = mpsc::channel(4);
        let generation = h.session().generation;
        h.socket(Socket::Open(generation, sender));
        h.feed(write());
        let Ok(ClientMessage::TagWrite {
            tag_id,
            value,
            command_id: Some(command_id),
        }) = queue.try_recv()
        else {
            panic!("expected a tag write on the socket");
        };
        assert_eq!((tag_id.as_str(), value), ("7", 1.5));
        assert!(command_id.starts_with(&format!("native-{}-", h.session().writes.nonce)));
        h.feed(Input::WriteTimeout(id, command_id.clone()));
        h.socket(Socket::Message(
            generation,
            ServerMessage::CommandAck {
                command_id,
                status: "ok".into(),
                device_response: None,
                latency_ms: Some(12),
            },
        ));
        let events = h.events();
        assert!(matches!(
            events.as_slice(),
            [
                Event::Written { result: Err(not_connected), late: false, .. },
                Event::Link(Link::Live),
                Event::Written { result: Err(_), late: false, .. },
                Event::Written { tag, result: Ok(detail), late: true },
            ] if not_connected == "Not connected" && *tag == 7 && detail == "ok (12 ms)"
        ));
    }

    #[test]
    fn a_refused_reconnect_ends_the_session_for_good() {
        let mut h = Harness::new();
        let id = h.sign_in();
        let generation = h.session().generation;
        h.socket(Socket::Closed(generation, "gone".into()));
        // A server that is merely unreachable is tried again.
        h.feed(Input::Rest(
            id,
            Rest::Reconnect {
                socket: generation,
                result: Err(unreachable()),
            },
        ));
        assert!(h.actor.session.is_some());
        h.feed(Input::Rest(
            id,
            Rest::Reconnect {
                socket: generation,
                result: Err(refused("Invalid credentials")),
            },
        ));
        assert!(h.actor.session.is_none(), "no more sign-in attempts");
        h.socket(Socket::Closed(generation, "gone".into()));
        h.feed(Input::Retry(generation));
        let events = h.events();
        assert!(is_failed(
            events.last(),
            "Not authorised: Invalid credentials"
        ));
    }

    #[test]
    fn a_refused_renewal_or_socket_token_ends_the_session() {
        let mut h = Harness::new();
        let id = h.sign_in();
        h.feed(Input::Rest(id, Rest::Renewed(Err(unreachable()))));
        assert!(h.actor.session.is_some(), "a missed renewal is tried again");
        h.feed(Input::Rest(id, Rest::Renewed(Ok("renewed".into()))));
        assert_eq!(h.session().api.token.as_deref(), Some("renewed"));
        h.feed(Input::Rest(
            id,
            Rest::Renewed(Err(refused("Account locked. Try again later."))),
        ));
        assert!(h.actor.session.is_none());
        assert!(is_failed(
            h.events().last(),
            "Account locked. Try again later. Sign in again."
        ));

        h.sign_in();
        let generation = h.session().generation;
        let (sender, _queue) = mpsc::channel(4);
        h.socket(Socket::Open(generation, sender));
        h.socket(Socket::Message(
            generation,
            ServerMessage::AuthError {
                message: "Invalid token".into(),
            },
        ));
        h.socket(Socket::Closed(generation, "closed".into()));
        assert!(h.actor.session.is_none());
        assert!(
            is_failed(h.events().last(), "Invalid token"),
            "not overwritten by the close"
        );
    }

    #[test]
    fn an_empty_current_password_means_the_sessions() {
        assert_eq!(current_password(String::new(), "signed-in"), "signed-in");
        assert_eq!(current_password("typed".into(), "signed-in"), "typed");
        let mut h = Harness::new();
        let id = h.sign_in();
        h.feed(Input::Rest(
            id,
            Rest::PasswordChanged {
                new: "rejected".into(),
                result: Err("Current password is incorrect".into()),
            },
        ));
        assert_eq!(h.session().password, "secret");
        h.feed(Input::Rest(
            id,
            Rest::PasswordChanged {
                new: "n3w-secret".into(),
                result: Ok(()),
            },
        ));
        assert_eq!(h.session().password, "n3w-secret", "used to sign in again");
        assert!(matches!(
            h.events().as_slice(),
            [
                Event::PasswordChanged(Err(_)),
                Event::PasswordChanged(Ok(()))
            ]
        ));
    }

    #[test]
    fn commands_keep_passwords_out_of_debug_output() {
        let sign_in = Command::SignIn {
            url: "http://plant".into(),
            username: "admin".into(),
            password: "hunter22".into(),
        };
        let change = Command::ChangePassword {
            current: "hunter22".into(),
            new: "hunter33".into(),
        };
        let text = format!("{sign_in:?} {change:?}");
        assert!(text.contains("admin"));
        assert!(!text.contains("hunter"));
    }

    /// The answer a spawned REST task sent back to the actor, if one did
    /// within a second.
    fn spawned_reply(h: &mut Harness) -> Option<(u64, CallKey, Reply)> {
        for _ in 0..100 {
            h.run_for(Duration::from_millis(10));
            while let Ok(input) = h.inputs.try_recv() {
                if let Input::Rest(session, Rest::Reply { key, reply }) = input {
                    return Some((session, key, reply));
                }
            }
        }
        None
    }

    #[test]
    fn a_generic_call_runs_with_the_session_and_answers_under_its_key() {
        let mut h = Harness::new();
        let id = h.sign_in();
        let key = CallKey::new("alarms.summary").with(3);
        assert_eq!(key.scope(), "alarms");
        h.feed(Input::Command(Command::Call {
            key,
            call: Call::new(|api| async move { Ok(format!("{} answered", api.base)) }),
        }));
        let (session, answered, reply) = spawned_reply(&mut h).expect("the call ran");
        assert_eq!((session, answered), (id, key));
        h.events();
        h.feed(Input::Rest(session, Rest::Reply { key, reply }));
        let Some(Event::Reply { key: got, reply }) = h.events().pop() else {
            panic!("expected the reply as an event");
        };
        assert_eq!(got, key);
        assert_eq!(
            reply.take::<String>().as_deref(),
            Ok("http://127.0.0.1:9 answered")
        );

        // A reply decoded as another type says so instead of panicking.
        let wrong = Reply::ok(5_i64).take::<String>();
        assert!(wrong.is_err_and(|error| error.contains("String")));
    }

    #[test]
    fn calls_need_a_session_and_an_ended_sessions_replies_are_dropped() {
        let mut h = Harness::new();
        let key = CallKey::new("commands.list");
        h.feed(Input::Command(Command::Call {
            key,
            call: Call::get::<serde_json::Value>("/api/v1/commands"),
        }));
        let Some(Event::Reply { reply, .. }) = h.events().pop() else {
            panic!("expected a failed reply");
        };
        assert_eq!(reply.error(), Some("Not signed in"));

        let old = h.sign_in();
        h.feed(Input::Command(Command::SignOut));
        h.sign_in();
        h.events();
        h.feed(Input::Rest(
            old,
            Rest::Reply {
                key,
                reply: Reply::ok(1_i64),
            },
        ));
        assert!(
            h.events().is_empty(),
            "the signed-out session's answer is dropped"
        );
    }

    #[test]
    fn device_status_messages_become_events() {
        let mut h = Harness::new();
        h.sign_in();
        let generation = h.session().generation;
        h.events();
        h.socket(Socket::Message(
            generation,
            ServerMessage::DeviceStatus {
                device_id: "inverter-2".into(),
                online: false,
                latency_ms: Some(40),
            },
        ));
        assert!(matches!(
            h.events().as_slice(),
            [Event::DeviceStatus { device_id, online: false, latency_ms: Some(40) }]
                if device_id == "inverter-2"
        ));
    }

    #[test]
    fn a_stress_test_acts_out_a_session_until_signed_out() {
        let mut h = Harness::new();
        h.sign_in();
        let options = StressOptions {
            tags: 50,
            rate: 5_000,
            seed: 1,
        };
        // `feed` runs inside the runtime, so the feed task spawns there.
        h.feed(Input::Command(Command::Stress(options)));
        assert!(
            h.actor.session.is_none(),
            "the stress test replaces the session"
        );
        h.run_for(Duration::from_millis(200));
        let events = h.events();
        assert!(matches!(
            &events[..4],
            [
                Event::SignedIn { username, server, .. },
                Event::Link(Link::Live),
                Event::Tags(tags),
                Event::Alarms(alarms),
            ] if username == STRESS_USER && server == STRESS_SERVER && tags.len() == 50 && alarms.is_empty()
        ));
        let updates: usize = events[4..]
            .iter()
            .map(|event| match event {
                Event::Updates(batch) => batch.len(),
                _ => 0,
            })
            .sum();
        assert!(updates >= 50, "batches of 50 every 10 ms, got {updates}");

        h.feed(Input::Command(Command::SignOut));
        h.run_for(Duration::from_millis(50));
        h.events();
        h.run_for(Duration::from_millis(100));
        assert!(h.events().is_empty(), "signing out stops the feed");
    }
}
