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
//! * Each update is checked as it arrives: its structure before it is
//!   decoded ([`voltrus_collab::guard`]), then the shape of the objects it
//!   touched ([`validate`] without references); what fails is put back at
//!   once, field by field, and a connection that keeps sending bad edits is
//!   sent away. References are checked when the edits are written.

pub mod validate;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use voltrus_collab::document::{self, ConnectionRow, ObjectKind, Rows, WidgetField, WidgetRow};
use voltrus_collab::wire::{self, Handled, Presence};
use voltrus_collab::yrs::types::map::MapEvent;
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
/// A member that sends nothing for this long (clients renew their presence
/// every 15 s) leaves the session.
pub const IDLE_MEMBER: Duration = Duration::from_secs(120);
/// Sessions one connection may be in.
pub const MAX_SESSIONS_PER_CONN: usize = 8;
/// Open sessions of a server.
pub const MAX_SESSIONS: usize = 200;
/// Edits put back by validation one connection may make per
/// [`STRIKE_WINDOW`] before it is sent away.
pub const MAX_STRIKES: usize = 10;
pub const STRIKE_WINDOW: Duration = Duration::from_secs(60);
/// Most awareness updates a connection sends per second (more are dropped).
pub const MAX_AWARENESS_RATE: usize = 30;
/// Stacking indexes longer than this are spread out again at a write.
pub const MAX_ZI_LEN: usize = 32;
/// How long the reference check's view of the tags, instances and
/// faceplates is reused (it is read again at once when it refuses an edit).
const CTX_TTL: Duration = Duration::from_secs(10);
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
        idle_ms: IDLE_MEMBER.as_millis() as u64,
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

/// Numbers memberships: a connection joining again starts a new one.
static JOINS: AtomicU64 = AtomicU64::new(0);

/// One person's connection in a session.
struct Member {
    user: String,
    colour: String,
    /// Latest awareness entry per Yjs client id of this connection.
    presence: BTreeMap<u64, Presence>,
    /// Times of the last sync messages (rate limit).
    recent: VecDeque<Instant>,
    /// Times of the last awareness updates (rate limit).
    recent_awareness: VecDeque<Instant>,
    /// Times of its edits put back by validation.
    strikes: VecDeque<Instant>,
    /// This membership.
    join: u64,
    /// `DesignSync` messages handled since the join.
    received: u64,
    /// The highest `received` a `DesignFlushed` has told it.
    acked: u64,
    /// Its last message of any kind.
    last_seen: Instant,
}

impl Member {
    fn new(user: &str) -> Member {
        Member {
            user: user.to_string(),
            colour: colour_of(user),
            presence: BTreeMap::new(),
            recent: VecDeque::new(),
            recent_awareness: VecDeque::new(),
            strikes: VecDeque::new(),
            join: JOINS.fetch_add(1, Ordering::Relaxed) + 1,
            received: 0,
            acked: 0,
            last_seen: Instant::now(),
        }
    }
}

/// Keep the times within `window` of `now` and add `now`; how many there are.
fn count_in(times: &mut VecDeque<Instant>, now: Instant, window: Duration) -> usize {
    while times
        .front()
        .is_some_and(|t| now.duration_since(*t) > window)
    {
        times.pop_front();
    }
    times.push_back(now);
    times.len()
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
    /// Updates arrived since the size was measured.
    size_dirty: bool,
    /// When the last member left.
    empty_since: Option<Instant>,
    /// [`h::design_writes`] of the screen when the session last compared its head.
    seen_writes: u64,
    /// Connections whose outgoing queue was full: told to join again.
    stale: BTreeSet<String>,
    /// Awareness client id -> the connection that uses it.
    clients: BTreeMap<u64, String>,
    closed: bool,
}

/// Updates (origin, bytes) and touched objects (origin, kind, id, whether
/// its stacking index changed) the document's observers collected during a
/// transaction.
#[derive(Default)]
struct Outbox {
    updates: Vec<(Vec<u8>, Vec<u8>)>,
    touched: Vec<(Vec<u8>, ObjectKind, String, bool)>,
}

/// Who changed an object since it was last written.
#[derive(Clone, Debug, Default, PartialEq)]
struct Author {
    /// The latest.
    last: String,
    /// Everyone.
    all: BTreeSet<String>,
    /// [`Inner::edits`] at the latest change.
    edit: u64,
}

#[derive(Clone, Debug, Default)]
struct Authors {
    objects: BTreeMap<(ObjectKind, String), Author>,
    /// Who last changed a stacking index, and [`Inner::edits`] then: the
    /// `config.z` renumbering a new order causes is theirs.
    restack: Option<(String, u64)>,
}

impl Authors {
    /// Forget what a write of the state as of `edits` covered.
    fn written(&mut self, edits: u64) {
        self.objects.retain(|_, a| a.edit > edits);
        if self.restack.as_ref().is_some_and(|(_, e)| *e <= edits) {
            self.restack = None;
        }
    }
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
    /// Who touched which object since the last write.
    authors: Mutex<Authors>,
}

