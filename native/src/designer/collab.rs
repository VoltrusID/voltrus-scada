//! Live collaboration in the native Designer (docs/collaboration.md): the
//! console joins the same session as the web Designer, with the same code
//! the server uses for the screen document (`voltrus-collab`, `yrs`).
//!
//! A [`Session`] holds the console's copy of the document. No UI here: the
//! editor feeds it the server's `Design*` frames ([`Session::on_server`]) and
//! its layout after every change ([`Session::local`]), and sends what
//! [`Session::outgoing`] gives on the WebSocket.
//!
//! - **Edits.** A local edit is the difference between the layout the
//!   session last saw and the editor's, applied to the document field by
//!   field ([`document::apply_rows`]) in a transaction with the local
//!   origin; remote updates come back as a new [`Layout`]. Updates go out
//!   batched every [`BATCH`].
//! - **Undo** is `yrs`'s `UndoManager` scoped to the local origin, so it
//!   takes back this console's edits only; a new editor step starts a new
//!   undo item ([`Session::local`] with `new_step`), steps of one key within
//!   [`edit::COALESCE`] are one item.
//! - **Joining** with edits not yet in the session (unsaved work when the
//!   Designer opened, edits made while waiting for the server's document):
//!   once the server's document is in, what the editor changed since the
//!   session last saw it is written into it as one edit.
//! - **Unflushed edits.** Every `DesignSync` sent since the latest
//!   `DesignJoined` is numbered; the objects and fields each local or undo
//!   transaction changed ([`document::touched_between`]) ride with the
//!   message that carried them until a `DesignFlushed` whose `acked` covers
//!   it. A same-epoch rejoin carries what is unacked on the step 2 sent
//!   after it; a new epoch (the server restarted) or `resync` starts from the
//!   server's document and writes them again ([`document::replay`]).
//! - **Membership lost** (`DesignError` `resync`, `not_joined`,
//!   `rate_limited`, `idle`, reload false): join again keeping the document
//!   and epoch, after a back-off that grows while it keeps happening.
//! - **Pacing.** Every `DesignSync` sent counts toward a rolling second kept
//!   under `max_rate - 10`; a merged update stays under `max_update - 64`
//!   (fewer queued updates per frame); presence goes out at least every
//!   third of `idle_ms` as a keepalive.
//! - **Older servers**, or collaboration turned off: no `DesignJoined`
//!   within [`JOIN_TIMEOUT`], or `DesignError disabled`/`forbidden`, and the
//!   session is [`Phase::Off`]: the editor saves whole layouts as before.
//!
//! [`edit::COALESCE`]: crate::mimic::edit::COALESCE

use std::collections::{BTreeMap, VecDeque};
use std::mem;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use voltrus_collab::document::{
    self, ConnectionRow as CRow, ObjectKind, Rows, Screen, Touched, WidgetRow,
};
use voltrus_collab::wire::{self, Handled, Presence};
use voltrus_collab::yrs::undo::{Options as UndoOptions, UndoManager};
use voltrus_collab::yrs::{Doc, Origin, Transact};

use crate::mimic::edit::COALESCE;
use crate::mimic::{self, ConnectionRow, EquipmentRow, Layout};
use crate::ws_types::{ClientMessage, DesignLimits, ServerMessage};

/// The origin of this console's own transactions.
pub const LOCAL: &str = "local";
/// The origin of updates from the server.
const REMOTE: &str = "remote";
/// Local updates are merged and sent at most this often.
pub const BATCH: Duration = Duration::from_millis(40);
/// Presence (cursor, selection) is sent at most this often.
pub const PRESENCE_EVERY: Duration = Duration::from_millis(50);
/// A server that has not answered `DesignJoin` by then has no sessions.
pub const JOIN_TIMEOUT: Duration = Duration::from_secs(5);
/// `DesignLimits::idle_ms` when the server sends none (an older server).
pub const IDLE: Duration = Duration::from_secs(120);
/// Room left under the server's `max_rate` (sync messages per second).
const RATE_MARGIN: u32 = 10;
/// Room left under the server's `max_update` for the message's framing.
const UPDATE_MARGIN: usize = 64;
/// The longest wait before joining again after the server dropped this
/// console.
const REJOIN_MAX: Duration = Duration::from_secs(32);
/// Without a dropped membership for this long, the back-off starts over.
const REJOIN_CALM: Duration = Duration::from_secs(30);

