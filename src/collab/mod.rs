//! Live collaboration in the Designer (docs/collaboration.md): one Yjs
//! document per screen draft while anyone edits it, kept here as the
//! authority, synced with the clients over `/api/v1/ws` and written to the
//! draft rows through the same path as `PUT …/scada-layout`.
//!
//! * [`CollabManager`] holds the open [`Session`]s (one per screen) and the
//!   connections in them. A background task (started with the first session)
//!   writes dirty documents every [`FLUSH_AFTER`], merges writes made outside
//!   the session, and closes sessions nobody used for [`IDLE_CLOSE`].
//! * Every write to a screen with an open session goes through the session's
//!   write lock ([`CollabManager::before_write`]): the session's pending edits
//!   are written first, the writer writes, and its difference is applied to
//!   the document ([`WriteGuard::absorb`]).

pub mod validate;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use voltrus_collab::document::{self, ConnectionRow, ObjectKind, Rows, WidgetRow};
use voltrus_collab::wire::{self, Handled, Presence};
use voltrus_collab::yrs::types::{DeepObservable, Event, PathSegment};
use voltrus_collab::yrs::{Doc, Transact, WriteTxn};

use crate::api::AppState;
use crate::models::{ScadaConnection, ScadaEquipment};
use crate::storage::design_history::{self as h, DesignActor};
use crate::ws_types::{
    DesignLimits, DesignRejectedObject, DesignSessionInfo, DesignUser, ServerMessage,
};

/// Largest sync message, bytes.
pub const MAX_UPDATE: usize = 256 * 1024;
/// Most sync messages a connection sends per second.
pub const MAX_RATE: u32 = 50;
/// Largest document, bytes (its encoded state, estimated between writes).
pub const MAX_DOC: usize = 5 * 1024 * 1024;
/// Largest awareness state of one client, bytes.
pub const MAX_AWARENESS: usize = 4096;
/// Edits wait at most this long before they are written to the draft.
pub const FLUSH_AFTER: Duration = Duration::from_secs(2);
/// How often the background task looks at the sessions.
pub const TICK: Duration = Duration::from_millis(500);
/// A session nobody is in is closed after this long (a reconnect within it
/// keeps its epoch, so offline edits merge).
pub const IDLE_CLOSE: Duration = Duration::from_secs(60);
/// Awareness client ids one connection may use (a reset document gets a new one).
const MAX_CLIENTS_PER_CONN: usize = 4;
/// The origin of changes the server makes.
const SERVER_ORIGIN: &str = "server";

/// Colours of people's cursors, picked by name.
const COLOURS: &[&str] = &[
    "#e5484d", "#3e63dd", "#30a46c", "#f76b15", "#8e4ec6", "#12a594", "#d6409f", "#0090ff",
    "#ad7f58", "#46a758",
];

/// A user's colour: the same on every screen and every client.
pub fn colour_of(user: &str) -> String {
    let mut hash: u32 = 2_166_136_261;
    for b in user.bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(16_777_619);
    }
    COLOURS[hash as usize % COLOURS.len()].to_string()
}

pub fn limits() -> DesignLimits {
    DesignLimits {
        max_update: MAX_UPDATE,
        max_rate: MAX_RATE,
        max_doc: MAX_DOC,
        max_awareness: MAX_AWARENESS,
        flush_ms: FLUSH_AFTER.as_millis() as u64,
    }
}

fn origin_of(conn: &str) -> String {
    format!("conn:{conn}")
}

fn conn_of_origin(origin: &[u8]) -> Option<&str> {
    std::str::from_utf8(origin).ok()?.strip_prefix("conn:")
}

// ── Rows ──

pub(crate) fn rows_of(eq: &[ScadaEquipment], cn: &[ScadaConnection]) -> Rows {
    Rows {
        widgets: eq
            .iter()
            .map(|e| WidgetRow {
                id: e.equipment_id.clone(),
                kind: e.equipment_type.clone(),
                name: e.equipment_name.clone(),
                x: e.x,
                y: e.y,
                width: e.width,
                height: e.height,
                rotation: e.rotation,
                sensors: e.sensor_bindings.clone(),
                config: e.config.clone(),
            })
            .collect(),
        connections: cn
            .iter()
            .map(|c| ConnectionRow {
                id: c.connection_id.clone(),
                from: c.from_equipment.clone(),
                to: c.to_equipment.clone(),
                kind: c.connection_type.clone(),
                waypoints: c.waypoints.clone(),
            })
            .collect(),
    }
}

pub(crate) fn models_of(screen: &str, rows: &Rows) -> (Vec<ScadaEquipment>, Vec<ScadaConnection>) {
    (
        rows.widgets
            .iter()
            .map(|w| ScadaEquipment {
                screen_id: screen.to_string(),
                equipment_id: w.id.clone(),
                equipment_type: w.kind.clone(),
                equipment_name: w.name.clone(),
                x: w.x,
                y: w.y,
                width: w.width,
                height: w.height,
                rotation: w.rotation,
                sensor_bindings: w.sensors.clone(),
                config: w.config.clone(),
            })
            .collect(),
        rows.connections
            .iter()
            .map(|c| ScadaConnection {
                screen_id: screen.to_string(),
                connection_id: c.id.clone(),
                from_equipment: c.from.clone(),
                to_equipment: c.to.clone(),
                waypoints: c.waypoints.clone(),
                connection_type: c.kind.clone(),
            })
            .collect(),
    )
}

