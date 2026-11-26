//! Sessions against a relay that plays the server's part with the shared
//! crate: no network, no GPU.

use std::time::{Duration, Instant};

use serde_json::json;
use voltrus_collab::document;
use voltrus_collab::wire::{self, Handled, Presence};
use voltrus_collab::yrs::{Doc, Transact};

use super::*;
use crate::mimic::{Connection, Widget, WidgetKind};
use crate::ws_types::DesignLimits;

const SCREEN: &str = "s1";

fn no_names(_: i64) -> Option<String> {
    None
}

fn widget(id: &str, x: f64) -> Widget {
    Widget {
        id: id.into(),
        kind: WidgetKind::parse("tank_level"),
        name: id.to_uppercase(),
        tags: vec![1],
        signals: Vec::new(),
        x,
        y: 10.0,
        width: 80.0,
        height: 120.0,
        config: serde_json::Map::new(),
    }
}

fn start() -> Layout {
    Layout {
        widgets: vec![widget("a", 0.0), widget("b", 200.0)],
        connections: vec![Connection {
            id: "c1".into(),
            from: "a".into(),
            to: "b".into(),
            kind: "flow".into(),
            waypoints: Vec::new(),
        }],
    }
}

fn limits() -> DesignLimits {
    DesignLimits {
        max_update: 256 * 1024,
        max_rate: 50,
        max_doc: 5 << 20,
        max_awareness: 4096,
        flush_ms: 2000,
        idle_ms: 0,
    }
}

/// The server's part: one document, relayed updates and presence.
struct Relay {
    doc: Doc,
    epoch: String,
    limits: DesignLimits,
    /// Frames for each console, in order.
    inbox: Vec<Vec<ServerMessage>>,
    /// Whether each console is a member (else its sync messages are
    /// dropped with `not_joined`).
    member: Vec<bool>,
    /// `DesignSync` messages from each console since its `DesignJoined`.
    count: Vec<u64>,
}

impl Relay {
    fn new(layout: &Layout) -> Self {
        let doc = Doc::new();
        let (eq, cn) = mimic::to_rows(layout);
        document::load(&mut doc.transact_mut(), &collab_rows(&eq, &cn));
        Self {
            doc,
            epoch: "e1".into(),
            limits: limits(),
            inbox: Vec::new(),
            member: Vec::new(),
            count: Vec::new(),
        }
    }

    fn connect(&mut self) -> usize {
        self.inbox.push(Vec::new());
        self.member.push(false);
        self.count.push(0);
        self.inbox.len() - 1
    }

    /// The server forgets `who` (it does not say so until `who` sends).
    fn forget(&mut self, who: usize) {
        self.member[who] = false;
    }

    /// The server drops `who` and says why.
    fn drop_member(&mut self, who: usize, code: &str) {
        self.member[who] = false;
        self.inbox[who].push(error(code));
    }

    /// A write to the draft as `who` is told of it: what it covers now.
    fn flushed(&self, who: usize) -> ServerMessage {
        ServerMessage::DesignFlushed {
            screen: SCREEN.into(),
            seq: 1,
            at_ms: 0,
            users: Vec::new(),
            acked: Some(self.count[who]),
        }
    }