/// Where the session stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// `DesignJoin` sent (or due), no answer yet. Once the server's
    /// document is in, edits continue into it and go out after joining.
    Joining { since: Instant },
    /// In the session.
    Live,
    /// The link is down: edits go into the document and out once back.
    Offline,
    /// No collaboration (an older server, turned off, not allowed): the
    /// editor saves whole layouts.
    Off(String),
}

/// The visible part of the screen: its centre on the stage and the zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

/// Another person in the session.
#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    pub client: u64,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    pub cursor: Option<(f64, f64)>,
    pub selection: Vec<String>,
    pub viewport: Option<Viewport>,
}

/// This console's presence.
#[derive(Debug, Clone, Default, PartialEq)]
struct Local {
    cursor: Option<(f64, f64)>,
    selection: Vec<String>,
    viewport: Option<Viewport>,
}

/// What a server frame asks of the editor.
#[derive(Debug, Default)]
pub struct Reaction {
    /// Frames to send.
    pub send: Vec<ClientMessage>,
    /// The document changed: the editor shows this layout.
    pub layout: Option<Layout>,
    /// Something to tell the user (text, whether it is an error).
    pub toast: Option<(String, bool)>,
    /// The session is off: save whole layouts from now on.
    pub fallback: bool,
}

/// The console's copy of one screen's live session.
pub struct Session {
    pub screen: String,
    pub phase: Phase,
    doc: Doc,
    undo: UndoManager<()>,
    /// Updates of local and undo transactions, not sent yet, each with the
    /// objects and fields it changed.
    outbox: Outbox,
    /// What edits whose updates left the outbox unsent changed (offline, a
    /// rejoin): the next step 2 this console sends carries them.
    carry: Touched,
    /// `DesignSync` messages sent since the latest `DesignJoined`.
    numbered: u64,
    /// Sent and not covered by a `DesignFlushed` yet: the message's number
    /// and what it changed.
    inflight: VecDeque<(u64, Touched)>,
    /// The server rebuilt the document: what to write again once its copy
    /// is in, the console's copy to take the values from, and whether the
    /// server restarted (the user is told).
    replay: Option<(Touched, Screen, bool)>,
    /// A join due then (after a dropped membership).
    rejoin_at: Option<Instant>,
    /// Memberships dropped in a row, and when the last was.
    strikes: u32,
    last_strike: Option<Instant>,
    /// Something to tell the user outside a server frame.
    notice: Option<(String, bool)>,
    /// The latest time the editor gave.
    now: Instant,
    epoch: Option<String>,
    /// The signed-in user and colour the server gave.
    pub user: String,
    pub color: String,
    pub limits: Option<DesignLimits>,
    /// The rows the current layout was made from (or written as).
    rows: Rows,
    layout: Layout,
    /// The server's document is in (its step 2 arrived).
    synced: bool,
    last_send: Option<Instant>,
    sent: VecDeque<Instant>,
    clock: u32,
    local: Local,
    presence_dirty: bool,
    last_presence: Option<Instant>,
    pub peers: BTreeMap<u64, Peer>,
    /// The last write to the draft: change-log head and whose edits.
    pub flushed: Option<(i64, Vec<String>)>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("screen", &self.screen)
            .field("phase", &self.phase)
            .field("epoch", &self.epoch)
            .field("peers", &self.peers.len())
            .finish_non_exhaustive()
    }
}

// ── Rows ──

/// The shared crate's rows of the console's rows.
pub fn collab_rows(equipment: &[EquipmentRow], connections: &[ConnectionRow]) -> Rows {
    Rows {
        widgets: equipment
            .iter()
            .map(|r| WidgetRow {
                id: r.equipment_id.clone(),
                kind: r.equipment_type.clone(),
                name: r.equipment_name.clone(),
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
                rotation: r.rotation,
                sensors: r.sensor_bindings.clone(),
                config: r.config.clone(),
            })
            .collect(),
        connections: connections
            .iter()
            .map(|r| CRow {
                id: r.connection_id.clone(),
                from: r.from_equipment.clone(),
                to: r.to_equipment.clone(),
                kind: r.connection_type.clone(),
                waypoints: r.waypoints.clone(),
            })
            .collect(),
    }
}