fn sort_rows(rows: &mut Rows) {
    rows.widgets.sort_by(|a, b| a.id.cmp(&b.id));
    rows.connections.sort_by(|a, b| a.id.cmp(&b.id));
}

/// Read a screen's draft rows and change-log head in one transaction.
fn read_rows(conn: &rusqlite::Connection, screen: &str) -> anyhow::Result<(Rows, i64)> {
    let draft = h::read_draft(conn, screen)?;
    let mut rows = rows_of(&draft.equipment, &draft.connections);
    sort_rows(&mut rows);
    Ok((rows, h::head_seq(conn, screen)?))
}

// ── Sessions ──

/// One person's connection in a session.
struct Member {
    user: String,
    colour: String,
    /// Latest awareness entry per Yjs client id of this connection.
    presence: BTreeMap<u64, Presence>,
    /// Times of the last sync messages (rate limit).
    recent: VecDeque<Instant>,
}

/// Mutable state of a session (behind a std mutex, never held across an await).
struct Inner {
    doc: Doc,
    members: BTreeMap<String, Member>,
    /// The rows as last read from or written to the draft.
    synced: Rows,
    /// The screen's change-log head as of `synced`.
    synced_seq: i64,
    /// Edits since `synced` (set by every applied client update).
    dirty_since: Option<Instant>,
    /// Bumped by every applied client update.
    edits: u64,
    /// Encoded size of the document as of the last measure, plus updates since.
    doc_size: usize,
    /// When the last member left.
    empty_since: Option<Instant>,
    /// [`h::DESIGN_WRITES`] when the session last compared its head.
    seen_writes: u64,
    /// Connections whose outgoing queue was full: told to join again.
    stale: BTreeSet<String>,
    closed: bool,
}

/// Updates (origin, bytes) and touched objects (origin, kind, id) the
/// document's observers collected during a transaction.
#[derive(Default)]
struct Outbox {
    updates: Vec<(Vec<u8>, Vec<u8>)>,
    touched: Vec<(Vec<u8>, ObjectKind, String)>,
}

/// The live document of one screen.
pub struct Session {
    pub screen: String,
    pub epoch: String,
    /// The per-screen write lock: flushes and every other write to the
    /// screen's rows while the session is open.
    write: Arc<tokio::sync::Mutex<()>>,
    inner: Mutex<Inner>,
    outbox: Arc<Mutex<Outbox>>,
    /// Who touched which object since the last write: (kind, id) -> user.
    authors: Mutex<BTreeMap<(ObjectKind, String), String>>,
}

fn new_epoch() -> String {
    let n: u64 = rand::random();
    format!("{:x}-{n:016x}", crate::timeutil::now_millis())
}