    fn from(&mut self, who: usize, message: ClientMessage) {
        match message {
            ClientMessage::DesignJoin { screen, .. } => {
                self.member[who] = true;
                self.count[who] = 0;
                self.inbox[who].push(ServerMessage::DesignJoined {
                    screen: screen.clone(),
                    epoch: self.epoch.clone(),
                    resync: false,
                    user: format!("user{who}"),
                    color: "#3366ff".into(),
                    protocol: 1,
                    limits: self.limits.clone(),
                });
                self.inbox[who].push(ServerMessage::DesignSync {
                    screen,
                    data: wire::step1(&self.doc),
                });
            }
            ClientMessage::DesignSync { screen, data } => {
                if !self.member[who] {
                    self.inbox[who].push(error("not_joined"));
                    return;
                }
                self.count[who] += 1;
                let msg = wire::decode(&data).unwrap();
                let payload = match &msg {
                    wire::Sync::SyncStep2(u) | wire::Sync::Update(u) => Some(u.clone()),
                    wire::Sync::SyncStep1(_) => None,
                };
                match wire::handle(&self.doc, &format!("c{who}"), msg).unwrap() {
                    Handled::Reply(reply) => self.inbox[who].push(ServerMessage::DesignSync {
                        screen,
                        data: reply,
                    }),
                    Handled::Applied => {
                        for (other, inbox) in self.inbox.iter_mut().enumerate() {
                            if other != who
                                && let Some(u) = &payload
                            {
                                inbox.push(ServerMessage::DesignSync {
                                    screen: screen.clone(),
                                    data: wire::update(u.clone()),
                                });
                            }
                        }
                    }
                }
            }
            ClientMessage::DesignAwareness { screen, data } => {
                for (other, inbox) in self.inbox.iter_mut().enumerate() {
                    if other != who {
                        inbox.push(ServerMessage::DesignAwareness {
                            screen: screen.clone(),
                            data: data.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

fn error(code: &str) -> ServerMessage {
    ServerMessage::DesignError {
        screen: SCREEN.into(),
        code: code.into(),
        message: code.into(),
        reload: false,
    }
}

/// A console: a session and the editor's layout.
struct Console {
    id: usize,
    session: Session,
    layout: Layout,
    /// What the user was told.
    toasts: Vec<(String, bool)>,
    /// The time of the latest `send`, and when each `DesignSync` went out.
    clock: Instant,
    syncs: Vec<Instant>,
}

impl Console {
    fn open(relay: &mut Relay, layout: &Layout, now: Instant) -> Self {
        let id = relay.connect();
        let (session, join) = Session::join(SCREEN, layout, now);
        relay.from(id, join);
        Self {
            id,
            session,
            layout: layout.clone(),
            toasts: Vec::new(),
            clock: now,
            syncs: Vec::new(),
        }
    }

    fn deliver(&mut self, relay: &mut Relay, message: ClientMessage) {
        if matches!(message, ClientMessage::DesignSync { .. }) {
            self.syncs.push(self.clock);
        }
        relay.from(self.id, message);
    }

    /// Read the relay's frames for this console, answer them.
    fn pump(&mut self, relay: &mut Relay) -> bool {
        let frames = std::mem::take(&mut relay.inbox[self.id]);
        let any = !frames.is_empty();
        for frame in frames {
            let r = self.session.on_server(&frame, &self.layout, &no_names);
            if let Some(layout) = r.layout {
                self.layout = layout;
            }
            self.toasts.extend(r.toast);
            for message in r.send {
                self.deliver(relay, message);
            }
        }
        any
    }

    fn edit(&mut self, new_step: bool, f: impl FnOnce(&mut Layout)) {
        f(&mut self.layout);
        self.session.local(&self.layout, new_step, &no_names);
        self.layout = self.session.layout().clone();
    }

    fn send(&mut self, relay: &mut Relay, now: Instant) {
        self.clock = now;
        for message in self.session.outgoing(now) {
            self.deliver(relay, message);
        }
        self.toasts.extend(self.session.take_notice());
    }
}

fn settle(relay: &mut Relay, consoles: &mut [&mut Console], now: Instant) {
    for _ in 0..20 {
        let mut any = false;
        for c in consoles.iter_mut() {
            c.send(relay, now);
            any |= c.pump(relay);
        }
        if !any {
            break;
        }
    }
}

fn x_of(layout: &Layout, id: &str) -> Option<f64> {
    layout.widget(id).map(|w| w.x)
}

#[test]
fn joining_brings_the_servers_document() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    // This console loaded an older copy: the server's wins.
    let mut old = start();
    old.widgets[0].x = 999.0;
    // Not dirty: what it shows is the server's, not a replay of the stale copy.
    let mut c = Console::open(&mut relay, &old, now);
    settle(&mut relay, &mut [&mut c], now);
    assert!(c.session.active());
    assert_eq!(x_of(&c.layout, "a"), Some(0.0));
    assert_eq!(c.layout.connections.len(), 1);
}

#[test]
fn a_local_edit_reaches_the_other_console() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    let mut b = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a, &mut b], now);
    a.edit(true, |l| l.widgets[0].x = 55.0);
    settle(&mut relay, &mut [&mut a, &mut b], now + BATCH);
    assert_eq!(x_of(&b.layout, "a"), Some(55.0));
}

#[test]
fn concurrent_edits_converge() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    let mut b = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a, &mut b], now);
    // Different widgets, and different config keys of one widget.
    a.edit(true, |l| {
        l.widgets[0].x = 11.0;
        l.widgets[1].config.insert("label".into(), json!("Tank B"));
    });
    b.edit(true, |l| {
        l.widgets[1].y = 77.0;
        l.widgets[1].config.insert("visible".into(), json!(false));
    });
    let later = now + BATCH;
    a.send(&mut relay, later);
    b.send(&mut relay, later);
    settle(&mut relay, &mut [&mut a, &mut b], later);
    assert_eq!(a.layout, b.layout);
    let w = b.layout.widget("b").unwrap();
    assert_eq!((w.y, w.text("label")), (77.0, Some("Tank B")));
    assert_eq!(w.flag("visible"), Some(false));
    assert_eq!(x_of(&a.layout, "a"), Some(11.0));
    // The same field: both end with one value.
    a.edit(true, |l| l.widgets[0].x = 1.0);
    b.edit(true, |l| l.widgets[0].x = 2.0);
    let later = later + BATCH;
    a.send(&mut relay, later);
    b.send(&mut relay, later);
    settle(&mut relay, &mut [&mut a, &mut b], later);
    assert_eq!(a.layout, b.layout);
}

#[test]
fn a_remote_update_keeps_the_local_selection() {
    // The editor keeps its own selection; a remote update only replaces
    // the layout, and an id that still exists stays selected.
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    let mut b = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a, &mut b], now);
    let selection = ["b".to_string()];
    a.edit(true, |l| l.widgets[0].x = 300.0);
    settle(&mut relay, &mut [&mut a, &mut b], now + BATCH);
    assert!(selection.iter().all(|id| b.layout.widget(id).is_some()));
    assert_eq!(x_of(&b.layout, "a"), Some(300.0));
}