fn new_epoch() -> String {
    let n: u64 = rand::random();
    format!("{:x}-{n:016x}", crate::timeutil::now_millis())
}

fn origin_bytes(txn: &voltrus_collab::yrs::TransactionMut) -> Vec<u8> {
    txn.origin()
        .map(|o| o.as_ref().to_vec())
        .unwrap_or_default()
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
            if let Ok(mut ob) = ob.lock() {
                ob.updates.push((origin_bytes(txn), e.update.clone()));
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
                let origin = origin_bytes(txn);
                let Ok(mut ob) = ob.lock() else { return };
                for e in events.iter() {
                    let path = e.path();
                    match path.front() {
                        Some(PathSegment::Key(id)) => {
                            let zi = path.len() == 1
                                && matches!(e, Event::Map(m) if map_event_has(m, txn, "zi"));
                            ob.touched.push((origin.clone(), kind, id.to_string(), zi))
                        }
                        Some(PathSegment::Index(_)) => {}
                        None => {
                            if let Event::Map(m) = e {
                                for k in m.keys(txn).keys() {
                                    ob.touched.push((origin.clone(), kind, k.to_string(), true));
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
                size_dirty: false,
                empty_since: Some(Instant::now()),
                seen_writes: h::design_writes(screen),
                stale: BTreeSet::new(),
                clients: BTreeMap::new(),
                closed: false,
            }),
            outbox,
            authors: Mutex::new(Authors::default()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn authors(&self) -> std::sync::MutexGuard<'_, Authors> {
        self.authors.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Pretend a member last sent something `ago` (tests).
    #[doc(hidden)]
    pub fn idle_for_tests(&self, conn: &str, ago: Duration) {
        if let Some(m) = self.lock().members.get_mut(conn) {
            m.last_seen = Instant::now().checked_sub(ago).unwrap_or(m.last_seen);
        }
    }

    /// The encoded size the session counts against [`MAX_DOC`] (tests).
    #[doc(hidden)]
    pub fn doc_size_for_tests(&self) -> usize {
        self.lock().doc_size
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

    /// Objects the updates from `conn` touched that are still in the outbox.
    fn touched_by(&self, conn: &str) -> BTreeSet<(ObjectKind, String)> {
        let origin = origin_of(conn).into_bytes();
        self.outbox
            .lock()
            .map(|o| {
                o.touched
                    .iter()
                    .filter(|t| t.0 == origin)
                    .map(|t| (t.1, t.2.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Take what the observers collected: record who touched what, and send
    /// the updates to the members (not back to the connection they came from).
    fn drain(&self, inner: &mut Inner, ws: &crate::ws_manager::WsManager) {
        let ob = std::mem::take(&mut *self.outbox.lock().unwrap_or_else(|e| e.into_inner()));
        if !ob.touched.is_empty() {
            let mut authors = self.authors();
            for (origin, kind, id, zi) in ob.touched {
                let user = match conn_of_origin(&origin).and_then(|c| inner.members.get(c)) {
                    Some(m) => m.user.clone(),
                    None if origin == SERVER_ORIGIN.as_bytes() => continue,
                    None => "system".to_string(),
                };
                let a = authors.objects.entry((kind, id)).or_default();
                a.last.clone_from(&user);
                a.all.insert(user.clone());
                a.edit = inner.edits;
                if zi && kind == ObjectKind::Widget {
                    authors.restack = Some((user, inner.edits));
                }
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

fn map_event_has(m: &MapEvent, txn: &voltrus_collab::yrs::TransactionMut, key: &str) -> bool {
    m.keys(txn).contains_key(key)
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

/// Members as of a snapshot: (connection, membership, messages handled).
type Marks = Vec<(String, u64, u64)>;

fn marks(inner: &Inner) -> Marks {
    inner
        .members
        .iter()
        .map(|(c, m)| (c.clone(), m.join, m.received))
        .collect()
}

/// Tell every member that the state as of `marks` is written: each gets how
/// many of its own messages that covers (0 for one that joined since).
fn send_flushed(
    inner: &mut Inner,
    ws: &crate::ws_manager::WsManager,
    screen: &str,
    seq: i64,
    users: &[String],
    marks: &Marks,
) {
    let conns: Vec<String> = inner.members.keys().cloned().collect();
    for c in conns {
        let acked = {
            let Some(m) = inner.members.get_mut(&c) else {
                continue;
            };
            let covered = marks
                .iter()
                .find(|(mc, join, _)| *mc == c && *join == m.join)
                .map_or(0, |(_, _, n)| *n);
            m.acked = m.acked.max(covered);
            covered
        };
        send(
            inner,
            ws,
            &c,
            ServerMessage::DesignFlushed {
                screen: screen.to_string(),
                seq,
                at_ms: crate::timeutil::now_millis(),
                users: users.to_vec(),
                acked: Some(acked),
            },
        );
    }
}

/// What [`CollabManager::flush`] did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Flushed {
    /// The change-log head after the write.
    pub seq: i64,
    /// Users whose changes were written.
    pub users: Vec<String>,
    /// Objects (or fields of them) put back by validation.
    pub rejected: Vec<DesignRejectedObject>,
}

/// The open sessions of a server.
#[derive(Default)]
pub struct CollabManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Connection -> screens it is in.
    conns: Mutex<HashMap<String, BTreeSet<String>>>,
    /// Connection -> the [`crate::storage::ACCESS`] generation its access to
    /// sessions was last checked at.
    verified: Mutex<HashMap<String, u64>>,
    /// The reference check's view, and when it was read.
    ctx: Mutex<Option<(Instant, Arc<validate::Ctx>)>>,
    /// Serialises session creation.
    opening: tokio::sync::Mutex<()>,
    /// Shutting down: no more edits are taken.
    closing: AtomicBool,
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
    inner.size_dirty = true;
    session.drain(&mut inner, &state.ws_manager);
    Ok(true)
}

/// Whether two widget rows differ only in `config.z`.
fn only_z_differs(a: &WidgetRow, b: &WidgetRow) -> bool {
    let strip = |r: &WidgetRow| {
        let mut r = r.clone();
        if let Ok(serde_json::Value::Object(mut m)) = serde_json::from_str(&r.config) {
            m.remove("z");
            r.config = serde_json::Value::Object(m).to_string();
        }
        r
    };
    strip(a) == strip(b)
}

/// The author of each changed object: who last changed it since the last
/// write; a widget whose only change is its renumbered `config.z` belongs to
/// whoever changed the stacking order; anything else to `system` (the
/// server's own changes).
fn attribute(
    synced: &Rows,
    target: &Rows,
    changed: &BTreeSet<(ObjectKind, String)>,
    authors: &Authors,
) -> BTreeMap<(ObjectKind, String), String> {
    let prev: BTreeMap<&str, &WidgetRow> =
        synced.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    let next: BTreeMap<&str, &WidgetRow> =
        target.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    changed
        .iter()
        .map(|obj| {
            let user = match authors.objects.get(obj) {
                Some(a) => a.last.clone(),
                None => {
                    let z_only = obj.0 == ObjectKind::Widget
                        && matches!(
                            (prev.get(obj.1.as_str()), next.get(obj.1.as_str())),
                            (Some(a), Some(b)) if only_z_differs(a, b)
                        );
                    match (&authors.restack, z_only) {
                        (Some((u, _)), true) => u.clone(),
                        _ => "system".to_string(),
                    }
                }
            };
            (obj.clone(), user)
        })
        .collect()
}

/// One flush's write, based on `base_seq`: the changed rows only, each
/// object's change logged under its author (one batch per author), in one
/// transaction. `None` when the change log moved (someone wrote outside).
fn write_batches(
    conn: &rusqlite::Connection,
    screen: &str,
    base_seq: i64,
    target: &Rows,
    owners: &BTreeMap<(ObjectKind, String), String>,
) -> anyhow::Result<Option<(Rows, i64)>> {
    if h::head_seq(conn, screen)? != base_seq {
        return Ok(None);
    }
    let (eq, cn) = models_of(screen, target);
    let eq: BTreeMap<&str, &ScadaEquipment> =
        eq.iter().map(|e| (e.equipment_id.as_str(), e)).collect();
    let cn: BTreeMap<&str, &ScadaConnection> =
        cn.iter().map(|c| (c.connection_id.as_str(), c)).collect();
    let mut widgets = Vec::new();
    let mut connections = Vec::new();
    for (kind, id) in owners.keys() {
        match kind {
            ObjectKind::Widget => {
                widgets.push((id.clone(), eq.get(id.as_str()).map(|e| (*e).clone())))
            }
            ObjectKind::Connection => {
                connections.push((id.clone(), cn.get(id.as_str()).map(|c| (*c).clone())))
            }
        }
    }
    let by_key: BTreeMap<(&str, &str), &str> = owners
        .iter()
        .map(|((k, id), u)| ((k.as_str(), id.as_str()), u.as_str()))
        .collect();
    let actor_of = |object_type: &str, object_id: &str| {
        let user = by_key
            .get(&(object_type, object_id))
            .copied()
            .unwrap_or("system");
        DesignActor::new(user.to_string(), "designer", "save").with_note("Live session")
    };
    crate::storage::write_some_rows(conn, screen, widgets, connections, &actor_of)?;
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

    // ── Access ──

    /// The [`crate::storage::ACCESS`] generation `conn`'s access was last
    /// checked at.
    pub fn verified(&self, conn: &str) -> Option<u64> {
        self.verified.lock().ok()?.get(conn).copied()
    }

    /// `conn` may edit as of generation `generation`.
    pub fn set_verified(&self, conn: &str, generation: u64) {
        if let Ok(mut v) = self.verified.lock() {
            v.insert(conn.to_string(), generation);
        }
    }

    /// `conn` may no longer edit: it leaves every session, told `code`.
    pub fn revoke(&self, state: &AppState, conn: &str, code: &str, message: &str) {
        let screens = self
            .conns
            .lock()
            .ok()
            .and_then(|c| c.get(conn).cloned())
            .unwrap_or_default();
        for screen in screens {
            if let Some(s) = self.session(&screen) {
                self.expel(state, &s, conn, code, message, false);
            }
        }
        if let Ok(mut v) = self.verified.lock() {
            v.remove(conn);
        }
    }

    // ── The reference check's view ──

    async fn ctx(
        &self,
        state: &AppState,
        fresh: bool,
    ) -> anyhow::Result<(Arc<validate::Ctx>, bool)> {
        if !fresh
            && let Some((at, ctx)) = self.ctx.lock().ok().and_then(|c| c.clone())
            && at.elapsed() < CTX_TTL
        {
            return Ok((ctx, true));
        }
        let ctx = Arc::new(state.storage.design_tx(validate::Ctx::load).await?);
        if let Ok(mut c) = self.ctx.lock() {
            *c = Some((Instant::now(), ctx.clone()));
        }
        Ok((ctx, false))
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

    async fn open(&self, state: &AppState, screen: &str) -> anyhow::Result<Option<Arc<Session>>> {
        if let Some(s) = self.session(screen) {
            return Ok(Some(s));
        }
        let _opening = self.opening.lock().await;
        if let Some(s) = self.session(screen) {
            return Ok(Some(s));
        }
        if self.sessions.lock().map(|s| s.len()).unwrap_or(0) >= MAX_SESSIONS {
            anyhow::bail!("the server has {MAX_SESSIONS} screens open for editing");
        }
        let id = screen.to_string();
        let found = state
            .storage
            .design_tx(move |c| {
                let exists: bool = c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM devices WHERE id = ?1)",
                    [&id],
                    |r| r.get(0),
                )?;
                if !exists {
                    return Ok(None);
                }
                read_rows(c, &id).map(Some)
            })
            .await?;
        let Some((rows, seq)) = found else {
            return Ok(None);
        };
        let session = Session::new(screen, rows, seq);
        self.sessions
            .lock()
            .map_err(|_| anyhow::anyhow!("collab sessions poisoned"))?
            .insert(screen.to_string(), session.clone());
        self.start_ticker(state);
        tracing::info!("collab: session opened for screen {screen}");
        Ok(Some(session))
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
        if self.closing.load(Ordering::Relaxed) {
            Self::error(
                state,
                conn,
                screen,
                "shutdown",
                "the server is shutting down",
                false,
            );
            return;
        }
        let in_sessions = self
            .conns
            .lock()
            .map(|c| c.get(conn).map(|s| (s.len(), s.contains(screen))))
            .ok()
            .flatten();
        if let Some((n, false)) = in_sessions
            && n >= MAX_SESSIONS_PER_CONN
        {
            Self::error(
                state,
                conn,
                screen,
                "too_many_sessions",
                &format!("one connection edits at most {MAX_SESSIONS_PER_CONN} screens at once"),
                false,
            );
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
            Ok(Some(s)) => s,
            Ok(None) => {
                Self::error(state, conn, screen, "not_found", "no such screen", false);
                return;
            }
            Err(e) => {
                tracing::warn!("collab: screen {screen} not opened: {e:#}");
                Self::error(
                    state,
                    conn,
                    screen,
                    "unavailable",
                    &format!("the screen could not be opened: {e}"),
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
            let member = Member::new(user);
            let colour = member.colour.clone();
            // A connection joining again replaces its old membership.
            let old = inner.members.insert(conn.to_string(), member);
            inner.clients.retain(|_, c| c != conn);
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
            let mut inner = s.lock();
            let stale = inner.stale.contains(conn);
            match inner.members.get_mut(conn) {
                Some(m) if !stale => {
                    m.last_seen = Instant::now();
                    true
                }
                _ => false,
            }
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
        if self.closing.load(Ordering::Relaxed) {
            // Not applied: the client keeps it and sends it again once the
            // server is back (a new session replays what was not written).
            Self::error(
                state,
                conn,
                screen,
                "shutdown",
                "the server is shutting down; your unsaved edits are kept and sent again when it is back",
                false,
            );
            return;
        }
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
        let mut sent_away = false;
        let refusal = {
            let mut inner = session.lock();
            let now = Instant::now();
            let Some(m) = inner.members.get_mut(conn) else {
                return;
            };
            let user = m.user.clone();
            if count_in(&mut m.recent, now, Duration::from_secs(1)) > MAX_RATE as usize {
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
                        if let Some(m) = inner.members.get_mut(conn) {
                            m.received += 1;
                        }
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
                        if let Some(m) = inner.members.get_mut(conn) {
                            m.received += 1;
                        }
                        let changed = !session
                            .outbox
                            .lock()
                            .map(|o| o.updates.is_empty())
                            .unwrap_or(true);
                        let mut put_back = Vec::new();
                        if changed {
                            inner.doc_size += data.len();
                            inner.size_dirty = true;
                            inner.edits += 1;
                            inner.dirty_since.get_or_insert(now);
                            put_back = check_arrived(&session, &mut inner, conn);
                        }
                        session.drain(&mut inner, &state.ws_manager);
                        if !put_back.is_empty() {
                            send(
                                &mut inner,
                                &state.ws_manager,
                                conn,
                                ServerMessage::DesignRejected {
                                    screen: screen.to_string(),
                                    objects: put_back.clone(),
                                },
                            );
                            for o in &put_back {
                                audit_rejected(state, &user, screen, o);
                            }
                            sent_away = strike(&mut inner, conn, put_back.len(), now);
                        }
                        tracing::trace!("collab: update from {user} on {screen}");
                        None
                    }
                    Err(wire::SyncError::Refused(e)) => Some(("refused", e.to_string(), true)),
                    Err(e) => Some(("bad_message", e.to_string(), true)),
                }
            }
        };
        if let Some((code, message, reload)) = refusal {
            self.expel(state, &session, conn, code, &message, reload);
        } else if sent_away {
            self.expel(
                state,
                &session,
                conn,
                "invalid_edits",
                &format!("more than {MAX_STRIKES} edits a minute did not pass validation"),
                true,
            );
        }
    }

    /// `DesignAwareness`: checked, stamped with the member's name and colour,
    /// kept for newcomers and forwarded. A connection sets and clears only
    /// the client ids it uses (the first to use one owns it).
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
        let Inner {
            members, clients, ..
        } = &mut *inner;
        let Some(m) = members.get_mut(conn) else {
            return;
        };
        if count_in(
            &mut m.recent_awareness,
            Instant::now(),
            Duration::from_secs(1),
        ) > MAX_AWARENESS_RATE
        {
            return;
        }
        let mut out = Vec::new();
        for mut e in entries {
            if clients.get(&e.client).is_some_and(|owner| owner != conn) {
                continue;
            }
            let known = m.presence.contains_key(&e.client);
            if !known && m.presence.len() >= MAX_CLIENTS_PER_CONN {
                continue;
            }
            if e.json == "null" {
                if known {
                    m.presence.remove(&e.client);
                    clients.remove(&e.client);
                    out.push(e);
                }
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
            clients.insert(e.client, conn.to_string());
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
            inner.clients.retain(|_, c| c != conn);
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
        if flush_now && !self.closing.load(Ordering::Relaxed) {
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
        if let Ok(mut v) = self.verified.lock() {
            v.remove(conn);
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

    /// Shutting down: take no more edits, then write what the sessions hold.
    /// Edits refused from here stay with their clients, which send them
    /// again to the next session.
    pub async fn shutdown(&self, state: &AppState) {
        self.closing.store(true, Ordering::Relaxed);
        self.flush_all(state).await;
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

    fn close(&self, state: &AppState, s: &Arc<Session>, code: &str, message: &str) {
        let members: Vec<String> = {
            let mut inner = s.lock();
            inner.closed = true;
            inner.clients.clear();
            std::mem::take(&mut inner.members).into_keys().collect()
        };
        for conn in members {
            Self::error(state, &conn, &s.screen, code, message, false);
            self.conns
                .lock()
                .map(|mut c| {
                    if let Some(set) = c.get_mut(&conn) {
                        set.remove(&s.screen);
                        if set.is_empty() {
                            c.remove(&conn);
                        }
                    }
                })
                .ok();
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
            self.close(state, &s, code, message);
        }
        self.conns.lock().map(|mut c| c.clear()).ok();
        self.broadcast_sessions(state);
    }

    /// Close one screen's session without writing it (the screen was
    /// deleted); its members are told `code`.
    pub async fn close_screen(&self, state: &AppState, screen: &str, code: &str, message: &str) {
        let Some(s) = self.session(screen) else {
            return;
        };
        let _lock = s.write.lock().await;
        self.sessions
            .lock()
            .map(|mut m| {
                if m.get(screen).is_some_and(|x| Arc::ptr_eq(x, &s)) {
                    m.remove(screen);
                }
            })
            .ok();
        self.close(state, &s, code, message);
        self.broadcast_sessions(state);
    }

    /// Write a screen's pending edits now, under its write lock.
    pub async fn flush(&self, state: &AppState, screen: &str) -> anyhow::Result<Option<Flushed>> {
        let Some(session) = self.session(screen) else {
            return Ok(None);
        };
        let _lock = session.write.lock().await;
        flush_locked(state, &session).await
    }

    /// The background pass: measure documents, flush, merge outside writes,
    /// acknowledge, retry stale members, drop idle ones, close idle sessions.
    /// A session whose write lock is busy (a long write) is skipped this
    /// time; the others are written concurrently.
    pub async fn tick(&self, state: &AppState) {
        let mut work = Vec::new();
        for s in self.all() {
            let (due, moved, idle, stale, gone) = {
                let mut inner = s.lock();
                if inner.size_dirty {
                    inner.doc_size = wire::state(&inner.doc).len();
                    inner.size_dirty = false;
                }
                let due = inner
                    .dirty_since
                    .is_some_and(|t| t.elapsed() >= FLUSH_AFTER)
                    || (inner.members.is_empty() && inner.dirty_since.is_some());
                let writes = h::design_writes(&s.screen);
                let moved = inner.seen_writes != writes;
                inner.seen_writes = writes;
                let idle = inner.members.is_empty()
                    && inner.empty_since.is_some_and(|t| t.elapsed() >= IDLE_CLOSE);
                let stale: Vec<String> = inner.stale.iter().cloned().collect();
                let gone: Vec<String> = inner
                    .members
                    .iter()
                    .filter(|(_, m)| m.last_seen.elapsed() >= IDLE_MEMBER)
                    .map(|(c, _)| c.clone())
                    .collect();
                // Nothing waits to be written: whatever members sent is in
                // the rows already.
                if inner.dirty_since.is_none()
                    && inner.members.values().any(|m| m.received > m.acked)
                {
                    let (seq, marks) = (inner.synced_seq, marks(&inner));
                    send_flushed(&mut inner, &state.ws_manager, &s.screen, seq, &[], &marks);
                }
                (due, moved, idle, stale, gone)
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
            for conn in gone {
                self.expel(
                    state,
                    &s,
                    &conn,
                    "idle",
                    "nothing heard from this editor for a while; join again",
                    false,
                );
            }
            if due || moved || idle {
                let Ok(lock) = s.write.clone().try_lock_owned() else {
                    continue;
                };
                let s = s.clone();
                work.push(async move {
                    let _lock = lock;
                    if due || moved {
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
                        let still_idle = {
                            let inner = s.lock();
                            inner.members.is_empty() && inner.dirty_since.is_none()
                        };
                        if still_idle {
                            s.lock().closed = true;
                            state
                                .collab
                                .sessions
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
                });
            }
        }
        futures_util::future::join_all(work).await;
    }
}

/// Count `n` edits of `conn` put back; whether it is over [`MAX_STRIKES`].
fn strike(inner: &mut Inner, conn: &str, n: usize, now: Instant) -> bool {
    let Some(m) = inner.members.get_mut(conn) else {
        return false;
    };
    let mut over = false;
    for _ in 0..n.max(1) {
        over |= count_in(&mut m.strikes, now, STRIKE_WINDOW) > MAX_STRIKES;
    }
    over
}

fn audit_rejected(state: &AppState, user: &str, screen: &str, o: &DesignRejectedObject) {
    state.audit_logger.log(crate::security::AuditEvent {
        action: "design_collab_rejected".into(),
        username: user.to_string(),
        resource: format!("screen:{screen}"),
        details: Some(format!("{} {}: {}", o.object_type, o.object_id, o.reason)),
    });
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

// ── Validation ──

/// Check the objects an update from `conn` just touched, as it arrives:
/// remove keys a screen document never holds, and put back (field by field)
/// what has the wrong shape. References are checked at the write.
fn check_arrived(session: &Session, inner: &mut Inner, conn: &str) -> Vec<DesignRejectedObject> {
    let ids = session.touched_by(conn);
    if ids.is_empty() {
        return Vec::new();
    }
    {
        let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
        document::prune(&mut txn, Some(&ids));
    }
    let now = document::read(&inner.doc.transact());
    let mut problems = Vec::new();
    for (kind, id) in &ids {
        let (reason, remove) = match kind {
            ObjectKind::Widget => {
                let Some((w, _)) = now.widgets.get(id) else {
                    continue;
                };
                let prev = inner.synced.widgets.iter().find(|r| &r.id == id);
                match validate::check_widget(&w.to_row(id, None), prev, None) {
                    Ok(()) => continue,
                    Err(r) => (r, prev.is_none()),
                }
            }
            ObjectKind::Connection => {
                let Some(c) = now.connections.get(id) else {
                    continue;
                };
                match validate::check_connection(&c.to_row(id)) {
                    Ok(()) => continue,
                    Err(r) => (r, !inner.synced.connections.iter().any(|r| &r.id == id)),
                }
            }
        };
        problems.push(validate::Problem {
            kind: *kind,
            id: id.clone(),
            reason,
            remove,
        });
    }
    if problems.is_empty() {
        return Vec::new();
    }
    put_back(inner, &problems, None)
        .into_iter()
        .map(|(_, o)| o)
        .collect()
}

/// Put back what failed validation, as a server change: of an object that
/// was written before, only the fields whose change fails (all of them when
/// only their combination fails); a new object, or a connection whose end is
/// gone, is removed.
fn put_back(
    inner: &mut Inner,
    problems: &[validate::Problem],
    ctx: Option<&validate::Ctx>,
) -> Vec<(ObjectKind, DesignRejectedObject)> {
    let synced = inner.synced.clone();
    let prev_w: BTreeMap<&str, &WidgetRow> =
        synced.widgets.iter().map(|w| (w.id.as_str(), w)).collect();
    let prev_c: BTreeMap<&str, &ConnectionRow> = synced
        .connections
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();
    let now = document::read(&inner.doc.transact());
    let mut out = Vec::new();
    let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
    for p in problems {
        let fields: Option<Vec<String>> = match p.kind {
            ObjectKind::Widget => {
                let Some((cur, _)) = now.widgets.get(&p.id) else {
                    continue;
                };
                match prev_w.get(p.id.as_str()).copied() {
                    Some(prev) if !p.remove => {
                        let (pw, _) = document::Widget::from_row(prev);
                        let changes = document::widget_changes(&pw, cur);
                        let mut bad: BTreeSet<String> = changes
                            .iter()
                            .filter(|c| {
                                let mut cand = pw.clone();
                                c.apply_to(&mut cand);
                                validate::check_widget(&cand.to_row(&p.id, None), Some(prev), ctx)
                                    .is_err()
                            })
                            .map(WidgetField::path)
                            .collect();
                        if bad.is_empty() {
                            bad = changes.iter().map(WidgetField::path).collect();
                        }
                        let reverts: Vec<WidgetField> = document::widget_changes(cur, &pw)
                            .into_iter()
                            .filter(|c| bad.contains(&c.path()))
                            .collect();
                        let mut fixed = cur.clone();
                        for r in &reverts {
                            r.apply_to(&mut fixed);
                        }
                        if validate::check_widget(&fixed.to_row(&p.id, None), Some(prev), ctx)
                            .is_ok()
                        {
                            document::write_widget_fields(&mut txn, &p.id, &reverts);
                            Some(bad.into_iter().collect())
                        } else {
                            document::put_widget(&mut txn, &p.id, Some(&pw), None);
                            None
                        }
                    }
                    _ => {
                        document::put_widget(&mut txn, &p.id, None, None);
                        None
                    }
                }
            }
            ObjectKind::Connection => {
                let Some(cur) = now.connections.get(&p.id) else {
                    continue;
                };
                match prev_c.get(p.id.as_str()).copied() {
                    Some(prev) if !p.remove => {
                        let pc = document::Connection::from_row(prev);
                        let changes = document::connection_changes(&pc, cur);
                        let mut bad: BTreeSet<&str> = changes
                            .iter()
                            .filter(|(k, v)| {
                                validate::check_connection(&pc.with_field(k, v).to_row(&p.id))
                                    .is_err()
                            })
                            .map(|(k, _)| *k)
                            .collect();
                        if bad.is_empty() {
                            bad = changes.iter().map(|(k, _)| *k).collect();
                        }
                        let reverts: Vec<(&str, serde_json::Value)> =
                            document::connection_changes(cur, &pc)
                                .into_iter()
                                .filter(|(k, _)| bad.contains(k))
                                .collect();
                        let mut fixed = cur.clone();
                        for (k, v) in &reverts {
                            fixed = fixed.with_field(k, v);
                        }
                        if validate::check_connection(&fixed.to_row(&p.id)).is_ok() {
                            document::write_connection_fields(&mut txn, &p.id, &reverts);
                            Some(bad.into_iter().map(str::to_string).collect())
                        } else {
                            document::put_connection(&mut txn, &p.id, Some(&pc));
                            None
                        }
                    }
                    _ => {
                        document::put_connection(&mut txn, &p.id, None);
                        None
                    }
                }
            }
        };
        let reason = match &fields {
            Some(f) => format!("{} (put back: {})", p.reason, f.join(", ")),
            None => p.reason.clone(),
        };
        out.push((
            p.kind,
            DesignRejectedObject {
                object_type: p.kind.as_str().to_string(),
                object_id: p.id.clone(),
                reason,
            },
        ));
    }
    out
}

/// Put back what failed at a write and tell everyone who changed those
/// objects since the last write; count it against their connections. The
/// connections that are over [`MAX_STRIKES`].
fn quarantine(
    state: &AppState,
    session: &Arc<Session>,
    problems: &[validate::Problem],
    ctx: &validate::Ctx,
) -> (Vec<DesignRejectedObject>, Vec<String>) {
    let mut inner = session.lock();
    let rejected = put_back(&mut inner, problems, Some(ctx));
    session.drain(&mut inner, &state.ws_manager);
    let authors = session.authors().clone();
    let mut by_user: BTreeMap<String, Vec<DesignRejectedObject>> = BTreeMap::new();
    let mut last: BTreeMap<String, usize> = BTreeMap::new();
    for (kind, o) in &rejected {
        let (all, latest) = match authors.objects.get(&(*kind, o.object_id.clone())) {
            Some(a) => (a.all.clone(), a.last.clone()),
            None => ([String::from("system")].into(), "system".to_string()),
        };
        for user in all {
            audit_rejected(state, &user, &session.screen, o);
            by_user.entry(user).or_default().push(o.clone());
        }
        *last.entry(latest).or_default() += 1;
    }
    let conns: Vec<(String, String)> = inner
        .members
        .iter()
        .map(|(c, m)| (c.clone(), m.user.clone()))
        .collect();
    let now = Instant::now();
    let mut over = Vec::new();
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
        if let Some(n) = last.get(&user)
            && strike(&mut inner, &conn, *n, now)
        {
            over.push(conn);
        }
    }
    (rejected.into_iter().map(|(_, o)| o).collect(), over)
}

/// `target` without the objects that failed: those put back keep their
/// written rows until the next write checks them again; those removed (new
/// objects, connections whose end is gone) are written removed.
fn hold_back(target: &Rows, synced: &Rows, problems: &[validate::Problem]) -> Rows {
    let mut out = target.clone();
    for p in problems {
        match p.kind {
            ObjectKind::Widget => {
                out.widgets.retain(|w| w.id != p.id);
                if !p.remove
                    && let Some(w) = synced.widgets.iter().find(|w| w.id == p.id)
                {
                    out.widgets.push(w.clone());
                }
            }
            ObjectKind::Connection => {
                out.connections.retain(|c| c.id != p.id);
                if !p.remove
                    && let Some(c) = synced.connections.iter().find(|c| c.id == p.id)
                {
                    out.connections.push(c.clone());
                }
            }
        }
    }
    sort_rows(&mut out);
    out
}

// ── Writing ──

/// Write the session's document to the draft rows (the caller holds the
/// write lock): merge outside writes first, tidy the document (stacking
/// indexes, unknown keys), validate the changed objects (putting back what
/// fails and leaving it for the next write), write the changed rows with each
/// object's change under its author, and apply what the write changed
/// (equipment binding) back to the document.
async fn flush_locked(state: &AppState, session: &Arc<Session>) -> anyhow::Result<Option<Flushed>> {
    let screen = session.screen.clone();
    let ws = &state.ws_manager;
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
        let (target, synced, synced_seq, edits, marks) = {
            let mut inner = session.lock();
            if inner.dirty_since.is_none() {
                return Ok(None);
            }
            {
                let mut txn = inner.doc.transact_mut_with(SERVER_ORIGIN);
                document::rebalance_zi(&mut txn, MAX_ZI_LEN);
                document::prune(&mut txn, None);
            }
            session.drain(&mut inner, ws);
            let doc_screen = document::read(&inner.doc.transact());
            let mut target = doc_screen.to_rows(&inner.synced);
            sort_rows(&mut target);
            (
                target,
                inner.synced.clone(),
                inner.synced_seq,
                inner.edits,
                marks(&inner),
            )
        };
        if target == synced {
            let mut inner = session.lock();
            if inner.edits == edits {
                inner.dirty_since = None;
            }
            session.authors().written(edits);
            let seq = inner.synced_seq;
            send_flushed(&mut inner, ws, &screen, seq, &[], &marks);
            return Ok(None);
        }
        // References against what exists (a cached view; read again before
        // refusing anything).
        let (mut ctx, cached) = state.collab.ctx(state, false).await?;
        let mut problems = validate::check(&synced, &target, Some(&ctx));
        if !problems.is_empty() && cached {
            ctx = state.collab.ctx(state, true).await?.0;
            problems = validate::check(&synced, &target, Some(&ctx));
        }
        let held_back = !problems.is_empty();
        let target = if held_back {
            let (rejected, over) = quarantine(state, session, &problems, &ctx);
            rejected_all.extend(rejected);
            for conn in over {
                state.collab.expel(
                    state,
                    session,
                    &conn,
                    "invalid_edits",
                    &format!("more than {MAX_STRIKES} edits a minute did not pass validation"),
                    true,
                );
            }
            hold_back(&target, &synced, &problems)
        } else {
            target
        };
        let changed = changed_objects(&synced, &target);
        if changed.is_empty() {
            // Everything changed failed: the put-back state is written next time.
            session.lock().dirty_since = Some(Instant::now());
            return Ok(Some(Flushed {
                seq: synced_seq,
                users: Vec::new(),
                rejected: rejected_all,
            }));
        }
        let owners = attribute(&synced, &target, &changed, &session.authors());
        let users: Vec<String> = owners
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let (id, t2, o2) = (screen.clone(), target.clone(), owners.clone());
        let written = state
            .storage
            .design_tx(move |c| write_batches(c, &id, synced_seq, &t2, &o2))
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
            inner.size_dirty = false;
            if inner.edits == edits && !held_back {
                inner.dirty_since = None;
            } else {
                inner.dirty_since = Some(Instant::now());
            }
            session.drain(&mut inner, ws);
            session
                .authors()
                .objects
                .retain(|k, a| a.edit > edits || !changed.contains(k));
            if !held_back {
                session.authors().written(edits);
            }
            // With objects held back, the members' messages are not all in
            // the rows yet: no acknowledgement this time.
            let none = Vec::new();
            send_flushed(
                &mut inner,
                ws,
                &screen,
                seq,
                &users,
                if held_back { &none } else { &marks },
            );
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