impl Session {
    fn new(screen: &str, rows: Rows, seq: i64) -> Arc<Session> {
        let doc = Doc::new();
        {
            let mut txn = doc.transact_mut_with(SERVER_ORIGIN);
            document::load(&mut txn, &rows);
        }
        let outbox = Arc::new(Mutex::new(Outbox::default()));
        let ob = outbox.clone();
        let _ = doc.observe_update_v1("collab", move |txn, e| {
            let origin = txn
                .origin()
                .map(|o| o.as_ref().to_vec())
                .unwrap_or_default();
            if let Ok(mut ob) = ob.lock() {
                ob.updates.push((origin, e.update.clone()));
            }
        });
        let mut txn = doc.transact_mut();
        for (root, kind) in [
            (document::WIDGETS, ObjectKind::Widget),
            (document::CONNECTIONS, ObjectKind::Connection),
        ] {
            let map = txn.get_or_insert_map(root);
            let ob = outbox.clone();
            map.observe_deep("collab", move |txn, events| {
                let origin = txn
                    .origin()
                    .map(|o| o.as_ref().to_vec())
                    .unwrap_or_default();
                let Ok(mut ob) = ob.lock() else { return };
                for e in events.iter() {
                    match e.path().front() {
                        Some(PathSegment::Key(id)) => {
                            ob.touched.push((origin.clone(), kind, id.to_string()))
                        }
                        Some(PathSegment::Index(_)) => {}
                        None => {
                            if let Event::Map(m) = e {
                                for k in m.keys(txn).keys() {
                                    ob.touched.push((origin.clone(), kind, k.to_string()));
                                }
                            }
                        }
                    }
                }
            });
        }
        drop(txn);
        let doc_size = wire::state(&doc).len();
        outbox.lock().map(|mut o| *o = Outbox::default()).ok();
        Arc::new(Session {
            screen: screen.to_string(),
            epoch: new_epoch(),
            write: Arc::new(tokio::sync::Mutex::new(())),
            inner: Mutex::new(Inner {
                doc,
                members: BTreeMap::new(),
                synced: rows,
                synced_seq: seq,
                dirty_since: None,
                edits: 0,
                doc_size,
                empty_since: Some(Instant::now()),
                seen_writes: h::DESIGN_WRITES.load(Ordering::Relaxed),
                stale: BTreeSet::new(),
                closed: false,
            }),
            outbox,
            authors: Mutex::new(BTreeMap::new()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The people in the session.
    pub fn users(&self) -> Vec<DesignUser> {
        let inner = self.lock();
        let mut seen = BTreeMap::new();
        for m in inner.members.values() {
            seen.entry(m.user.clone())
                .or_insert_with(|| m.colour.clone());
        }
        seen.into_iter()
            .map(|(user, color)| DesignUser { user, color })
            .collect()
    }

    /// The server's document (tests).
    #[doc(hidden)]
    pub fn doc_for_tests(&self) -> Doc {
        self.lock().doc.clone()
    }

    /// Pretend the last member left `ago` (tests).
    #[doc(hidden)]
    pub fn age_for_tests(&self, ago: Duration) {
        self.lock().empty_since = Instant::now().checked_sub(ago);
    }

    /// The connections in the session.
    pub fn member_conns(&self) -> Vec<String> {
        self.lock().members.keys().cloned().collect()
    }

    /// The document as rows now (for tests and diagnostics).
    pub fn rows(&self) -> Rows {
        let inner = self.lock();
        let screen = document::read(&inner.doc.transact());
        screen.to_rows(&inner.synced)
    }

    /// Take what the observers collected: record who touched what, and send
    /// the updates to the members (not back to the connection they came from).
    fn drain(&self, inner: &mut Inner, ws: &crate::ws_manager::WsManager) {
        let ob = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|e| e.into_inner()));
        if !ob.touched.is_empty() {
            let mut authors = self.authors.lock().unwrap_or_else(|e| e.into_inner());
            for (origin, kind, id) in ob.touched {
                let user = match conn_of_origin(&origin).and_then(|c| inner.members.get(c)) {
                    Some(m) => m.user.clone(),
                    None if origin == SERVER_ORIGIN.as_bytes() => continue,
                    None => "system".to_string(),
                };
                authors.insert((kind, id), user);
            }
        }
        for (origin, update) in ob.updates {
            let from = conn_of_origin(&origin).map(str::to_string);
            let msg = ServerMessage::DesignSync {
                screen: self.screen.clone(),
                data: wire::update(update),
            };
            let targets: Vec<String> = inner
                .members
                .keys()
                .filter(|c| Some(c.as_str()) != from.as_deref())
                .cloned()
                .collect();
            for conn in targets {
                send(inner, ws, &conn, msg.clone());
            }
        }
    }
}

/// Send to a member; a full queue makes it stale (it is told to join again).
fn send(inner: &mut Inner, ws: &crate::ws_manager::WsManager, conn: &str, msg: ServerMessage) {
    if inner.stale.contains(conn) {
        return;
    }
    if !ws.send_to(conn, msg) {
        inner.stale.insert(conn.to_string());
    }
}

/// What [`CollabManager::flush`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flushed {
    /// The change-log head after the write.
    pub seq: i64,
    /// Users whose changes were written.
    pub users: Vec<String>,
    /// Objects put back by validation.
    pub rejected: Vec<DesignRejectedObject>,
}

/// The open sessions of a server.
#[derive(Default)]
pub struct CollabManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Connection -> screens it is in.
    conns: Mutex<HashMap<String, BTreeSet<String>>>,
    /// Serialises session creation.
    opening: tokio::sync::Mutex<()>,
    ticker: std::sync::Once,
}

impl std::fmt::Debug for CollabManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollabManager")
            .field(
                "sessions",
                &self.sessions.lock().map(|s| s.len()).unwrap_or(0),
            )
            .finish()
    }
}

/// Holds a screen's write lock while another writer writes (REST, MCP,
/// revert, publish). [`WriteGuard::absorb`] then applies what it wrote to the
/// document.
pub struct WriteGuard {
    session: Arc<Session>,
    _lock: tokio::sync::OwnedMutexGuard<()>,
}

impl WriteGuard {
    /// Merge the rows as they are now into the document (as a server change)
    /// and release the lock.
    pub async fn absorb(self, state: &AppState) {
        if let Err(e) = absorb(state, &self.session).await {
            tracing::warn!(
                "collab: merging a write into screen {} failed: {e:#}",
                self.session.screen
            );
        }
    }
}

/// Bring a session's document up to the rows: when the change log moved since
/// the session last wrote or read them, apply the difference as a server change.
async fn absorb(state: &AppState, session: &Arc<Session>) -> anyhow::Result<bool> {
    let screen = session.screen.clone();
    let (rows, seq) = state
        .storage
        .design_tx(move |c| read_rows(c, &screen))
        .await?;
    let mut inner = session.lock();
    if seq == inner.synced_seq && rows == inner.synced {
        return Ok(false);
    }
    let before = std::mem::replace(&mut inner.synced, rows.clone());
    inner.synced_seq = seq;
    {
        let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
        document::apply_rows(&mut txn, &before, &rows);
    }
    session.drain(&mut inner, &state.ws_manager);
    Ok(true)
}

/// One flush step's write: a batch per author, in one transaction, based on
/// `base_seq`. `None` when the change log moved (someone wrote outside).
fn write_batches(
    conn: &rusqlite::Connection,
    screen: &str,
    base_seq: i64,
    synced: &Rows,
    target: &Rows,
    groups: &[(String, BTreeSet<(ObjectKind, String)>)],
) -> anyhow::Result<Option<(Rows, i64)>> {
    if h::head_seq(conn, screen)? != base_seq {
        return Ok(None);
    }
    let mut cur = synced.clone();
    let want_w: BTreeMap<&str, &WidgetRow> =
        target.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    let want_c: BTreeMap<&str, &ConnectionRow> = target
        .connections
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    for (user, objects) in groups {
        for (kind, id) in objects {
            match kind {
                ObjectKind::Widget => {
                    cur.widgets.retain(|w| &w.id != id);
                    if let Some(w) = want_w.get(id.as_str()) {
                        cur.widgets.push((*w).clone());
                    }
                }
                ObjectKind::Connection => {
                    cur.connections.retain(|c| &c.id != id);
                    if let Some(c) = want_c.get(id.as_str()) {
                        cur.connections.push((*c).clone());
                    }
                }
            }
        }
        sort_rows(&mut cur);
        let (eq, cn) = models_of(screen, &cur);
        let actor = DesignActor::new(user.clone(), "designer", "save").with_note("Live session");
        crate::storage::write_layout_rows(conn, screen, Some(eq), Some(cn), &actor)?;
    }
    let (rows, seq) = read_rows(conn, screen)?;
    Ok(Some((rows, seq)))
}