#[test]
fn undo_takes_back_only_own_edits() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    let mut b = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a, &mut b], now);
    a.edit(true, |l| l.widgets[0].x = 40.0);
    let t = now + BATCH;
    settle(&mut relay, &mut [&mut a, &mut b], t);
    b.edit(true, |l| l.widgets[1].x = 500.0);
    let t = t + BATCH;
    settle(&mut relay, &mut [&mut a, &mut b], t);
    assert!(a.session.can_undo());
    a.layout = a.session.undo().expect("an undo step");
    let t = t + BATCH;
    settle(&mut relay, &mut [&mut a, &mut b], t);
    assert_eq!(x_of(&a.layout, "a"), Some(0.0), "own move undone");
    assert_eq!(x_of(&a.layout, "b"), Some(500.0), "the other's stays");
    assert_eq!(b.layout, a.layout);
    a.layout = a.session.redo().expect("a redo step");
    assert_eq!(x_of(&a.layout, "a"), Some(40.0));
}

#[test]
fn steps_of_one_drag_are_one_undo_item() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a], now);
    a.edit(true, |l| l.widgets[0].x = 10.0);
    a.edit(false, |l| l.widgets[0].x = 20.0);
    a.edit(false, |l| l.widgets[0].x = 30.0);
    a.edit(true, |l| l.widgets[1].x = 1.0);
    a.layout = a.session.undo().unwrap();
    assert_eq!(x_of(&a.layout, "a"), Some(30.0));
    a.layout = a.session.undo().unwrap();
    assert_eq!(x_of(&a.layout, "a"), Some(0.0), "the whole drag at once");
}

