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
    }
}

/// The server's part: one document, relayed updates and presence.
struct Relay {
    doc: Doc,
    epoch: String,
    /// Frames for each console, in order.
    inbox: Vec<Vec<ServerMessage>>,
}

impl Relay {
    fn new(layout: &Layout) -> Self {
        let doc = Doc::new();
        let (eq, cn) = mimic::to_rows(layout);
        document::load(&mut doc.transact_mut(), &collab_rows(&eq, &cn));
        Self {
            doc,
            epoch: "e1".into(),
            inbox: Vec::new(),
        }
    }

    fn connect(&mut self) -> usize {
        self.inbox.push(Vec::new());
        self.inbox.len() - 1
    }

    fn from(&mut self, who: usize, message: ClientMessage) {
        match message {
            ClientMessage::DesignJoin { screen, .. } => {
                self.inbox[who].push(ServerMessage::DesignJoined {
                    screen: screen.clone(),
                    epoch: self.epoch.clone(),
                    resync: false,
                    user: format!("user{who}"),
                    color: "#3366ff".into(),
                    protocol: 1,
                    limits: limits(),
                });
                self.inbox[who].push(ServerMessage::DesignSync {
                    screen,
                    data: wire::step1(&self.doc),
                });
            }
            ClientMessage::DesignSync { screen, data } => {
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

/// A console: a session and the editor's layout.
struct Console {
    id: usize,
    session: Session,
    layout: Layout,
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
        }
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
            for message in r.send {
                relay.from(self.id, message);
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
        for message in self.session.outgoing(now) {
            relay.from(self.id, message);
        }
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