impl CollabManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The open session of a screen.
    pub fn session(&self, screen: &str) -> Option<Arc<Session>> {
        self.sessions.lock().ok()?.get(screen).cloned()
    }

    fn all(&self) -> Vec<Arc<Session>> {
        self.sessions
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Who edits which screen.
    pub fn sessions_info(&self) -> Vec<DesignSessionInfo> {
        let mut out: Vec<DesignSessionInfo> = self
            .all()
            .into_iter()
            .map(|s| DesignSessionInfo {
                screen: s.screen.clone(),
                users: s.users(),
            })
            .filter(|s| !s.users.is_empty())
            .collect();
        out.sort_by(|a, b| a.screen.cmp(&b.screen));
        out
    }

    fn broadcast_sessions(&self, state: &AppState) {
        let msg = ServerMessage::DesignSessions {
            sessions: self.sessions_info(),
        };
        let conns: Vec<String> = self
            .conns
            .lock()
            .map(|c| c.keys().cloned().collect())
            .unwrap_or_default();
        for c in conns {
            state.ws_manager.send_to(&c, msg.clone());
        }
    }

    fn error(state: &AppState, conn: &str, screen: &str, code: &str, message: &str, reload: bool) {
        state.ws_manager.send_to(
            conn,
            ServerMessage::DesignError {
                screen: screen.to_string(),
                code: code.to_string(),
                message: message.to_string(),
                reload,
            },
        );
    }

    fn start_ticker(&self, state: &AppState) {
        self.ticker.call_once(|| {
            let state = state.clone();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(TICK);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tick.tick().await;
                    state.collab.tick(&state).await;
                }
            });
        });
    }

    async fn open(&self, state: &AppState, screen: &str) -> anyhow::Result<Arc<Session>> {
        if let Some(s) = self.session(screen) {
            return Ok(s);
        }
        let _opening = self.opening.lock().await;
        if let Some(s) = self.session(screen) {
            return Ok(s);
        }
        let id = screen.to_string();
        let (rows, seq) = state.storage.design_tx(move |c| read_rows(c, &id)).await?;
        let session = Session::new(screen, rows, seq);
        self.sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("collab sessions poisoned"))?
            .insert(screen.to_string(), session.clone());
        self.start_ticker(state);
        tracing::info!("collab: session opened for screen {screen}");
        Ok(session)
    }

    /// `DesignJoin`. The caller checked the role.
    pub async fn join(
        &self,
        state: &AppState,
        conn: &str,
        user: &str,
        screen: &str,
        protocol: u32,
        epoch: Option<&str>,
    ) {
        if protocol != voltrus_collab::PROTOCOL {
            Self::error(
                state,
                conn,
                screen,
                "protocol",
                &format!(
                    "this server speaks collaboration protocol {}, the client {protocol}: reload the page",
                    voltrus_collab::PROTOCOL
                ),
                false,
            );
            return;
        }
        if screen.is_empty() || screen.len() > 128 {
            Self::error(state, conn, screen, "bad_message", "bad screen id", false);
            return;
        }
        match state.storage.design_settings().await {
            Ok(s) if s.collaboration => {}
            Ok(_) => {
                Self::error(
                    state,
                    conn,
                    screen,
                    "disabled",
                    "live collaboration is turned off on this server",
                    false,
                );
                return;
            }
            Err(e) => {
                tracing::warn!("collab: settings not readable: {e:#}");
                Self::error(
                    state,
                    conn,
                    screen,
                    "disabled",
                    "settings not readable",
                    false,
                );
                return;
            }
        }
        let session = match self.open(state, screen).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("collab: screen {screen} not opened: {e:#}");
                Self::error(
                    state,
                    conn,
                    screen,
                    "bad_message",
                    "the screen could not be opened",
                    false,
                );
                return;
            }
        };
        {
            let mut inner = session.lock();
            if inner.closed {
                drop(inner);
                Self::error(
                    state,
                    conn,
                    screen,
                    "resync",
                    "the session closed; join again",
                    false,
                );
                return;
            }
            let colour = colour_of(user);
            // A connection joining again replaces its old membership.
            let old = inner.members.insert(
                conn.to_string(),
                Member {
                    user: user.to_string(),
                    colour: colour.clone(),
                    presence: BTreeMap::new(),
                    recent: VecDeque::new(),
                },
            );
            inner.stale.remove(conn);
            inner.empty_since = None;
            let resync = epoch.is_some_and(|e| e != session.epoch);
            send(
                &mut inner,
                &state.ws_manager,
                conn,
                ServerMessage::DesignJoined {
                    screen: screen.to_string(),
                    epoch: session.epoch.clone(),
                    resync,
                    user: user.to_string(),
                    color: colour,
                    protocol: voltrus_collab::PROTOCOL,
                    limits: limits(),
                },
            );
            let step1 = wire::step1(&inner.doc);
            send(
                &mut inner,
                &state.ws_manager,
                conn,
                ServerMessage::DesignSync {
                    screen: screen.to_string(),
                    data: step1,
                },
            );
            let others: Vec<Presence> = inner
                .members
                .iter()
                .filter(|(c, _)| c.as_str() != conn)
                .flat_map(|(_, m)| m.presence.values().cloned())
                .collect();
            if !others.is_empty() {
                send(
                    &mut inner,
                    &state.ws_manager,
                    conn,
                    ServerMessage::DesignAwareness {
                        screen: screen.to_string(),
                        data: wire::encode_awareness(&others),
                    },
                );
            }
            if let Some(old) = old {
                let gone = removals(&old);
                if !gone.is_empty() {
                    broadcast_awareness(&mut inner, state, screen, conn, &gone);
                }
            }
        }
        self.conns
            .lock()
            .map(|mut c| {
                c.entry(conn.to_string())
                    .or_default()
                    .insert(screen.to_string());
            })
            .ok();
        tracing::info!("collab: {user} joined screen {screen} (conn {conn})");
        self.broadcast_sessions(state);
    }

    fn member_session(&self, state: &AppState, conn: &str, screen: &str) -> Option<Arc<Session>> {
        let s = self.session(screen);
        let joined = s.as_ref().is_some_and(|s| {
            let inner = s.lock();
            inner.members.contains_key(conn) && !inner.stale.contains(conn)
        });
        if !joined {
            Self::error(
                state,
                conn,
                screen,
                "not_joined",
                "join the screen's session first",
                false,
            );
            return None;
        }
        s
    }

    /// Drop a connection from a session because of what it sent.
    fn expel(
        &self,
        state: &AppState,
        session: &Arc<Session>,
        conn: &str,
        code: &str,
        message: &str,
        reload: bool,
    ) {
        tracing::warn!(
            "collab: {conn} left screen {}: {code}: {message}",
            session.screen
        );
        Self::error(state, conn, &session.screen, code, message, reload);
        self.remove_member(state, session, conn);
    }

    /// `DesignSync`.
    pub async fn sync(&self, state: &AppState, conn: &str, screen: &str, data: &[u8]) {
        let Some(session) = self.member_session(state, conn, screen) else {
            return;
        };
        if data.len() > MAX_UPDATE {
            self.expel(
                state,
                &session,
                conn,
                "too_large",
                &format!(
                    "a change of {} bytes is over the limit of {MAX_UPDATE}",
                    data.len()
                ),
                true,
            );
            return;
        }
        let msg = match wire::decode(data) {
            Ok(m) => m,
            Err(e) => {
                self.expel(state, &session, conn, "bad_message", &format!("{e}"), true);
                return;
            }
        };
        let is_update = !matches!(msg, wire::Sync::SyncStep1(_));
        let refusal = {
            let mut inner = session.lock();
            let now = Instant::now();
            let Some(m) = inner.members.get_mut(conn) else {
                return;
            };
            while m
                .recent
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(1))
            {
                m.recent.pop_front();
            }
            m.recent.push_back(now);
            let user = m.user.clone();
            if m.recent.len() > MAX_RATE as usize {
                Some((
                    "rate_limited",
                    format!("more than {MAX_RATE} changes a second"),
                    false,
                ))
            } else if is_update && inner.doc_size + data.len() > MAX_DOC {
                Some((
                    "doc_too_large",
                    format!("the screen document would be over {MAX_DOC} bytes"),
                    true,
                ))
            } else {
                match wire::handle(&inner.doc, &origin_of(conn), msg) {
                    Ok(Handled::Reply(r)) => {
                        send(
                            &mut inner,
                            &state.ws_manager,
                            conn,
                            ServerMessage::DesignSync {
                                screen: screen.to_string(),
                                data: r,
                            },
                        );
                        None
                    }
                    Ok(Handled::Applied) => {
                        let changed = !session
                            .outbox
                            .lock()
                            .map(|o| o.updates.is_empty())
                            .unwrap_or(true);
                        if changed {
                            inner.doc_size += data.len();
                            inner.edits += 1;
                            inner.dirty_since.get_or_insert(now);
                        }
                        session.drain(&mut inner, &state.ws_manager);
                        tracing::trace!("collab: update from {user} on {screen}");
                        None
                    }
                    Err(e) => Some(("bad_message", e.to_string(), true)),
                }
            }
        };
        if let Some((code, message, reload)) = refusal {
            self.expel(state, &session, conn, code, &message, reload);
        }
    }

    /// `DesignAwareness`: checked, stamped with the member's name and colour,
    /// kept for newcomers and forwarded.
    pub fn awareness(&self, state: &AppState, conn: &str, screen: &str, data: &[u8]) {
        let Some(session) = self.member_session(state, conn, screen) else {
            return;
        };
        if data.len() > MAX_AWARENESS * 2 {
            return;
        }
        let Ok(entries) = wire::decode_awareness(data) else {
            return;
        };
        let mut inner = session.lock();
        let Some(m) = inner.members.get_mut(conn) else {
            return;
        };
        let mut out = Vec::new();
        for mut e in entries {
            let known = m.presence.contains_key(&e.client);
            if !known && m.presence.len() >= MAX_CLIENTS_PER_CONN {
                continue;
            }
            if e.json == "null" {
                m.presence.remove(&e.client);
                out.push(e);
                continue;
            }
            let Ok(serde_json::Value::Object(mut state_json)) =
                serde_json::from_str::<serde_json::Value>(&e.json)
            else {
                continue;
            };
            state_json.insert(
                "user".into(),
                serde_json::json!({"name": m.user, "color": m.colour}),
            );
            e.json = serde_json::Value::Object(state_json).to_string();
            if e.json.len() > MAX_AWARENESS {
                continue;
            }
            m.presence.insert(e.client, e.clone());
            out.push(e);
        }
        if !out.is_empty() {
            broadcast_awareness(&mut inner, state, screen, conn, &out);
        }
    }

    fn remove_member(&self, state: &AppState, session: &Arc<Session>, conn: &str) {
        let flush_now = {
            let mut inner = session.lock();
            let Some(old) = inner.members.remove(conn) else {
                return;
            };
            inner.stale.remove(conn);
            let gone = removals(&old);
            if !gone.is_empty() {
                broadcast_awareness(&mut inner, state, &session.screen, conn, &gone);
            }
            if inner.members.is_empty() {
                inner.empty_since = Some(Instant::now());
            }
            inner.members.is_empty() && inner.dirty_since.is_some()
        };
        self.conns
            .lock()
            .map(|mut c| {
                if let Some(s) = c.get_mut(conn) {
                    s.remove(&session.screen);
                    if s.is_empty() {
                        c.remove(conn);
                    }
                }
            })
            .ok();
        self.broadcast_sessions(state);
        if flush_now {
            // The last one left: write their edits now.
            let state2 = state.clone();
            let screen = session.screen.clone();
            tokio::spawn(async move {
                if let Err(e) = state2.collab.flush(&state2, &screen).await {
                    tracing::warn!(
                        "collab: writing screen {screen} after the last leave failed: {e:#}"
                    );
                }
            });
        }
    }

    /// `DesignLeave`.
    pub fn leave(&self, state: &AppState, conn: &str, screen: &str) {
        if let Some(s) = self.session(screen) {
            self.remove_member(state, &s, conn);
        }
    }

    /// A socket closed: leave every session.
    pub fn disconnect(&self, state: &AppState, conn: &str) {
        let screens = self
            .conns
            .lock()
            .ok()
            .and_then(|mut c| c.remove(conn))
            .unwrap_or_default();
        for screen in screens {
            if let Some(s) = self.session(&screen) {
                self.remove_member(state, &s, conn);
            }
        }
    }

    /// `DesignFlush`: write now.
    pub async fn flush_request(&self, state: &AppState, conn: &str, screen: &str) {
        if self.member_session(state, conn, screen).is_none() {
            return;
        }
        if let Err(e) = self.flush(state, screen).await {
            tracing::warn!("collab: writing screen {screen} failed: {e:#}");
            Self::error(
                state,
                conn,
                screen,
                "write_failed",
                &format!("{e:#}"),
                false,
            );
        }
    }

    /// Before another writer writes `screen`: when a session is open, take its
    /// write lock and write its pending edits first. Call
    /// [`WriteGuard::absorb`] after the write.
    pub async fn before_write(&self, state: &AppState, screen: &str) -> Option<WriteGuard> {
        let session = self.session(screen)?;
        let lock = session.write.clone().lock_owned().await;
        if let Err(e) = flush_locked(state, &session).await {
            tracing::warn!("collab: writing screen {screen} before another write failed: {e:#}");
        }
        Some(WriteGuard {
            session,
            _lock: lock,
        })
    }

    /// Write every open session's pending edits (before a project import, at
    /// shutdown).
    pub async fn flush_all(&self, state: &AppState) {
        for s in self.all() {
            let _lock = s.write.lock().await;
            if let Err(e) = flush_locked(state, &s).await {
                tracing::warn!("collab: writing screen {} failed: {e:#}", s.screen);
            }
        }
    }

    /// Merge the rows into every open session (after a project import).
    pub async fn absorb_all(&self, state: &AppState) {
        for s in self.all() {
            let _lock = s.write.lock().await;
            if let Err(e) = absorb(state, &s).await {
                tracing::warn!("collab: merging into screen {} failed: {e:#}", s.screen);
            }
        }
    }

    /// Close every session (collaboration turned off): their edits are
    /// written, the members told `code`.
    pub async fn close_all(&self, state: &AppState, code: &str, message: &str) {
        self.flush_all(state).await;
        let sessions: Vec<Arc<Session>> = self
            .sessions
            .lock()
            .map(|mut s| s.drain().map(|(_, v)| v).collect())
            .unwrap_or_default();
        for s in sessions {
            let members: Vec<String> = {
                let mut inner = s.lock();
                inner.closed = true;
                std::mem::take(&mut inner.members).into_keys().collect()
            };
            for conn in members {
                Self::error(state, &conn, &s.screen, code, message, false);
            }
        }
        self.conns.lock().map(|mut c| c.clear()).ok();
    }

    /// Write a screen's pending edits now, under its write lock.
    pub async fn flush(&self, state: &AppState, screen: &str) -> anyhow::Result<Option<Flushed>> {
        let Some(session) = self.session(screen) else {
            return Ok(None);
        };
        let _lock = session.write.lock().await;
        flush_locked(state, &session).await
    }

    /// The background pass: flush, merge outside writes, retry stale
    /// members, close idle sessions.
    pub async fn tick(&self, state: &AppState) {
        let writes = h::DESIGN_WRITES.load(Ordering::Relaxed);
        for s in self.all() {
            let (due, moved, idle, stale) = {
                let mut inner = s.lock();
                let due = inner
                    .dirty_since
                    .is_some_and(|t| t.elapsed() >= FLUSH_AFTER)
                    || (inner.members.is_empty() && inner.dirty_since.is_some());
                let moved = inner.seen_writes != writes;
                inner.seen_writes = writes;
                let idle = inner.members.is_empty()
                    && inner.empty_since.is_some_and(|t| t.elapsed() >= IDLE_CLOSE);
                let stale: Vec<String> = inner.stale.iter().cloned().collect();
                (due, moved, idle, stale)
            };
            for conn in stale {
                // Tell it to join again; it stays stale until that goes out.
                let told = state.ws_manager.send_to(
                    &conn,
                    ServerMessage::DesignError {
                        screen: s.screen.clone(),
                        code: "resync".into(),
                        message: "some changes could not be delivered; join again".into(),
                        reload: false,
                    },
                );
                if told || !state.ws_manager.has_connection(&conn) {
                    self.remove_member(state, &s, &conn);
                }
            }
            if due || moved {
                let _lock = s.write.lock().await;
                let result = if due {
                    flush_locked(state, &s).await.map(|_| ())
                } else {
                    absorb(state, &s).await.map(|_| ())
                };
                if let Err(e) = result {
                    tracing::warn!("collab: screen {}: {e:#}", s.screen);
                }
            }
            if idle {
                let _lock = s.write.lock().await;
                let still_idle = {
                    let inner = s.lock();
                    inner.members.is_empty() && inner.dirty_since.is_none()
                };
                if still_idle {
                    s.lock().closed = true;
                    self.sessions
                        .lock()
                        .map(|mut m| {
                            if m.get(&s.screen).is_some_and(|x| Arc::ptr_eq(x, &s)) {
                                m.remove(&s.screen);
                            }
                        })
                        .ok();
                    tracing::info!("collab: session closed for screen {} (idle)", s.screen);
                }
            }
        }
    }
}