#[test]
fn unsaved_work_from_before_joining_is_written_into_the_session() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let saved = start();
    let mut c = Console::open(&mut relay, &saved, now);
    // Edited before the server answered.
    c.layout.widgets[1].x = 640.0;
    settle(&mut relay, &mut [&mut c], now);
    assert_eq!(x_of(&c.layout, "b"), Some(640.0));
    assert!(!c.session.can_undo(), "joining is not an undo step");
    let mut other = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut c, &mut other], now + BATCH);
    assert_eq!(x_of(&other.layout, "b"), Some(640.0));
}

#[test]
fn edits_offline_merge_on_reconnect_and_survive_a_server_restart() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a], now);
    a.session.offline();
    a.edit(true, |l| l.widgets[0].x = 70.0);
    assert!(
        a.session.outgoing(now + BATCH).is_empty(),
        "nothing goes out offline"
    );
    // Same session on the server: the y-sync exchange carries the edit.
    let join = a.session.reconnect(now).unwrap();
    relay.from(a.id, join);
    settle(&mut relay, &mut [&mut a], now + BATCH);
    let probe = document::read(&relay.doc.transact());
    assert_eq!(probe.widgets["a"].0.x, 70.0);

    // The server restarted: a new document from the rows, a new epoch.
    a.session.offline();
    a.edit(true, |l| l.widgets[1].x = 90.0);
    let mut fresh = Relay::new(&start());
    fresh.epoch = "e2".into();
    fresh.connect();
    let join = a.session.reconnect(now).unwrap();
    fresh.from(0, join);
    let t = now + BATCH * 2;
    settle(&mut fresh, &mut [&mut a], t);
    let s = document::read(&fresh.doc.transact());
    assert_eq!(
        s.widgets["b"].0.x, 90.0,
        "the offline edit is written again"
    );
}

#[test]
fn presence_goes_both_ways_with_the_users_identity() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    let mut b = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a, &mut b], now);
    a.session.set_presence(
        Some((100.4, 50.0)),
        &["b".to_string()],
        Some(Viewport {
            x: 960.0,
            y: 540.0,
            scale: 0.5,
        }),
    );
    settle(&mut relay, &mut [&mut a, &mut b], now + PRESENCE_EVERY);
    let peer = b.session.peers.values().next().expect("a peer");
    assert_eq!(peer.name, "user0");
    assert_eq!(peer.cursor, Some((100.0, 50.0)));
    assert_eq!(peer.selection, vec!["b".to_string()]);
    assert_eq!(peer.viewport.map(|v| v.scale), Some(0.5));
    // A removal (state null) drops the peer.
    let gone = wire::encode_awareness(&[Presence {
        client: peer.client,
        clock: 99,
        json: "null".into(),
    }]);
    b.session.on_server(
        &ServerMessage::DesignAwareness {
            screen: SCREEN.into(),
            data: gone,
        },
        &b.layout,
        &no_names,
    );
    assert!(b.session.peers.is_empty());
}

#[test]
fn an_older_server_or_a_refusal_falls_back_to_saving_layouts() {
    let now = Instant::now();
    let (mut s, _) = Session::join(SCREEN, &start(), now);
    assert!(!s.expire(now + Duration::from_secs(1)));
    assert!(s.expire(now + JOIN_TIMEOUT + Duration::from_millis(1)));
    assert!(s.off() && !s.active());
    assert!(s.reconnect(now).is_none(), "no more joining");

    let (mut s, _) = Session::join(SCREEN, &start(), now);
    let r = s.on_server(
        &ServerMessage::DesignError {
            screen: SCREEN.into(),
            code: "disabled".into(),
            message: "Collaboration is off".into(),
            reload: false,
        },
        &start(),
        &no_names,
    );
    assert!(r.fallback && s.off());
    // A frame for another screen is not this session's.
    let (mut s, _) = Session::join(SCREEN, &start(), now);
    let r = s.on_server(
        &ServerMessage::DesignError {
            screen: "other".into(),
            code: "disabled".into(),
            message: String::new(),
            reload: false,
        },
        &start(),
        &no_names,
    );
    assert!(!r.fallback && !s.off());
}

