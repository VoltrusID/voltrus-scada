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
//!   Designer opened, edits made while the link was down and the server
//!   restarted meanwhile): once the server's document is in, the objects the
//!   console changed are written into it again as one edit.
//! - **Older servers**, or collaboration turned off: no `DesignJoined`
//!   within [`JOIN_TIMEOUT`], or `DesignError disabled`/`forbidden`, and the
//!   session is [`Phase::Off`]: the editor saves whole layouts as before.
//!
//! [`edit::COALESCE`]: crate::mimic::edit::COALESCE

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use voltrus_collab::document::{self, ConnectionRow as CRow, Rows, WidgetRow};
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

/// Where the session stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// `DesignJoin` sent, no answer yet.
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
    /// Updates of local and undo transactions, not sent yet.
    outbox: Outbox,
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
    /// Rows the console's own edits started from, to write again once the
    /// server's document is in.
    replay_base: Option<Rows>,
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

/// Updates waiting to go out.
type Outbox = Arc<Mutex<Vec<Vec<u8>>>>;

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
            out.push(e.update.clone());
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
            replay_base: Some(rows.clone()),
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
        self.synced && matches!(self.phase, Phase::Live | Phase::Offline)
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
    }

    pub fn can_undo(&self) -> bool {
        self.active() && self.undo.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.active() && self.undo.can_redo()
    }

    /// The link dropped: edits continue into the document; the objects
    /// changed from here are written again if the server lost the session.
    pub fn offline(&mut self) {
        if self.off() {
            return;
        }
        if self.synced && self.replay_base.is_none() {
            self.replay_base = Some(self.rows.clone());
        }
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
        if !self.synced {
            self.phase = Phase::Joining { since: now };
        }
        if let Ok(mut out) = self.outbox.lock() {
            // The step 2 of the exchange carries them.
            out.clear();
        }
        self.presence_dirty = true;
        Some(self.join_frame())
    }

    /// Joining took too long: an older server. Whether the session just
    /// turned off.
    pub fn expire(&mut self, now: Instant) -> bool {
        if let Phase::Joining { since } = self.phase
            && !self.synced
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
                if !same && self.synced {
                    // The server rebuilt the document: start from its copy
                    // and write the console's changes again.
                    if self.replay_base.is_none() {
                        self.replay_base = Some(self.rows.clone());
                    }
                    self.reset_doc();
                }
                self.epoch = Some(epoch.clone());
                self.user = user.clone();
                self.color = color.clone();
                self.limits = Some(limits.clone());
                self.phase = Phase::Live;
                self.presence_dirty = true;
                r.send.push(ClientMessage::DesignSync {
                    screen: self.screen.clone(),
                    data: wire::step1(&self.doc),
                });
            }
            ServerMessage::DesignSync { screen, data } if *screen == self.screen => {
                let Ok(msg) = wire::decode(data) else {
                    return r;
                };
                let step2 = matches!(msg, wire::Sync::SyncStep2(_));
                match wire::handle(&self.doc, REMOTE, msg) {
                    Ok(Handled::Reply(reply)) => r.send.push(ClientMessage::DesignSync {
                        screen: self.screen.clone(),
                        data: reply,
                    }),
                    Ok(Handled::Applied) => {
                        if step2 && !self.synced {
                            self.synced = true;
                            if matches!(self.phase, Phase::Offline) {
                                self.phase = Phase::Live;
                            }
                            self.refresh();
                            if let Some(base) = self.replay_base.take() {
                                let theirs = self.layout.clone();
                                let mine_rows = rows_of(&theirs, mine, tag_name);
                                self.apply_local(&base, &mine_rows);
                                // Joining is not an undo step of the user's.
                                self.undo.clear_all();
                            }
                        } else if step2 {
                            self.replay_base = None;
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
                    self.replay_base = None;
                    self.epoch = None;
                    self.reset_doc();
                    self.phase = Phase::Joining {
                        since: Instant::now(),
                    };
                    r.send.push(self.join_frame());
                    r.toast = Some((format!("Live session reloaded: {message}"), true));
                }
                _ => r.toast = Some((message.clone(), true)),
            },
            ServerMessage::DesignRejected { screen, objects } if *screen == self.screen => {
                let list: Vec<String> = objects
                    .iter()
                    .map(|o| format!("{} ({})", o.object_id, o.reason))
                    .collect();
                r.toast = Some((format!("Put back by the server: {}", list.join("; ")), true));
            }
            ServerMessage::DesignFlushed {
                screen, seq, users, ..
            } if *screen == self.screen => {
                self.flushed = Some((*seq, users.clone()));
            }
            _ => {}
        }
        r
    }

    /// Apply `before` → `after` to the document as a local edit.
    fn apply_local(&mut self, before: &Rows, after: &Rows) {
        {
            let mut txn = self.doc.transact_mut_with(Origin::from(LOCAL));
            document::apply_rows(&mut txn, before, after);
        }
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
        (self.active() && self.undo.undo_blocking()).then(|| self.refresh())
    }

    pub fn redo(&mut self) -> Option<Layout> {
        (self.active() && self.undo.redo_blocking()).then(|| self.refresh())
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

    /// Frames due now: the merged local updates (every [`BATCH`], within
    /// the server's rate) and this console's presence.
    pub fn outgoing(&mut self, now: Instant) -> Vec<ClientMessage> {
        let mut out = Vec::new();
        if self.phase != Phase::Live {
            return out;
        }
        let rate = self.limits.as_ref().map_or(20, |l| (l.max_rate / 2).max(1)) as usize;
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
        if due && self.sent.len() < rate {
            let updates = self
                .outbox
                .lock()
                .map(|mut o| std::mem::take(&mut *o))
                .unwrap_or_default();
            if !updates.is_empty() {
                let merged = if updates.len() == 1 {
                    updates.into_iter().next().unwrap_or_default()
                } else {
                    voltrus_collab::yrs::merge_updates_v1(&updates).unwrap_or_else(|_| {
                        // Unmergeable (never expected): the whole state.
                        wire::state(&self.doc)
                    })
                };
                out.push(ClientMessage::DesignSync {
                    screen: self.screen.clone(),
                    data: wire::update(merged),
                });
                self.last_send = Some(now);
                self.sent.push_back(now);
            }
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