/// Awareness entries that remove a member's clients (clock + 1, state null).
fn removals(m: &Member) -> Vec<Presence> {
    m.presence
        .values()
        .map(|p| Presence {
            client: p.client,
            clock: p.clock.saturating_add(1),
            json: "null".into(),
        })
        .collect()
}

fn broadcast_awareness(
    inner: &mut Inner,
    state: &AppState,
    screen: &str,
    from: &str,
    entries: &[Presence],
) {
    let msg = ServerMessage::DesignAwareness {
        screen: screen.to_string(),
        data: wire::encode_awareness(entries),
    };
    let targets: Vec<String> = inner
        .members
        .keys()
        .filter(|c| c.as_str() != from)
        .cloned()
        .collect();
    for c in targets {
        send(inner, &state.ws_manager, &c, msg.clone());
    }
}

/// Write the session's document to the draft rows (the caller holds the
/// write lock): merge outside writes first, validate the changed objects
/// (putting back those that fail), write one batch per author, and apply what
/// the write changed (equipment binding) back to the document.
async fn flush_locked(state: &AppState, session: &Arc<Session>) -> anyhow::Result<Option<Flushed>> {
    let screen = session.screen.clone();
    let mut rejected_all: Vec<DesignRejectedObject> = Vec::new();
    for _attempt in 0..4 {
        // Outside writes since the last write come in first.
        let id = screen.clone();
        let head = state
            .storage
            .design_tx(move |c| h::head_seq(c, &id))
            .await?;
        if head != session.lock().synced_seq {
            absorb(state, session).await?;
        }
        let (target, synced, synced_seq, edits) = {
            let inner = session.lock();
            if inner.dirty_since.is_none() {
                return Ok(None);
            }
            let doc_screen = document::read(&inner.doc.transact());
            let mut target = doc_screen.to_rows(&inner.synced);
            sort_rows(&mut target);
            (target, inner.synced.clone(), inner.synced_seq, inner.edits)
        };
        if target == synced {
            let mut inner = session.lock();
            if inner.edits == edits {
                inner.dirty_since = None;
            }
            session.authors.lock().map(|mut a| a.clear()).ok();
            return Ok(None);
        }
        // Validation: objects that fail are put back, then the rows are read again.
        let ctx = state.storage.design_tx(validate::Ctx::load).await?;
        let problems = validate::check(&synced, &target, &ctx);
        if !problems.is_empty() {
            let rejected = quarantine(state, session, &synced, &problems);
            rejected_all.extend(rejected);
            continue;
        }
        // One batch per author: who last touched each changed object.
        let changed = changed_objects(&synced, &target);
        let authors = session
            .authors
            .lock()
            .map(|a| a.clone())
            .unwrap_or_default();
        let mut groups: BTreeMap<String, BTreeSet<(ObjectKind, String)>> = BTreeMap::new();
        let fallback = authors
            .values()
            .last()
            .cloned()
            .unwrap_or_else(|| "system".into());
        for obj in &changed {
            let user = authors
                .get(obj)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            groups.entry(user).or_default().insert(obj.clone());
        }
        let groups: Vec<(String, BTreeSet<(ObjectKind, String)>)> = groups.into_iter().collect();
        let users: Vec<String> = groups.iter().map(|(u, _)| u.clone()).collect();
        let (id, s2, t2, g2) = (
            screen.clone(),
            synced.clone(),
            target.clone(),
            groups.clone(),
        );
        let written = state
            .storage
            .design_tx(move |c| write_batches(c, &id, synced_seq, &s2, &t2, &g2))
            .await?;
        let Some((rows, seq)) = written else {
            // Someone wrote in between: merge it and try again.
            continue;
        };
        {
            let mut inner = session.lock();
            if rows != target {
                // The write changed rows (equipment instances bound): the document follows.
                let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
                document::apply_rows(&mut txn, &target, &rows);
            }
            inner.synced = rows;
            inner.synced_seq = seq;
            inner.doc_size = wire::state(&inner.doc).len();
            if inner.edits == edits {
                inner.dirty_since = None;
            } else {
                inner.dirty_since = Some(Instant::now());
            }
            session.drain(&mut inner, &state.ws_manager);
            session
                .authors
                .lock()
                .map(|mut a| {
                    for obj in &changed {
                        if authors.get(obj) == a.get(obj) {
                            a.remove(obj);
                        }
                    }
                })
                .ok();
            let msg = ServerMessage::DesignFlushed {
                screen: screen.clone(),
                seq,
                at_ms: crate::timeutil::now_millis(),
                users: users.clone(),
            };
            let members: Vec<String> = inner.members.keys().cloned().collect();
            for c in members {
                send(&mut inner, &state.ws_manager, &c, msg.clone());
            }
        }
        return Ok(Some(Flushed {
            seq,
            users,
            rejected: rejected_all,
        }));
    }
    anyhow::bail!("the screen kept changing while it was written")
}