#[test]
fn updates_are_batched() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = Console::open(&mut relay, &start(), now);
    settle(&mut relay, &mut [&mut a], now);
    let _ = a.session.outgoing(now);
    for i in 0..5 {
        a.edit(false, |l| l.widgets[0].x = f64::from(i));
    }
    let first = a.session.outgoing(now + BATCH);
    assert_eq!(
        first
            .iter()
            .filter(|m| matches!(m, ClientMessage::DesignSync { .. }))
            .count(),
        1,
        "five edits, one frame"
    );
    a.edit(false, |l| l.widgets[0].x = 99.0);
    assert!(
        a.session
            .outgoing(now + BATCH + Duration::from_millis(5))
            .iter()
            .all(|m| !matches!(m, ClientMessage::DesignSync { .. })),
        "not before the batch interval"
    );
}

#[test]
fn names_and_colours() {
    assert_eq!(initials("ana.lee"), "AL");
    assert_eq!(initials("admin"), "AD");
    assert_eq!(initials(""), "?");
    assert_eq!(color_rgb("#3366ff"), Some(0x3366ff));
    assert_eq!(color_rgb("blue"), None);
}

// ── Membership, acknowledgements, restarts, pacing ──

fn relay_x(relay: &Relay, id: &str) -> Option<f64> {
    document::read(&relay.doc.transact())
        .widgets
        .get(id)
        .map(|(w, _)| w.x)
}

fn joined(relay: &mut Relay, now: Instant) -> Console {
    let mut c = Console::open(relay, &start(), now);
    settle(relay, &mut [&mut c], now);
    assert!(c.session.active());
    c
}

#[test]
fn a_dropped_membership_rejoins_keeping_the_document() {
    for code in ["resync", "not_joined", "idle"] {
        let now = Instant::now();
        let mut relay = Relay::new(&start());
        let mut a = joined(&mut relay, now);
        let mut b = joined(&mut relay, now);
        a.edit(true, |l| l.widgets[0].x = 55.0);
        // The server lost the membership: the update is dropped, the
        // server says so, and the console joins again at once.
        if code == "not_joined" {
            relay.forget(a.id);
        } else {
            relay.drop_member(a.id, code);
        }
        let t = now + BATCH;
        a.send(&mut relay, t);
        assert_eq!(relay_x(&relay, "a"), Some(0.0), "{code}: dropped");
        a.pump(&mut relay);
        assert!(relay.member[a.id], "{code}: joined again");
        settle(&mut relay, &mut [&mut a, &mut b], t);
        assert_eq!(
            relay_x(&relay, "a"),
            Some(55.0),
            "{code}: the exchange carries it"
        );
        assert_eq!(x_of(&b.layout, "a"), Some(55.0));
        assert_eq!(
            x_of(&a.layout, "a"),
            Some(55.0),
            "{code}: the document is kept"
        );
        assert!(a.session.can_undo(), "{code}: and its undo");
        assert!(a.toasts.iter().all(|(_, error)| !error), "{code}: quietly");
    }
}

#[test]
fn rate_limited_backs_off_and_repeats_slow_down() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = joined(&mut relay, now);
    a.edit(true, |l| l.widgets[0].x = 21.0);
    relay.drop_member(a.id, "rate_limited");
    a.pump(&mut relay);
    assert!(!relay.member[a.id], "not at once");
    a.send(&mut relay, now + Duration::from_millis(500));
    assert!(!relay.member[a.id], "not within a second");
    a.send(&mut relay, now + Duration::from_millis(1000));
    assert!(relay.member[a.id], "after about a second");
    let t = now + Duration::from_millis(1100);
    settle(&mut relay, &mut [&mut a], t);
    assert_eq!(relay_x(&relay, "a"), Some(21.0));
    // Dropped again right away: no loop, the next join waits longer.
    relay.drop_member(a.id, "not_joined");
    a.pump(&mut relay);
    assert!(!relay.member[a.id], "a second drop in a row waits");
    a.send(&mut relay, t + Duration::from_millis(500));
    assert!(!relay.member[a.id]);
    a.send(&mut relay, t + Duration::from_secs(1));
    assert!(relay.member[a.id]);
}