/// The layout of the shared crate's rows.
pub fn layout_of(rows: &Rows) -> Layout {
    let equipment: Vec<EquipmentRow> = rows
        .widgets
        .iter()
        .map(|r| EquipmentRow {
            screen_id: String::new(),
            equipment_id: r.id.clone(),
            equipment_type: r.kind.clone(),
            equipment_name: r.name.clone(),
            x: r.x,
            y: r.y,
            width: r.width,
            height: r.height,
            rotation: r.rotation,
            sensor_bindings: r.sensors.clone(),
            config: r.config.clone(),
        })
        .collect();
    let connections: Vec<ConnectionRow> = rows
        .connections
        .iter()
        .map(|r| ConnectionRow {
            screen_id: String::new(),
            connection_id: r.id.clone(),
            from_equipment: r.from.clone(),
            to_equipment: r.to.clone(),
            waypoints: r.waypoints.clone(),
            connection_type: r.kind.clone(),
        })
        .collect();
    mimic::parse(&equipment, &connections)
}

/// The rows of `next`: widgets that differ from `prev` get the name the
/// web's save gives them (label, stored name, tag name, title); the others
/// keep theirs, so an edit never renames what it did not touch.
pub fn rows_of(prev: &Layout, next: &Layout, tag_name: &dyn Fn(i64) -> Option<String>) -> Rows {
    let mut named = next.clone();
    for widget in &mut named.widgets {
        if prev.widget(&widget.id) != Some(widget) {
            widget.name = super::saved_name(widget, tag_name);
        }
    }
    let (equipment, connections) = mimic::to_rows(&named);
    collab_rows(&equipment, &connections)
}

/// A presence colour as `0xrrggbb`.
pub fn color_rgb(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6)
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

/// Up to two initials of a name (`ana.lee` → `AL`).
pub fn initials(name: &str) -> String {
    let parts: Vec<&str> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|p| !p.is_empty())
        .collect();
    let letters: String = match parts.as_slice() {
        [] => "?".into(),
        [one] => one.chars().take(2).collect(),
        [first, .., last] => first.chars().take(1).chain(last.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

fn peer_of(p: &Presence) -> Option<Peer> {
    let v: Value = serde_json::from_str(&p.json).ok()?;
    let user = v.get("user")?;
    let point = |v: &Value| Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?));
    Some(Peer {
        client: p.client,
        name: user.get("name")?.as_str()?.to_string(),
        color: user
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("#888888")
            .to_string(),
        cursor: v.get("cursor").and_then(point),
        selection: v
            .get("selection")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        viewport: v.get("viewport").and_then(|vp| {
            let (x, y) = point(vp)?;
            Some(Viewport {
                x,
                y,
                scale: vp.get("scale")?.as_f64()?,
            })
        }),
    })
}

/// Updates waiting to go out, each with what it changed.
type Outbox = Arc<Mutex<Vec<(Vec<u8>, Touched)>>>;

/// The touched sets of the outbox, which is emptied.
fn drain(outbox: &Outbox) -> Touched {
    let mut t = Touched::default();
    if let Ok(mut out) = outbox.lock() {
        for (_, touched) in out.drain(..) {
            t.merge(&touched);
        }
    }
    t
}

/// A size as people read it.
fn size_text(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    }
}

/// Remove `object` from `t`.
fn forget(t: &mut Touched, object: &(ObjectKind, String)) {
    t.0.remove(object);
}

fn fresh_doc() -> (Doc, UndoManager<()>, Outbox) {
    let doc = Doc::new();
    let widgets = doc.get_or_insert_map(document::WIDGETS);
    let connections = doc.get_or_insert_map(document::CONNECTIONS);
    let outbox: Outbox = Arc::default();
    let sink = outbox.clone();
    let remote = Origin::from(REMOTE);
    let _ = doc.observe_update_v1("outbox", move |txn, e| {
        if txn.origin() != Some(&remote)
            && let Ok(mut out) = sink.lock()
        {
            out.push((e.update.clone(), Touched::default()));
        }
    });
    let mut undo = UndoManager::with_options(UndoOptions {
        capture_timeout_millis: u64::try_from(COALESCE.as_millis()).unwrap_or(1000),
        ..UndoOptions::default()
    });
    undo.expand_scope(&doc, &widgets);
    undo.expand_scope(&doc, &connections);
    undo.include_origin(Origin::from(LOCAL));
    (doc, undo, outbox)
}