/// Objects whose rows differ between two row sets.
fn changed_objects(a: &Rows, b: &Rows) -> BTreeSet<(ObjectKind, String)> {
    let mut out = BTreeSet::new();
    let aw: BTreeMap<&str, &WidgetRow> = a.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    let bw: BTreeMap<&str, &WidgetRow> = b.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    for id in aw.keys().chain(bw.keys()) {
        if aw.get(id) != bw.get(id) {
            out.insert((ObjectKind::Widget, id.to_string()));
        }
    }
    let ac: BTreeMap<&str, &ConnectionRow> =
        a.connections.iter().map(|c| (c.id.as_str(), c)).collect();
    let bc: BTreeMap<&str, &ConnectionRow> =
        b.connections.iter().map(|c| (c.id.as_str(), c)).collect();
    for id in ac.keys().chain(bc.keys()) {
        if ac.get(id) != bc.get(id) {
            out.insert((ObjectKind::Connection, id.to_string()));
        }
    }
    out
}

/// Put objects that failed validation back as they were last written (or
/// remove them), as a server change; tell their authors and the audit log.
fn quarantine(
    state: &AppState,
    session: &Arc<Session>,
    synced: &Rows,
    problems: &[validate::Problem],
) -> Vec<DesignRejectedObject> {
    let prev = document::Screen::from_rows(synced);
    let mut inner = session.lock();
    let authors = session
        .authors
        .lock()
        .map(|a| a.clone())
        .unwrap_or_default();
    {
        let now = document::read(&inner.doc.transact());
        let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
        for p in problems {
            match p.kind {
                ObjectKind::Widget => {
                    let target = if p.remove {
                        None
                    } else {
                        prev.widgets.get(&p.id).map(|(w, _)| w)
                    };
                    document::put_widget(&mut txn, &p.id, target, None);
                }
                ObjectKind::Connection => {
                    let target = prev.connections.get(&p.id).filter(|c| {
                        !p.remove
                            && now.widgets.contains_key(&c.from)
                            && now.widgets.contains_key(&c.to)
                    });
                    document::put_connection(&mut txn, &p.id, target);
                }
            }
        }
    }
    session.drain(&mut inner, &state.ws_manager);
    let mut by_user: BTreeMap<String, Vec<DesignRejectedObject>> = BTreeMap::new();
    let mut out = Vec::new();
    for p in problems {
        let obj = DesignRejectedObject {
            object_type: p.kind.as_str().to_string(),
            object_id: p.id.clone(),
            reason: p.reason.clone(),
        };
        let user = authors
            .get(&(p.kind, p.id.clone()))
            .cloned()
            .unwrap_or_else(|| "system".into());
        state.audit_logger.log(crate::security::AuditEvent {
            action: "design_collab_rejected".into(),
            username: user.clone(),
            resource: format!("screen:{}", session.screen),
            details: Some(format!(
                "{} {}: {}",
                obj.object_type, obj.object_id, obj.reason
            )),
        });
        by_user.entry(user).or_default().push(obj.clone());
        out.push(obj);
    }
    session
        .authors
        .lock()
        .map(|mut a| {
            for p in problems {
                a.remove(&(p.kind, p.id.clone()));
            }
        })
        .ok();
    let conns: Vec<(String, String)> = inner
        .members
        .iter()
        .map(|(c, m)| (c.clone(), m.user.clone()))
        .collect();
    for (conn, user) in conns {
        if let Some(objects) = by_user.get(&user) {
            send(
                &mut inner,
                &state.ws_manager,
                &conn,
                ServerMessage::DesignRejected {
                    screen: session.screen.clone(),
                    objects: objects.clone(),
                },
            );
        }
    }
    out
}