#[test]
fn a_flush_acknowledges_messages_by_number() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = joined(&mut relay, now);
    assert!(!a.session.unflushed(), "nothing edited");
    a.edit(true, |l| l.widgets[0].x = 1.0);
    assert!(a.session.unflushed(), "queued");
    let t = now + BATCH;
    a.send(&mut relay, t);
    assert!(a.session.unflushed(), "sent, not written");
    // The server writes what it has now; its answer crosses a newer edit.
    let flushed = relay.flushed(a.id);
    a.edit(true, |l| l.widgets[1].x = 2.0);
    a.send(&mut relay, t + BATCH);
    a.session.on_server(&flushed, &a.layout, &no_names);
    assert!(a.session.unflushed(), "the newer update is not covered");
    let flushed = relay.flushed(a.id);
    a.session.on_server(&flushed, &a.layout, &no_names);
    assert!(!a.session.unflushed(), "all written");
    // An older server's flush (no `acked`) covers what was sent before it.
    a.edit(true, |l| l.widgets[1].x = 3.0);
    a.send(&mut relay, t + BATCH * 2);
    let old = ServerMessage::DesignFlushed {
        screen: SCREEN.into(),
        seq: 2,
        at_ms: 0,
        users: vec!["user0".into()],
        acked: None,
    };
    a.session.on_server(&old, &a.layout, &no_names);
    assert!(!a.session.unflushed());
}

#[test]
fn sent_but_unflushed_edits_survive_a_server_restart() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = joined(&mut relay, now);
    // Written to the draft before the restart: not replayed.
    a.edit(true, |l| l.widgets[1].x = 90.0);
    let t = now + BATCH;
    settle(&mut relay, &mut [&mut a], t);
    let flushed = relay.flushed(a.id);
    a.session.on_server(&flushed, &a.layout, &no_names);
    // Sent, applied by the server, never written.
    a.edit(true, |l| l.widgets[0].x = 70.0);
    let t = t + BATCH;
    settle(&mut relay, &mut [&mut a], t);
    assert_eq!(relay_x(&relay, "a"), Some(70.0));
    assert!(a.session.unflushed());

    // The server restarts from the draft rows (b at 90, a at 0); someone
    // moves b again before this console is back.
    let mut draft = start();
    draft.widgets[1].x = 90.0;
    let mut fresh = Relay::new(&draft);
    fresh.epoch = "e2".into();
    {
        let mut txn = fresh.doc.transact_mut();
        let mut b = document::read(&txn).widgets["b"].0.clone();
        b.x = 95.0;
        document::put_widget(&mut txn, "b", Some(&b), None);
    }
    a.session.offline();
    fresh.connect();
    let join = a.session.reconnect(t).unwrap();
    fresh.from(0, join);
    settle(&mut fresh, &mut [&mut a], t + BATCH);
    assert_eq!(relay_x(&fresh, "a"), Some(70.0), "the unflushed move again");
    assert_eq!(relay_x(&fresh, "b"), Some(95.0), "the flushed one is not");
    assert_eq!(a.layout, layout_of(&a.session.rows));
    assert!(
        a.toasts
            .iter()
            .any(|(text, _)| text
                == "The live session restarted; 1 unsaved change was applied again"),
        "{:?}",
        a.toasts
    );
    assert!(!a.session.can_undo(), "the replay is not an undo step");
}

#[test]
fn an_offline_edit_does_not_bring_back_what_others_deleted() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = joined(&mut relay, now);
    a.session.offline();
    a.edit(true, |l| l.widgets[1].x = 300.0);
    // Meanwhile the server restarted and b was deleted.
    let mut fresh = Relay::new(&start());
    fresh.epoch = "e2".into();
    {
        let mut txn = fresh.doc.transact_mut();
        document::put_widget(&mut txn, "b", None, None);
    }
    fresh.connect();
    let join = a.session.reconnect(now).unwrap();
    fresh.from(0, join);
    settle(&mut fresh, &mut [&mut a], now + BATCH);
    assert_eq!(relay_x(&fresh, "b"), None, "stays deleted");
    assert!(a.layout.widget("b").is_none());
}