impl Session {
    /// Open a session for `screen`, whose editor shows `layout`; the
    /// returned frame asks to join. `epoch`: the session this console was
    /// in before (a reconnect).
    pub fn join(screen: &str, layout: &Layout, now: Instant) -> (Self, ClientMessage) {
        let (doc, undo, outbox) = fresh_doc();
        let (equipment, connections) = mimic::to_rows(layout);
        let rows = collab_rows(&equipment, &connections);
        let session = Self {
            screen: screen.to_string(),
            phase: Phase::Joining { since: now },
            doc,
            undo,
            outbox,
            epoch: None,
            user: String::new(),
            color: String::new(),
            limits: None,
            carry: Touched::default(),
            numbered: 0,
            inflight: VecDeque::new(),
            replay: None,
            rejoin_at: None,
            strikes: 0,
            last_strike: None,
            notice: None,
            now,
            rows,
            layout: layout.clone(),
            synced: false,
            last_send: None,
            sent: VecDeque::new(),
            clock: 0,
            local: Local::default(),
            presence_dirty: true,
            last_presence: None,
            peers: BTreeMap::new(),
            flushed: None,
        };
        let join = session.join_frame();
        (session, join)
    }

    fn join_frame(&self) -> ClientMessage {
        ClientMessage::DesignJoin {
            screen: self.screen.clone(),
            protocol: voltrus_collab::PROTOCOL,
            epoch: self.epoch.clone(),
        }
    }

    /// Whether edits go through the session (not whole-layout saves).
    pub fn active(&self) -> bool {
        self.synced && !self.off()
    }

    /// Whether the session gave up (the editor saves whole layouts).
    pub fn off(&self) -> bool {
        matches!(self.phase, Phase::Off(_))
    }

    /// The layout the session last saw.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Local edits not sent yet.
    pub fn unsent(&self) -> bool {
        self.outbox.lock().is_ok_and(|o| !o.is_empty())
            || !self.carry.is_empty()
            || self.replay.is_some()
    }

    /// Edits of this console the server has not written to the draft yet:
    /// queued, sent but not acknowledged by a `DesignFlushed`, made offline
    /// or waiting to be written again into a rebuilt document. Leaving the
    /// Designer with any asks first.
    pub fn unflushed(&self) -> bool {
        !self.off() && (self.unsent() || !self.inflight.is_empty())
    }

    /// Something to tell the user that came of [`Self::outgoing`].
    pub fn take_notice(&mut self) -> Option<(String, bool)> {
        self.notice.take()
    }

    /// The server's idle limit: a member that sends nothing that long is
    /// removed.
    fn idle(&self) -> Duration {
        match self.limits.as_ref().map(|l| l.idle_ms) {
            Some(ms) if ms > 0 => Duration::from_millis(ms),
            _ => IDLE,
        }
    }

    /// `DesignSync` messages this console sends in any second.
    fn budget(&self) -> usize {
        self.limits.as_ref().map_or(20, |l| {
            l.max_rate.saturating_sub(RATE_MARGIN).max(1) as usize
        })
    }

    /// A `DesignSync` frame goes out: numbered, counted toward the rate,
    /// and what it carries kept until a flush acknowledges it.
    fn sync_frame(&mut self, data: Vec<u8>, touched: Touched) -> ClientMessage {
        self.numbered += 1;
        self.sent.push_back(self.now);
        if !touched.is_empty() {
            self.inflight.push_back((self.numbered, touched));
        }
        ClientMessage::DesignSync {
            screen: self.screen.clone(),
            data,
        }
    }

    /// Everything not written to the draft yet, as one set (the outbox is
    /// emptied).
    fn unacked(&mut self) -> Touched {
        let mut t = mem::take(&mut self.carry);
        for (_, touched) in self.inflight.drain(..) {
            t.merge(&touched);
        }
        t.merge(&drain(&self.outbox));
        t
    }