#[test]
fn an_offline_edit_and_another_field_edited_meanwhile_both_stand() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    let mut a = joined(&mut relay, now);
    a.session.offline();
    a.edit(true, |l| l.widgets[1].x = 300.0);
    let mut fresh = Relay::new(&start());
    fresh.epoch = "e2".into();
    {
        let mut txn = fresh.doc.transact_mut();
        let mut b = document::read(&txn).widgets["b"].0.clone();
        b.y = 444.0;
        document::put_widget(&mut txn, "b", Some(&b), None);
    }
    fresh.connect();
    let join = a.session.reconnect(now).unwrap();
    fresh.from(0, join);
    settle(&mut fresh, &mut [&mut a], now + BATCH);
    let s = document::read(&fresh.doc.transact());
    assert_eq!((s.widgets["b"].0.x, s.widgets["b"].0.y), (300.0, 444.0));
    let w = a.layout.widget("b").unwrap();
    assert_eq!((w.x, w.y), (300.0, 444.0));
}

#[test]
fn sync_messages_stay_under_the_rate() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    relay.limits.max_rate = 15;
    let budget = 5;
    let mut a = joined(&mut relay, now);
    // An edit every 10 ms for three seconds, a send attempt each time.
    for i in 1..=300u32 {
        let t = now + Duration::from_millis(u64::from(i) * 10);
        a.edit(false, |l| l.widgets[0].x = f64::from(i));
        a.send(&mut relay, t);
        a.pump(&mut relay);
    }
    let syncs = &a.syncs;
    assert!(syncs.len() > budget, "updates went out: {}", syncs.len());
    for (i, t) in syncs.iter().enumerate() {
        let within = syncs[i..]
            .iter()
            .take_while(|u| u.saturating_duration_since(*t) < Duration::from_secs(1))
            .count();
        assert!(within <= budget, "{within} sync messages within a second");
    }
    // What was held goes out in the end.
    settle(&mut relay, &mut [&mut a], now + Duration::from_secs(5));
    assert_eq!(relay_x(&relay, "a"), Some(300.0));
}

#[test]
fn queued_updates_are_split_under_the_size_limit() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    relay.limits.max_update = 1024;
    let mut a = joined(&mut relay, now);
    let text = |c: char| c.to_string().repeat(300);
    a.edit(true, |l| {
        l.widgets[0].config.insert("label".into(), json!(text('x')));
    });
    a.edit(true, |l| {
        l.widgets[1].config.insert("label".into(), json!(text('y')));
    });
    let first = a.session.outgoing(now + BATCH);
    let sizes: Vec<usize> = first
        .iter()
        .filter_map(|m| match m {
            ClientMessage::DesignSync { data, .. } => Some(data.len()),
            _ => None,
        })
        .collect();
    assert_eq!(sizes.len(), 1, "one frame per batch");
    assert!(sizes[0] <= 1024 - 64, "{sizes:?}");
    assert!(a.session.unsent(), "the other waits");
    for m in first {
        a.deliver(&mut relay, m);
    }
    settle(&mut relay, &mut [&mut a], now + BATCH * 2);
    let s = document::read(&relay.doc.transact());
    assert_eq!(s.widgets["a"].0.config["label"], json!(text('x')));
    assert_eq!(s.widgets["b"].0.config["label"], json!(text('y')));
}

#[test]
fn an_update_too_large_to_send_is_taken_back() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    relay.limits.max_update = 1024;
    let mut a = joined(&mut relay, now);
    // A small edit sent and not written, then one too large.
    a.edit(true, |l| l.widgets[1].x = 33.0);
    let t = now + BATCH;
    settle(&mut relay, &mut [&mut a], t);
    a.edit(true, |l| {
        l.widgets[0]
            .config
            .insert("label".into(), json!("z".repeat(4000)));
    });
    let t = t + BATCH;
    let out = a.session.outgoing(t);
    assert!(
        out.iter()
            .all(|m| !matches!(m, ClientMessage::DesignSync { .. })),
        "not sent"
    );
    assert!(
        out.iter()
            .any(|m| matches!(m, ClientMessage::DesignJoin { .. }))
    );
    let (text, error) = a.session.take_notice().expect("the user is told");
    assert!(error && text.contains("too large"), "{text}");
    for m in out {
        a.deliver(&mut relay, m);
    }
    settle(&mut relay, &mut [&mut a], t);
    assert_eq!(a.layout.widget("a").unwrap().text("label"), None);
    assert_eq!(x_of(&a.layout, "b"), Some(33.0), "the other edit stands");
    assert_eq!(relay_x(&relay, "b"), Some(33.0));
}

#[test]
fn presence_is_a_keepalive() {
    let now = Instant::now();
    let mut relay = Relay::new(&start());
    relay.limits.idle_ms = 3000;
    let mut a = joined(&mut relay, now);
    let presence = |out: &[ClientMessage]| {
        out.iter()
            .filter(|m| matches!(m, ClientMessage::DesignAwareness { .. }))
            .count()
    };
    assert_eq!(
        presence(&a.session.outgoing(now + Duration::from_millis(500))),
        0
    );
    assert_eq!(
        presence(&a.session.outgoing(now + Duration::from_millis(1000))),
        1
    );
    assert_eq!(
        presence(&a.session.outgoing(now + Duration::from_millis(1500))),
        0
    );
    assert_eq!(
        presence(&a.session.outgoing(now + Duration::from_millis(2000))),
        1
    );
}

#[test]
fn a_stack_bomb_from_the_server_is_refused() {
    // On a small stack, as the console's tasks may run on: a recursive
    // decode would overflow it.
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let now = Instant::now();
            let mut relay = Relay::new(&start());
            let mut a = joined(&mut relay, now);
            // `guard::tests::stack_bomb`: an item `widgets.w` holding
            // `[117, 1]` 120 000 times (an array of an array of …).
            const ANY: u8 = 8;
            const HAS_PARENT_SUB: u8 = 0b0010_0000;
            let root = document::WIDGETS.as_bytes();
            let mut bomb = vec![1u8, 1, 5, 0, ANY | HAS_PARENT_SUB, 1, root.len() as u8];
            bomb.extend(root);
            bomb.extend([1, b'w', 1]);
            for _ in 0..120_000 {
                bomb.extend([117, 1]);
            }
            bomb.extend([125, 1, 0]);
            let before = a.layout.clone();
            let state = wire::state(&a.session.doc);
            let r = a.session.on_server(
                &ServerMessage::DesignSync {
                    screen: SCREEN.into(),
                    data: wire::update(bomb),
                },
                &a.layout,
                &no_names,
            );
            let (text, error) = r.toast.expect("an error to show");
            assert!(error, "{text}");
            assert!(r.layout.is_none());
            assert_eq!(*a.session.layout(), before);
            assert_eq!(wire::state(&a.session.doc), state, "nothing applied");
            assert!(a.session.active(), "the session goes on");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn bringing_one_widget_to_front_rewrites_only_its_stacking_index() {
    let now = Instant::now();
    let mut three = start();
    three.widgets.push(widget("c", 400.0));
    let mut relay = Relay::new(&three);
    let mut a = Console::open(&mut relay, &three, now);
    settle(&mut relay, &mut [&mut a], now);
    let before = document::read(&relay.doc.transact());
    a.edit(true, |l| {
        let w = l.widgets.remove(0);
        l.widgets.push(w);
    });
    settle(&mut relay, &mut [&mut a], now + BATCH);
    let after = document::read(&relay.doc.transact());
    assert_eq!(after.order(), ["b", "c", "a"]);
    let touched = document::touched_between(&before, &after);
    let mut zi = std::collections::BTreeSet::new();
    zi.insert("zi".to_string());
    assert_eq!(
        touched.0.into_iter().collect::<Vec<_>>(),
        vec![(
            (document::ObjectKind::Widget, "a".to_string()),
            document::Fields::Some(zi)
        )]
    );
}