    /// Run `f` (a local or undo transaction) and keep what it changed with
    /// the update it queued.
    fn tracked<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let before = document::read(&self.doc.transact());
        let queued = self.outbox.lock().map_or(0, |o| o.len());
        let r = f(self);
        let after = document::read(&self.doc.transact());
        let touched = document::touched_between(&before, &after);
        if !touched.is_empty() {
            let mut kept = false;
            if let Ok(mut out) = self.outbox.lock()
                && let Some((_, t)) = out.get_mut(queued..).and_then(<[_]>::first_mut)
            {
                t.merge(&touched);
                kept = true;
            }
            if !kept {
                self.carry.merge(&touched);
            }
        }
        r
    }

    /// The server dropped this console's membership (or its last message):
    /// join again, keeping the document and epoch, after a back-off that
    /// grows while it keeps happening. The join frame when due now.
    fn rejoin(&mut self, rate_limited: bool) -> Option<ClientMessage> {
        let now = self.now;
        if self
            .last_strike
            .is_some_and(|t| now.saturating_duration_since(t) > REJOIN_CALM)
        {
            self.strikes = 0;
        }
        self.strikes += 1;
        self.last_strike = Some(now);
        let mut delay = if rate_limited {
            Duration::from_secs(1)
        } else {
            Duration::ZERO
        };
        if self.strikes > 1 {
            let backoff = Duration::from_secs(1 << (self.strikes - 2).min(5));
            delay = delay.max(backoff).min(REJOIN_MAX);
        }
        self.phase = Phase::Joining { since: now };
        self.peers.clear();
        if delay.is_zero() {
            self.rejoin_at = None;
            Some(self.join_frame())
        } else {
            self.rejoin_at = Some(now + delay);
            None
        }
    }

    /// Start again from the server's copy, writing `touched` (values from
    /// `mine`) into it once it is in. `restarted`: the server's session
    /// started over (the user is told how many changes were written again).
    fn rebuild(&mut self, touched: Touched, mine: Screen, restarted: bool) {
        self.reset_doc();
        self.replay = (!touched.is_empty()).then_some((touched, mine, restarted));
    }

    pub fn can_undo(&self) -> bool {
        self.active() && self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.active() && self.undo.can_redo()
    }

    /// The link dropped: edits continue into the document; what is not
    /// flushed yet is written again if the server lost the session.
    pub fn offline(&mut self) {
        if self.off() {
            return;
        }
        self.rejoin_at = None;
        if self.synced {
            self.phase = Phase::Offline;
        }
        self.peers.clear();
    }

    /// The link is back: join again (same epoch: the y-sync exchange
    /// merges what both sides did meanwhile).
    pub fn reconnect(&mut self, now: Instant) -> Option<ClientMessage> {
        if self.off() {
            return None;
        }
        self.now = self.now.max(now);
        if !self.synced {
            self.phase = Phase::Joining { since: now };
        }
        self.rejoin_at = None;
        // The step 2 of the exchange carries them.
        let queued = drain(&self.outbox);
        self.carry.merge(&queued);
        self.presence_dirty = true;
        Some(self.join_frame())
    }

    /// Joining took too long: an older server. Whether the session just
    /// turned off.
    pub fn expire(&mut self, now: Instant) -> bool {
        if let Phase::Joining { since } = self.phase
            && !self.synced
            && self.rejoin_at.is_none()
            && now.saturating_duration_since(since) > JOIN_TIMEOUT
        {
            self.phase = Phase::Off("The server has no live sessions".into());
            return true;
        }
        false
    }

    /// Start over with an empty document (the server's arrives next).
    fn reset_doc(&mut self) {
        let (doc, undo, outbox) = fresh_doc();
        self.doc = doc;
        self.undo = undo;
        self.outbox = outbox;
        self.synced = false;
    }

    /// The document's layout, made the session's.
    fn refresh(&mut self) -> Layout {
        let screen = document::read(&self.doc.transact());
        self.rows = screen.to_rows(&self.rows);
        self.layout = layout_of(&self.rows);
        self.layout.clone()
    }

    /// A frame from the server. `mine` is the editor's layout now (its
    /// edits are written into a newly joined document).
    pub fn on_server(
        &mut self,
        message: &ServerMessage,
        mine: &Layout,
        tag_name: &dyn Fn(i64) -> Option<String>,
    ) -> Reaction {
        let mut r = Reaction::default();
        match message {
            ServerMessage::DesignJoined {
                screen,
                epoch,
                resync,
                user,
                color,
                limits,
                ..
            } if *screen == self.screen => {
                let same = self.epoch.as_deref() == Some(epoch.as_str()) && !resync;
                // Numbering starts over; what was sent before and not
                // acknowledged rides on the step 2 sent after this join.
                let unacked = self.unacked();
                self.numbered = 0;
                if !same && self.synced {
                    // The server rebuilt the document: start from its copy
                    // and write the console's unflushed edits again.
                    let mine = document::read(&self.doc.transact());
                    self.rebuild(unacked, mine, true);
                } else if let Some((t, _, _)) = self.replay.as_mut() {
                    // Still waiting for the copy to replay into.
                    t.merge(&unacked);
                } else {
                    self.carry = unacked;
                }
                self.epoch = Some(epoch.clone());
                self.user = user.clone();
                self.color = color.clone();
                self.limits = Some(limits.clone());
                self.phase = Phase::Live;
                self.rejoin_at = None;
                self.presence_dirty = true;
                let step1 = wire::step1(&self.doc);
                r.send.push(self.sync_frame(step1, Touched::default()));
            }
            ServerMessage::DesignSync { screen, data } if *screen == self.screen => {
                let Ok(msg) = wire::decode(data) else {
                    return r;
                };
                let step2 = matches!(msg, wire::Sync::SyncStep2(_));
                match wire::handle(&self.doc, REMOTE, msg) {
                    Ok(Handled::Reply(reply)) => {
                        // The step 2 after a join: it holds every edit the
                        // server may lack.
                        let carried = mem::take(&mut self.carry);
                        r.send.push(self.sync_frame(reply, carried));
                    }
                    Ok(Handled::Applied) => {
                        if step2 && !self.synced {
                            self.synced = true;
                            if matches!(self.phase, Phase::Offline) {
                                self.phase = Phase::Live;
                            }
                            r.toast = self.first_sync(mine, tag_name);
                        }
                        if self.synced {
                            r.layout = Some(self.refresh());
                        }
                    }
                    Err(e) => {
                        r.toast = Some((format!("Live session: {e}"), true));
                    }
                }
            }
            ServerMessage::DesignAwareness { screen, data } if *screen == self.screen => {
                let own = self.doc.client_id().get();
                for p in wire::decode_awareness(data).unwrap_or_default() {
                    if p.client == own {
                        continue;
                    }
                    match peer_of(&p) {
                        Some(peer) => {
                            self.peers.insert(p.client, peer);
                        }
                        None => {
                            self.peers.remove(&p.client);
                        }
                    }
                }
            }
            ServerMessage::DesignError {
                screen,
                code,
                message,
                reload,
            } if *screen == self.screen => match code.as_str() {
                "disabled" | "forbidden" | "protocol" => {
                    self.phase = Phase::Off(message.clone());
                    r.fallback = true;
                    if code == "protocol" {
                        r.toast = Some((
                            format!(
                                "Live editing needs a console that matches the server: {message}"
                            ),
                            true,
                        ));
                    }
                }
                _ if *reload => {
                    // The server could not take the document as it is:
                    // start again from its copy.
                    self.epoch = None;
                    self.reset_doc();
                    self.replay = None;
                    self.carry = Touched::default();
                    self.inflight.clear();
                    self.rejoin_at = None;
                    self.phase = Phase::Joining { since: self.now };
                    r.send.push(self.join_frame());
                    r.toast = Some((format!("Live session reloaded: {message}"), true));
                }
                "resync" | "not_joined" | "rate_limited" | "idle" => {
                    let rate_limited = code == "rate_limited";
                    if let Some(join) = self.rejoin(rate_limited) {
                        r.send.push(join);
                    }
                    if rate_limited {
                        r.toast = Some((
                            "Live session: too many edits at once; rejoining".into(),
                            false,
                        ));
                    }
                }
                _ => r.toast = Some((message.clone(), true)),
            },
            ServerMessage::DesignRejected { screen, objects } if *screen == self.screen => {
                // Put back by the server: not written again later.
                for o in objects {
                    let kind = match o.object_type.as_str() {
                        "connection" => ObjectKind::Connection,
                        _ => ObjectKind::Widget,
                    };
                    let object = (kind, o.object_id.clone());
                    forget(&mut self.carry, &object);
                    for (_, t) in &mut self.inflight {
                        forget(t, &object);
                    }
                    if let Ok(mut out) = self.outbox.lock() {
                        for (_, t) in out.iter_mut() {
                            forget(t, &object);
                        }
                    }
                }
                let list: Vec<String> = objects
                    .iter()
                    .map(|o| format!("{} ({})", o.object_id, o.reason))
                    .collect();
                r.toast = Some((format!("Put back by the server: {}", list.join("; ")), true));
            }
            ServerMessage::DesignFlushed {
                screen,
                seq,
                users,
                acked,
                ..
            } if *screen == self.screen => {
                self.flushed = Some((*seq, users.clone()));
                match acked {
                    Some(n) => self.inflight.retain(|(k, _)| k > n),
                    // An older server: what was sent before it counts as
                    // written.
                    None => self.inflight.clear(),
                }
            }
            _ => {}
        }
        r
    }

    /// The server's document just came in: write into it the unflushed
    /// edits of a rebuilt session, then what the editor changed since the
    /// session last saw it (`mine`). What to tell the user.
    fn first_sync(
        &mut self,
        mine: &Layout,
        tag_name: &dyn Fn(i64) -> Option<String>,
    ) -> Option<(String, bool)> {
        let base_rows = self.rows.clone();
        let base_layout = self.layout.clone();
        self.refresh();
        let mut toast = None;
        let mut wrote = false;
        if let Some((touched, theirs, restarted)) = self.replay.take() {
            self.tracked(|s| {
                let mut txn = s.doc.transact_mut_with(Origin::from(LOCAL));
                document::replay(&mut txn, &touched, &theirs);
            });
            self.refresh();
            wrote = true;
            let n = touched.len();
            toast = restarted.then(|| {
                (
                    format!(
                        "The live session restarted; {n} unsaved change{} applied again",
                        if n == 1 { " was" } else { "s were" }
                    ),
                    false,
                )
            });
        }
        if *mine != base_layout {
            let mine_rows = rows_of(&base_layout, mine, tag_name);
            self.apply_local(&base_rows, &mine_rows);
            wrote = true;
        }
        if wrote {
            // Joining is not an undo step of the user's.
            self.undo.clear_all();
        }
        toast
    }

    /// Apply `before` → `after` to the document as a local edit.
    fn apply_local(&mut self, before: &Rows, after: &Rows) {
        self.tracked(|s| {
            let mut txn = s.doc.transact_mut_with(Origin::from(LOCAL));
            document::apply_rows(&mut txn, before, after);
        });
        self.refresh();
    }

    /// The editor's layout after its edits. `new_step`: the edits began an
    /// undo step (else they join the running one). Whether the document
    /// changed.
    pub fn local(
        &mut self,
        layout: &Layout,
        new_step: bool,
        tag_name: &dyn Fn(i64) -> Option<String>,
    ) -> bool {
        if !self.active() || *layout == self.layout {
            return false;
        }
        let rows = rows_of(&self.layout, layout, tag_name);
        if new_step {
            self.undo.reset();
        }
        let before = self.rows.clone();
        self.apply_local(&before, &rows);
        true
    }

    /// Undo this console's last edit; the layout after it.
    pub fn undo(&mut self) -> Option<Layout> {
        (self.active() && self.tracked(|s| s.undo.undo_blocking())).then(|| self.refresh())
    }

    pub fn redo(&mut self) -> Option<Layout> {
        (self.active() && self.tracked(|s| s.undo.redo_blocking())).then(|| self.refresh())
    }

    /// Where the pointer is, what is selected and what is in view.
    pub fn set_presence(
        &mut self,
        cursor: Option<(f64, f64)>,
        selection: &[String],
        viewport: Option<Viewport>,
    ) {
        let next = Local {
            cursor: cursor.map(|(x, y)| (x.round(), y.round())),
            selection: selection.to_vec(),
            viewport,
        };
        if next != self.local {
            self.local = next;
            self.presence_dirty = true;
        }
    }

    fn presence_json(&self) -> String {
        let point = |p: Option<(f64, f64)>| p.map_or(Value::Null, |(x, y)| json!({"x": x, "y": y}));
        json!({
            "user": {"name": self.user, "color": self.color},
            "cursor": point(self.local.cursor),
            "selection": self.local.selection,
            "viewport": self.local.viewport.map_or(Value::Null, |v| json!({"x": v.x, "y": v.y, "scale": v.scale})),
            "client": "native",
        })
        .to_string()
    }

    /// Frames due now: a join after a dropped membership, the merged local
    /// updates (every [`BATCH`], within the server's rate and update size)
    /// and this console's presence (also as a keepalive).
    pub fn outgoing(&mut self, now: Instant) -> Vec<ClientMessage> {
        let mut out = Vec::new();
        self.now = self.now.max(now);
        if let Some(at) = self.rejoin_at
            && now >= at
        {
            self.rejoin_at = None;
            self.phase = Phase::Joining { since: now };
            out.push(self.join_frame());
            return out;
        }
        if self.phase != Phase::Live {
            return out;
        }
        while self
            .sent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= Duration::from_secs(1))
        {
            self.sent.pop_front();
        }
        let due = self
            .last_send
            .is_none_or(|t| now.saturating_duration_since(t) >= BATCH);
        if due && self.sent.len() < self.budget() {
            let queued = self
                .outbox
                .lock()
                .map(|mut o| mem::take(&mut *o))
                .unwrap_or_default();
            if !queued.is_empty() {
                match self.merge_queued(queued) {
                    Some((merged, touched)) => {
                        out.push(self.sync_frame(wire::update(merged), touched));
                        self.last_send = Some(now);
                    }
                    None => {
                        out.push(self.join_frame());
                        return out;
                    }
                }
            }
        }
        if self.phase == Phase::Live
            && self
                .last_presence
                .is_some_and(|t| now.saturating_duration_since(t) >= self.idle() / 3)
        {
            // A keepalive: the server removes members that stay silent.
            self.presence_dirty = true;
        }
        let presence_due = self
            .last_presence
            .is_none_or(|t| now.saturating_duration_since(t) >= PRESENCE_EVERY);
        if self.presence_dirty && presence_due && !self.user.is_empty() {
            self.clock += 1;
            out.push(ClientMessage::DesignAwareness {
                screen: self.screen.clone(),
                data: wire::encode_awareness(&[Presence {
                    client: self.doc.client_id().get(),
                    clock: self.clock,
                    json: self.presence_json(),
                }]),
            });
            self.presence_dirty = false;
            self.last_presence = Some(now);
        }
        out
    }

    /// Merge as many queued updates (oldest first) as fit in one message;
    /// the rest go back to the outbox. A single update too large to send:
    /// it is taken back (the session starts again from the server's copy,
    /// writing this console's other unflushed edits again: join again) and
    /// there is nothing to send.
    fn merge_queued(&mut self, mut queued: Vec<(Vec<u8>, Touched)>) -> Option<(Vec<u8>, Touched)> {
        let limit = self.limits.as_ref().map_or(usize::MAX, |l| {
            l.max_update.saturating_sub(UPDATE_MARGIN).max(1)
        });
        let merge = |part: &[(Vec<u8>, Touched)], doc: &Doc| -> Vec<u8> {
            if let [(one, _)] = part {
                return one.clone();
            }
            let updates: Vec<&[u8]> = part.iter().map(|(u, _)| u.as_slice()).collect();
            voltrus_collab::yrs::merge_updates_v1(&updates).unwrap_or_else(|_| {
                // Unmergeable (never expected): the whole state.
                wire::state(doc)
            })
        };
        let mut take = queued.len();
        let merged = loop {
            let merged = merge(&queued[..take], &self.doc);
            if merged.len() <= limit || take == 1 {
                break merged;
            }
            take = take.div_ceil(2).min(take - 1);
        };
        let rest = queued.split_off(take);
        if merged.len() > limit {
            let mut others = self.unacked();
            for (_, t) in &rest {
                others.merge(t);
            }
            for (_, t) in &queued {
                for object in t.0.keys() {
                    forget(&mut others, object);
                }
            }
            let mine = document::read(&self.doc.transact());
            self.rebuild(others, mine, false);
            self.phase = Phase::Joining { since: self.now };
            self.notice = Some((
                format!(
                    "This change is too large to share live ({}, the limit is {}): it was taken back",
                    size_text(merged.len()),
                    size_text(limit)
                ),
                true,
            ));
            return None;
        }
        if let Ok(mut out) = self.outbox.lock() {
            let newer = mem::take(&mut *out);
            *out = rest;
            out.extend(newer);
        }
        let mut touched = Touched::default();
        for (_, t) in &queued {
            touched.merge(t);
        }
        Some((merged, touched))
    }

    /// ⌘S in a session: write the pending edits to the draft now.
    pub fn flush_frame(&self) -> ClientMessage {
        ClientMessage::DesignFlush {
            screen: self.screen.clone(),
        }
    }

    /// Leaving the Designer.
    pub fn leave_frame(&self) -> ClientMessage {
        ClientMessage::DesignLeave {
            screen: self.screen.clone(),
        }
    }
}

#[cfg(test)]
#[path = "collab_tests.rs"]
mod tests;
