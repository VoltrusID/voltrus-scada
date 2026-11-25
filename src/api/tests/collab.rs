//! Live collaboration (docs/collaboration.md): clients joining a screen over
//! the WebSocket messages, converging on concurrent edits (same and different
//! objects, nested bindings, stacking order), validation putting bad edits
//! back, REST writes during a session, writes to the change log per author,
//! permissions, limits, a server restart, comments and the setting.

use std::collections::BTreeMap;

use super::*;
use http::StatusCode;
use serde_json::{Value, json};
use tokio::sync::mpsc::Receiver;
use voltrus_collab::document::{self, Rows, Widget};
use voltrus_collab::wire::{self, Handled, Presence};
use voltrus_collab::yrs::{Doc, ReadTxn, Transact, TransactionMut, WriteTxn};

use crate::api::websocket::{After, WsPrincipal, handle_client_message};
use crate::ws_manager::WsFrame;
use crate::ws_types::{ClientMessage, ServerMessage};

const PROJECT: &str = r#"
format: voltrus-project/1
tags:
- {key: p1_run, device: plc_1, name: P1 run}
- {key: level, device: plc_1, name: Level}
screens:
- id: s1
  name: Screen one
  scada:
    equipment:
    - {equipment_id: a, equipment_type: label, x: 0, config: {text: A, z: 0}}
    - {equipment_id: b, equipment_type: symbol, x: 200, sensors: [plc_1/p1_run], config: {symbol: pump, z: 1}}
    - {equipment_id: c, equipment_type: label, x: 400, config: {text: C, z: 2}}
    connections:
    - {connection_id: c1, from_equipment: a, to_equipment: b, connection_type: flow, waypoints: []}
"#;

struct T {
    app: axum::Router,
    state: AppState,
    admin: String,
}

impl T {
    async fn new() -> Self {
        let state = seeded_admin_app_state().await;
        let app = create_router(state.clone());
        let admin = format!("voltrus_token={}", admin_cookie(&app).await);
        let t = T { app, state, admin };
        t.import(PROJECT).await;
        t
    }

    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        send_request(&self.app, method, uri, None, Some(&self.admin), body).await
    }

    async fn call_as(
        &self,
        cookie: &str,
        method: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        send_request(&self.app, method, uri, None, Some(cookie), body).await
    }

    async fn user(&self, name: &str, role: &str) -> String {
        let hash = crate::api::auth::hash_password("pw-1234567");
        self.state
            .storage
            .create_user(name, &hash, "", role)
            .await
            .unwrap();
        format!(
            "voltrus_token={}",
            login_cookie(&self.app, name, "pw-1234567").await
        )
    }

    async fn import(&self, yaml: &str) {
        let req = http::Request::builder()
            .method("POST")
            .uri("/api/v1/project/import")
            .header("cookie", &self.admin)
            .header("content-type", "application/yaml")
            .body(axum::body::Body::from(yaml.to_string()))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(self.app.clone(), req)
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    async fn layout(&self) -> Value {
        self.call("GET", "/api/v1/screens/s1/scada-layout", None)
            .await
            .1
    }

    /// The draft rows of s1 by id: (x, config).
    async fn rows(&self) -> BTreeMap<String, (f64, Value)> {
        self.layout().await["equipment"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| {
                (
                    e["equipment_id"].as_str().unwrap().to_string(),
                    (
                        e["x"].as_f64().unwrap(),
                        serde_json::from_str(e["config"].as_str().unwrap()).unwrap(),
                    ),
                )
            })
            .collect()
    }

    async fn history(&self) -> Vec<Value> {
        let (s, b) = self.call("GET", "/api/v1/screens/s1/history", None).await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b["changes"].as_array().unwrap().clone()
    }

    async fn flush(&self) -> Option<crate::collab::Flushed> {
        self.state.collab.flush(&self.state, "s1").await.unwrap()
    }
}

/// A client of the session: its own document, fed by the frames the server
/// queues for its connection.
struct Client {
    conn: String,
    principal: WsPrincipal,
    rx: Receiver<WsFrame>,
    doc: Doc,
    epoch: Option<String>,
    resync: bool,
    errors: Vec<(String, bool)>,
    rejected: Vec<(String, String)>,
    flushed: Vec<i64>,
    acks: Vec<(Vec<String>, Option<u64>)>,
    presence: BTreeMap<u64, String>,
    sessions: Vec<Value>,
    comments: Vec<(String, Value)>,
    got_sync: bool,
}

impl Client {
    fn new(t: &T, conn: &str, user: &str) -> Client {
        let rx = t.state.ws_manager.connect(conn.into(), user.into(), vec![]);
        Client {
            conn: conn.into(),
            principal: WsPrincipal::User(user.into()),
            rx,
            doc: Doc::new(),
            epoch: None,
            resync: false,
            errors: vec![],
            rejected: vec![],
            flushed: vec![],
            acks: vec![],
            presence: BTreeMap::new(),
            sessions: vec![],
            comments: vec![],
            got_sync: false,
        }
    }

    async fn send(&self, t: &T, msg: ClientMessage) {
        assert_eq!(self.send_after(t, msg).await, After::Continue);
    }

    async fn send_after(&self, t: &T, msg: ClientMessage) -> After {
        handle_client_message(
            &msg,
            &self.conn,
            &t.state.ws_manager,
            &t.state,
            &self.principal,
            None,
        )
        .await
    }

    async fn sync(&self, t: &T, data: Vec<u8>) {
        self.send(
            t,
            ClientMessage::DesignSync {
                screen: "s1".into(),
                data,
            },
        )
        .await;
    }

    async fn join(&mut self, t: &T) {
        let epoch = self.epoch.clone();
        self.send(
            t,
            ClientMessage::DesignJoin {
                screen: "s1".into(),
                protocol: voltrus_collab::PROTOCOL,
                epoch,
            },
        )
        .await;
        self.pump(t).await;
        if self.epoch.is_some() {
            // Our step 1: the server answers with what we lack.
            self.send(
                t,
                ClientMessage::DesignSync {
                    screen: "s1".into(),
                    data: wire::step1(&self.doc),
                },
            )
            .await;
            self.pump(t).await;
        }
    }

    /// Handle every queued frame; answer step 1s.
    async fn pump(&mut self, t: &T) {
        loop {
            let mut replies = Vec::new();
            while let Ok(frame) = self.rx.try_recv() {
                let msg = match frame {
                    WsFrame::Message(m) => m,
                    WsFrame::Binary(b) => rmp_serde::from_slice(&b).unwrap(),
                };
                match msg {
                    ServerMessage::DesignJoined { epoch, resync, .. } => {
                        if resync {
                            self.doc = Doc::new();
                        }
                        self.resync = resync;
                        self.epoch = Some(epoch);
                    }
                    ServerMessage::DesignSync { data, .. } => {
                        self.got_sync = true;
                        let m = wire::decode(&data).unwrap();
                        if let Handled::Reply(r) = wire::handle(&self.doc, "server", m).unwrap() {
                            replies.push(r);
                        }
                    }
                    ServerMessage::DesignAwareness { data, .. } => {
                        for p in wire::decode_awareness(&data).unwrap() {
                            if p.json == "null" {
                                self.presence.remove(&p.client);
                            } else {
                                self.presence.insert(p.client, p.json);
                            }
                        }
                    }
                    ServerMessage::DesignError { code, reload, .. } => {
                        self.errors.push((code, reload))
                    }
                    ServerMessage::DesignRejected { objects, .. } => self
                        .rejected
                        .extend(objects.into_iter().map(|o| (o.object_id, o.reason))),
                    ServerMessage::DesignFlushed {
                        seq, users, acked, ..
                    } => {
                        if !users.is_empty() {
                            self.flushed.push(seq);
                        }
                        self.acks.push((users, acked));
                    }
                    ServerMessage::DesignSessions { sessions } => {
                        self.sessions = sessions
                            .iter()
                            .map(|s| serde_json::to_value(s).unwrap())
                            .collect()
                    }
                    ServerMessage::DesignComment { event, comment, .. } => {
                        self.comments.push((event, comment))
                    }
                    _ => {}
                }
            }
            if replies.is_empty() {
                return;
            }
            for r in replies {
                self.send(
                    t,
                    ClientMessage::DesignSync {
                        screen: "s1".into(),
                        data: r,
                    },
                )
                .await;
            }
        }
    }

    /// Make an edit locally and send it.
    async fn edit(&mut self, t: &T, f: impl FnOnce(&mut TransactionMut)) {
        let data = self.edit_offline(f);
        self.send(
            t,
            ClientMessage::DesignSync {
                screen: "s1".into(),
                data,
            },
        )
        .await;
    }

    fn edit_offline(&mut self, f: impl FnOnce(&mut TransactionMut)) -> Vec<u8> {
        let sv = self.doc.transact().state_vector();
        {
            let mut txn = self.doc.transact_mut();
            f(&mut txn);
        }
        wire::update(self.doc.transact().encode_diff_v1(&sv))
    }

    fn widget(&self, id: &str) -> Option<Widget> {
        document::read(&self.doc.transact())
            .widgets
            .get(id)
            .map(|(w, _)| w.clone())
    }

    fn screen(&self) -> document::Screen {
        document::read(&self.doc.transact())
    }
}

/// Change one widget in a transaction.
fn change(txn: &mut TransactionMut, id: &str, f: impl FnOnce(&mut Widget)) {
    let mut w = document::read(txn).widgets[id].0.clone();
    f(&mut w);
    document::put_widget(txn, id, Some(&w), None);
}

fn bind(w: &mut Widget, prop: &str, tag: &str) {
    let b = w.config.entry("bindings").or_insert_with(|| json!({}));
    b[prop] = json!({"kind": "tag", "tag": tag});
}

async fn sync_all(t: &T, clients: &mut [&mut Client]) {
    for _ in 0..3 {
        for c in clients.iter_mut() {
            c.pump(t).await;
        }
    }
}

fn server_rows(t: &T) -> Rows {
    t.state.collab.session("s1").unwrap().rows()
}

#[tokio::test]
async fn two_clients_converge_and_each_change_is_logged_under_its_author() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    a.pump(&t).await;
    assert!(a.got_sync && b.got_sync);
    assert_eq!(
        a.widget("b").unwrap().x,
        200.0,
        "the document came from the rows"
    );
    assert_eq!(
        a.sessions[0]["users"].as_array().unwrap().len(),
        2,
        "both are editing s1"
    );

    a.edit(&t, |txn| change(txn, "a", |w| w.x = 55.0)).await;
    b.edit(&t, |txn| {
        change(txn, "c", |w| bind(w, "label", "plc_1/level"))
    })
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.screen(), b.screen());
    assert_eq!(b.widget("a").unwrap().x, 55.0);
    assert_eq!(
        a.widget("c").unwrap().config["bindings"]["label"]["tag"],
        "plc_1/level"
    );

    let f = t.flush().await.expect("something to write");
    assert_eq!(f.users, vec!["admin".to_string(), "bob".to_string()]);
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.flushed, vec![f.seq]);
    let rows = t.rows().await;
    assert_eq!(rows["a"].0, 55.0);
    assert_eq!(rows["c"].1["bindings"]["label"]["tag"], "plc_1/level");
    assert_eq!(rows["b"].1["z"], 1, "untouched rows keep their z");
    let changes = t.history().await;
    let who: BTreeMap<String, (String, String)> = changes
        .iter()
        .take(2)
        .map(|c| {
            (
                c["object_id"].as_str().unwrap().to_string(),
                (
                    c["username"].as_str().unwrap().to_string(),
                    c["source"].as_str().unwrap().to_string(),
                ),
            )
        })
        .collect();
    assert_eq!(who["a"], ("admin".into(), "designer".into()));
    assert_eq!(who["c"], ("bob".into(), "designer".into()));
    assert_eq!(t.layout().await["seq"], f.seq);
    // Nothing new: nothing written.
    assert_eq!(t.flush().await, None);
}

#[tokio::test]
async fn concurrent_edits_of_one_widget_merge_by_field_and_binding() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    // Both edit widget b at once, without seeing each other first.
    let ua = a.edit_offline(|txn| {
        change(txn, "b", |w| {
            w.x = 10.0;
            w.config.insert("label".into(), json!("Pump A"));
            bind(w, "visible", "plc_1/p1_run");
        })
    });
    let ub = b.edit_offline(|txn| {
        change(txn, "b", |w| {
            w.x = 20.0;
            w.config.insert("rotation".into(), json!(90));
            bind(w, "label", "plc_1/level");
        })
    });
    a.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ua,
        },
    )
    .await;
    b.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ub,
        },
    )
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.screen(), b.screen(), "converged");
    let w = a.widget("b").unwrap();
    assert!(
        w.x == 10.0 || w.x == 20.0,
        "one x wins, the same everywhere"
    );
    assert_eq!(
        w.config["label"], "Pump A",
        "different config keys both stay"
    );
    assert_eq!(w.config["rotation"], 90);
    assert_eq!(
        w.config["bindings"]["visible"]["tag"], "plc_1/p1_run",
        "different bindings merge"
    );
    assert_eq!(w.config["bindings"]["label"]["tag"], "plc_1/level");
    t.flush().await.unwrap();
    let rows = t.rows().await;
    assert_eq!(rows["b"].1["bindings"].as_object().unwrap().len(), 2);
    let layout = t.layout().await;
    let row_b = layout["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "b")
        .unwrap()
        .clone();
    assert_eq!(
        row_b["rotation"], 90.0,
        "the rotation column follows config.rotation"
    );
}

#[tokio::test]
async fn stacking_order_and_a_concurrent_delete_agree() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    assert_eq!(a.screen().order(), vec!["a", "b", "c"]);
    // A sends a to the front; B deletes b (and its connection) meanwhile.
    let ua = a.edit_offline(|txn| {
        let s = document::read(txn);
        let top = s.widgets["c"].1.clone();
        let key = voltrus_collab::fractional::key_between(Some(&top), None);
        let w = s.widgets["a"].0.clone();
        document::put_widget(txn, "a", Some(&w), Some(&key));
    });
    let ub = b.edit_offline(|txn| {
        document::put_widget(txn, "b", None, None);
        document::put_connection(txn, "c1", None);
    });
    a.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ua,
        },
    )
    .await;
    b.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ub,
        },
    )
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.screen().order(), vec!["c", "a"]);
    assert_eq!(b.screen().order(), vec!["c", "a"]);
    t.flush().await.unwrap();
    let rows = t.rows().await;
    assert_eq!(
        (rows["c"].1["z"].clone(), rows["a"].1["z"].clone()),
        (json!(0), json!(1))
    );
    assert!(!rows.contains_key("b"));
    assert_eq!(t.layout().await["connections"], json!([]));
}

#[tokio::test]
async fn invalid_edits_are_put_back_and_their_author_told() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    // A binds to a tag that does not exist and moves c (valid); B adds a
    // connection to a widget A deletes at the same time.
    a.edit(&t, |txn| {
        change(txn, "a", |w| bind(w, "label", "nope/missing"));
        change(txn, "c", |w| w.x = 444.0);
    })
    .await;
    let ub = b.edit_offline(|txn| {
        let c = document::Connection {
            from: "c".into(),
            to: "b".into(),
            kind: "flow".into(),
            waypoints: json!([]),
        };
        document::put_connection(txn, "c2", Some(&c));
    });
    let ua = a.edit_offline(|txn| {
        document::put_widget(txn, "b", None, None);
        document::put_connection(txn, "c1", None);
    });
    a.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ua,
        },
    )
    .await;
    b.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: ub,
        },
    )
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    let (logger, mut audit) = crate::security::AuditLogger::new();
    let mut audited = t.state.clone();
    audited.audit_logger = logger;
    let f = audited.collab.flush(&audited, "s1").await.unwrap().unwrap();
    assert_eq!(f.rejected.len(), 2, "{:?}", f.rejected);
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.rejected.len(), 1);
    assert!(a.rejected[0].1.contains("nope/missing"), "{:?}", a.rejected);
    assert_eq!(b.rejected[0].0, "c2");
    assert!(b.rejected[0].1.contains("no longer exists"));
    // Every replica is back to the valid state; the valid edit stays.
    assert_eq!(a.screen(), b.screen());
    assert!(a.widget("a").unwrap().config.get("bindings").is_none());
    assert!(!b.screen().connections.contains_key("c2"));
    let rows = t.rows().await;
    assert_eq!(rows["c"].0, 444.0);
    assert!(rows["a"].1.get("bindings").is_none());
    assert!(!rows.contains_key("b"));
    // The rejections are in the audit log, under their authors.
    let events: Vec<_> = std::iter::from_fn(|| audit.try_recv().ok())
        .filter(|e| e.action == "design_collab_rejected")
        .map(|e| e.username)
        .collect();
    assert_eq!(events.len(), 2);
    assert!(
        events.contains(&"admin".to_string()) && events.contains(&"bob".to_string()),
        "{events:?}"
    );
}

#[tokio::test]
async fn a_rest_write_during_a_session_reaches_the_editors_and_overwrites_nobody() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    let before = t.layout().await;
    // A moves a; not written yet.
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 77.0)).await;
    // A REST client that loaded before A's edit: its base is stale once the
    // session's edits are written, so it is refused.
    let mut eq: Vec<Value> = before["equipment"].as_array().unwrap().clone();
    for e in eq.iter_mut() {
        if e["equipment_id"] == "c" {
            e["x"] = json!(999);
        }
    }
    let body = json!({"equipment": eq.clone(), "connections": before["connections"], "base_seq": before["seq"]});
    let (s, b) = t
        .call("PUT", "/api/v1/screens/s1/scada-layout", Some(body))
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(
        t.rows().await["a"].0,
        77.0,
        "the session's edit was written first"
    );
    // Based on the current head it goes through, and A sees it at once.
    let now = t.layout().await;
    let mut eq: Vec<Value> = now["equipment"].as_array().unwrap().clone();
    for e in eq.iter_mut() {
        if e["equipment_id"] == "c" {
            e["x"] = json!(999);
        }
    }
    let body = json!({"equipment": eq, "connections": now["connections"], "base_seq": now["seq"]});
    let (s, b) = t
        .call("PUT", "/api/v1/screens/s1/scada-layout", Some(body))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    a.pump(&t).await;
    assert_eq!(a.widget("c").unwrap().x, 999.0);
    assert_eq!(a.widget("a").unwrap().x, 77.0);
    // A write that bypasses the lock (as equipment propagation does) is
    // merged by the background pass.
    let mut eq2 = eq_rows(&t).await;
    eq2.iter_mut().find(|e| e.equipment_id == "b").unwrap().x = 321.0;
    t.state
        .storage
        .save_scada_layout(
            "s1",
            Some(eq2),
            None,
            Default::default(),
            &crate::storage::DesignActor::system("equipment"),
        )
        .await
        .unwrap();
    t.state.collab.tick(&t.state).await;
    a.pump(&t).await;
    assert_eq!(a.widget("b").unwrap().x, 321.0);
    assert_eq!(
        server_rows(&t)
            .widgets
            .iter()
            .find(|w| w.id == "b")
            .unwrap()
            .x,
        321.0
    );
    assert_eq!(
        t.flush().await,
        None,
        "nothing of the session's own to write"
    );
}

async fn eq_rows(t: &T) -> Vec<crate::models::ScadaEquipment> {
    t.state.storage.get_scada_equipment("s1").await.unwrap()
}

#[tokio::test]
async fn revert_and_publish_go_through_the_session() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 5.0)).await;
    // Publishing writes the session's edits first.
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"message": "live"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, published) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    let pa = published["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "a")
        .unwrap()
        .clone();
    assert_eq!(pa["x"], 5.0);
    // A revert of a reaches the editor.
    let seq_before = t
        .history()
        .await
        .iter()
        .filter(|c| c["object_id"] == "a")
        .map(|c| c["seq"].as_i64().unwrap())
        .min()
        .unwrap();
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"object": "a", "to_seq": seq_before - 1})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    a.pump(&t).await;
    assert_eq!(a.widget("a").unwrap().x, 0.0);
}

#[tokio::test]
async fn only_roles_that_configure_join_and_nothing_leaks_to_others() {
    let t = T::new().await;
    t.user("viv", "viewer").await;
    t.user("opa", "operator").await;
    for (conn, user) in [("cv", "viv"), ("co", "opa")] {
        let mut c = Client::new(&t, conn, user);
        c.join(&t).await;
        assert_eq!(c.errors, vec![("forbidden".to_string(), false)]);
        assert!(!c.got_sync && c.epoch.is_none());
        // Sending without joining is refused too.
        c.send(
            &t,
            ClientMessage::DesignSync {
                screen: "s1".into(),
                data: wire::step1(&c.doc),
            },
        )
        .await;
        c.pump(&t).await;
        assert_eq!(c.errors.last().unwrap().0, "forbidden");
        assert!(!c.got_sync);
    }
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 1.0)).await;
    // The viewer's socket got nothing of the session.
    let mut v = Client::new(&t, "cv2", "viv");
    v.pump(&t).await;
    assert!(!v.got_sync && v.presence.is_empty());
    // An older client protocol is refused.
    let mut old = Client::new(&t, "cold", "admin");
    old.send(
        &t,
        ClientMessage::DesignJoin {
            screen: "s1".into(),
            protocol: 99,
            epoch: None,
        },
    )
    .await;
    old.pump(&t).await;
    assert_eq!(old.errors, vec![("protocol".to_string(), false)]);
    // The REST routes follow the configure tier too.
    let viewer = t.user("vic", "viewer").await;
    let (s, _) = t
        .call_as(&viewer, "GET", "/api/v1/design/sessions", None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call_as(&viewer, "GET", "/api/v1/screens/s1/comments", None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn limits_refuse_oversized_and_flooding_clients() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    // Over 256 KB: refused, the client must reload its document.
    let big = a.edit_offline(|txn| {
        change(txn, "a", |w| {
            w.config.insert(
                "text".into(),
                json!("x".repeat(crate::collab::MAX_UPDATE + 10)),
            );
        })
    });
    assert!(big.len() > crate::collab::MAX_UPDATE);
    a.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: big,
        },
    )
    .await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("too_large".to_string(), true));
    assert!(
        server_rows(&t)
            .widgets
            .iter()
            .all(|w| !w.config.contains("xxxx"))
    );
    // It is out of the session until it joins again.
    a.send(
        &t,
        ClientMessage::DesignFlush {
            screen: "s1".into(),
        },
    )
    .await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap().0, "not_joined");

    // More than 50 messages a second.
    let mut b = Client::new(&t, "cb", "admin");
    b.join(&t).await;
    for i in 0..60 {
        b.edit(&t, |txn| change(txn, "c", |w| w.x = f64::from(i)))
            .await;
    }
    b.pump(&t).await;
    assert!(
        b.errors.iter().any(|e| e.0 == "rate_limited"),
        "{:?}",
        b.errors
    );

    // A malformed message.
    let mut c = Client::new(&t, "cc", "admin");
    c.join(&t).await;
    c.send(
        &t,
        ClientMessage::DesignSync {
            screen: "s1".into(),
            data: vec![2, 5, 1, 2],
        },
    )
    .await;
    c.pump(&t).await;
    assert_eq!(c.errors.last().unwrap(), &("bad_message".to_string(), true));
}

fn presence(client: u64, clock: u32, json: Value) -> Vec<u8> {
    wire::encode_awareness(&[Presence {
        client,
        clock,
        json: json.to_string(),
    }])
}

#[tokio::test]
async fn presence_is_stamped_with_the_user_capped_and_removed_on_leave() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    // A claims to be someone else: the server names A.
    let data = presence(
        11,
        1,
        json!({"user": {"name": "mallory", "color": "#000"}, "cursor": {"x": 5, "y": 6}}),
    );
    a.send(
        &t,
        ClientMessage::DesignAwareness {
            screen: "s1".into(),
            data,
        },
    )
    .await;
    // A newcomer gets the presence of those already there.
    b.join(&t).await;
    let state: Value = serde_json::from_str(&b.presence[&11]).unwrap();
    assert_eq!(state["user"]["name"], "admin");
    assert_eq!(state["user"]["color"], crate::collab::colour_of("admin"));
    assert_eq!(state["cursor"]["x"], 5);
    // Oversized states are dropped.
    let data = presence(
        11,
        2,
        json!({"cursor": "x".repeat(crate::collab::MAX_AWARENESS)}),
    );
    a.send(
        &t,
        ClientMessage::DesignAwareness {
            screen: "s1".into(),
            data,
        },
    )
    .await;
    b.pump(&t).await;
    assert!(serde_json::from_str::<Value>(&b.presence[&11]).unwrap()["cursor"].is_object());
    // A leaves: B is told at once.
    a.send(
        &t,
        ClientMessage::DesignLeave {
            screen: "s1".into(),
        },
    )
    .await;
    b.pump(&t).await;
    assert!(b.presence.is_empty());
    assert_eq!(b.sessions[0]["users"].as_array().unwrap().len(), 1);
    // And a dropped socket the same.
    a.join(&t).await;
    a.send(
        &t,
        ClientMessage::DesignAwareness {
            screen: "s1".into(),
            data: presence(12, 1, json!({})),
        },
    )
    .await;
    b.pump(&t).await;
    assert_eq!(b.presence.len(), 1);
    t.state.collab.disconnect(&t.state, "ca");
    b.pump(&t).await;
    assert!(b.presence.is_empty());
}

#[tokio::test]
async fn the_last_leave_writes_and_a_restart_rebuilds_from_the_rows() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    let epoch = a.epoch.clone().unwrap();
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 66.0)).await;
    a.send(
        &t,
        ClientMessage::DesignLeave {
            screen: "s1".into(),
        },
    )
    .await;
    let mut written = false;
    for _ in 0..100 {
        if t.rows().await["a"].0 == 66.0 {
            written = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(written, "written when the last one left");

    // The server restarts: no sessions, the document is rebuilt from rows.
    let mut state = t.state.clone();
    state.collab = Default::default();
    let t2 = T {
        app: create_router(state.clone()),
        state,
        admin: t.admin.clone(),
    };
    let mut back = Client::new(&t2, "ca2", "admin");
    back.doc = std::mem::replace(&mut a.doc, Doc::new());
    back.epoch = Some(epoch.clone());
    back.join(&t2).await;
    assert!(back.resync, "a new epoch: start from the server's document");
    assert_ne!(back.epoch.as_deref(), Some(epoch.as_str()));
    assert_eq!(back.widget("a").unwrap().x, 66.0);
    assert_eq!(
        back.screen(),
        document::read(
            &t2.state
                .collab
                .session("s1")
                .unwrap()
                .doc_for_tests()
                .transact()
        )
    );
}

#[tokio::test]
async fn a_reconnect_with_the_same_epoch_merges_offline_edits() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    // The socket drops; A keeps editing.
    t.state.collab.disconnect(&t.state, "ca");
    let _ = a.edit_offline(|txn| change(txn, "c", |w| w.x = 808.0));
    let mut again = Client::new(&t, "ca-2", "admin");
    again.doc = std::mem::replace(&mut a.doc, Doc::new());
    again.epoch = a.epoch.clone();
    again.join(&t).await;
    assert!(!again.resync);
    assert_eq!(
        server_rows(&t)
            .widgets
            .iter()
            .find(|w| w.id == "c")
            .unwrap()
            .x,
        808.0
    );
}

#[tokio::test]
async fn comments_threads_mentions_and_events() {
    let t = T::new().await;
    let bob = t.user("bob", "demo").await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    let (s, c) = t
        .call(
            "POST",
            "/api/v1/screens/s1/comments",
            Some(
                json!({"body": "Check this pump @bob @nobody", "object_id": "b", "x": 10, "y": 20}),
            ),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    assert_eq!(c["mentions"], json!(["bob"]));
    assert_eq!(c["object_type"], "widget");
    a.pump(&t).await;
    assert_eq!(a.comments[0].0, "created");
    // Bob is notified and replies; the reply joins the thread and its pin.
    let (_, notes) = t
        .call_as(&bob, "GET", "/api/v1/user-notifications", None)
        .await;
    assert!(notes.to_string().contains("mentioned you"), "{notes}");
    let (s, r) = t
        .call_as(
            &bob,
            "POST",
            "/api/v1/screens/s1/comments",
            Some(json!({"body": "Done", "parent_id": c["id"]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        (r["parent_id"].clone(), r["object_id"].clone()),
        (c["id"].clone(), json!("b"))
    );
    // Bob may not edit the admin's text, but may resolve the thread.
    let uri = format!("/api/v1/screens/s1/comments/{}", c["id"]);
    let (s, _) = t
        .call_as(&bob, "PATCH", &uri, Some(json!({"body": "mine now"})))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call_as(&bob, "PATCH", &uri, Some(json!({"resolved": true})))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, open) = t.call("GET", "/api/v1/screens/s1/comments", None).await;
    assert_eq!(open["comments"], json!([]), "a resolved thread is hidden");
    let (_, all) = t
        .call("GET", "/api/v1/screens/s1/comments?resolved=1", None)
        .await;
    assert_eq!(all["comments"].as_array().unwrap().len(), 2);
    assert_eq!(all["comments"][0]["resolved_by"], "bob");
    // Bob may not delete it; the admin may, with its replies.
    let (s, _) = t.call_as(&bob, "DELETE", &uri, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t.call("DELETE", &uri, None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, all) = t
        .call("GET", "/api/v1/screens/s1/comments?resolved=1", None)
        .await;
    assert_eq!(all["comments"], json!([]));
    a.pump(&t).await;
    assert_eq!(
        a.comments.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["created", "created", "updated", "deleted"]
    );
    // Bad input.
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/s1/comments",
            Some(json!({"body": "  "})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_setting_turns_collaboration_off() {
    let t = T::new().await;
    let (_, status) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(status["collaboration"]["enabled"], true);
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 9.0)).await;
    let (_, status) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(status["collaboration"]["editors"][0]["user"], "admin");
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/settings/design-history",
            Some(json!({"collaboration": false})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap().0, "disabled");
    assert_eq!(
        t.rows().await["a"].0,
        9.0,
        "pending edits were written first"
    );
    assert!(t.state.collab.session("s1").is_none());
    let mut b = Client::new(&t, "cb", "admin");
    b.join(&t).await;
    assert_eq!(b.errors, vec![("disabled".to_string(), false)]);
}

#[tokio::test]
async fn idle_sessions_close() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    a.send(
        &t,
        ClientMessage::DesignLeave {
            screen: "s1".into(),
        },
    )
    .await;
    assert!(
        t.state.collab.session("s1").is_some(),
        "kept for a reconnect"
    );
    t.state
        .collab
        .session("s1")
        .unwrap()
        .age_for_tests(crate::collab::IDLE_CLOSE);
    t.state.collab.tick(&t.state).await;
    assert!(t.state.collab.session("s1").is_none());
}

// ── Hardening (review of PR #45) ──

/// The review's stack bomb: one item whose value is `[117, 1]` repeated
/// 120 000 times (an array of an array of …), about 240 KB.
fn stack_bomb() -> Vec<u8> {
    let mut u = vec![1u8, 1, 5, 0, 8 | 0x20, 1, 7];
    u.extend(b"widgets");
    u.extend([1, b'w', 1]);
    for _ in 0..120_000 {
        u.extend([117, 1]);
    }
    u.extend([125, 1, 0]);
    u
}

fn member(t: &T, conn: &str) -> bool {
    t.state
        .collab
        .session("s1")
        .is_some_and(|s| s.member_conns().iter().any(|c| c == conn))
}

#[tokio::test]
async fn malicious_updates_are_refused_before_anyone_sees_them() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    let before = server_rows(&t);
    let seen = b.screen();

    // The stack bomb: under the size limit, refused whole, the server lives.
    let bomb = wire::update(stack_bomb());
    assert!(bomb.len() < crate::collab::MAX_UPDATE);
    a.sync(&t, bomb).await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("refused".to_string(), true));
    assert!(!member(&t, "ca"));
    b.pump(&t).await;
    assert_eq!(b.screen(), seen, "nothing was forwarded");
    assert_eq!(server_rows(&t), before);

    // Shallower but still too deep, and a top-level type of its own.
    for build in [
        |txn: &mut TransactionMut| {
            let mut v = voltrus_collab::yrs::Any::from(1);
            for _ in 0..40 {
                v = voltrus_collab::yrs::Any::Array(vec![v].into());
            }
            let w = txn.get_or_insert_map("widgets");
            voltrus_collab::yrs::Map::insert(&w, txn, "deep", v);
        },
        |txn: &mut TransactionMut| {
            let j = txn.get_or_insert_map("junk");
            voltrus_collab::yrs::Map::insert(&j, txn, "k", "x".repeat(10_000));
        },
    ] {
        let mut m = Client::new(&t, "cm", "admin");
        m.join(&t).await;
        let data = m.edit_offline(build);
        m.sync(&t, data).await;
        m.pump(&t).await;
        assert_eq!(m.errors.last().unwrap(), &("refused".to_string(), true));
        assert!(!member(&t, "cm"));
        b.pump(&t).await;
        assert_eq!(b.screen(), seen);
        t.state.ws_manager.disconnect("cm");
    }
    assert_eq!(server_rows(&t), before);
}

#[tokio::test]
async fn bad_shapes_are_put_back_field_by_field_as_they_arrive() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    // One edit: a valid label, a position off the stage and junk keys.
    a.edit(&t, |txn| {
        change(txn, "a", |w| {
            w.config.insert("text".into(), json!("Fine"));
            w.x = 1e9;
        });
        let ws = txn.get_or_insert_map("widgets");
        if let Some(voltrus_collab::yrs::Out::YMap(m)) =
            voltrus_collab::yrs::Map::get(&ws, txn, "a")
        {
            voltrus_collab::yrs::Map::insert(&m, txn, "padding", "x".repeat(100_000));
        }
    })
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(a.rejected.len(), 1, "{:?}", a.rejected);
    assert!(a.rejected[0].1.contains("put back: x"), "{:?}", a.rejected);
    for c in [&a, &b] {
        let w = c.widget("a").unwrap();
        assert_eq!((w.x, w.config["text"].as_str()), (0.0, Some("Fine")));
    }
    assert_eq!(a.screen(), b.screen());
    // The junk key is gone everywhere, and the size is measured again.
    let doc = t.state.collab.session("s1").unwrap().doc_for_tests();
    {
        let txn = doc.transact();
        let ws = txn.get_map("widgets").unwrap();
        let Some(voltrus_collab::yrs::Out::YMap(m)) = voltrus_collab::yrs::Map::get(&ws, &txn, "a")
        else {
            panic!("a is a map")
        };
        assert!(voltrus_collab::yrs::Map::get(&m, &txn, "padding").is_none());
    }
    t.state.collab.tick(&t.state).await;
    let size = t.state.collab.session("s1").unwrap().doc_size_for_tests();
    assert!(size < 20_000, "measured again: {size}");
    // The valid part is written.
    t.flush().await;
    let rows = t.rows().await;
    assert_eq!(
        (rows["a"].0, rows["a"].1["text"].as_str()),
        (0.0, Some("Fine"))
    );

    // A stream of bad edits sends its author away.
    for i in 0..crate::collab::MAX_STRIKES + 1 {
        if !member(&t, "ca") {
            break;
        }
        a.edit(&t, |txn| change(txn, "c", |w| w.width = -1.0 - i as f64))
            .await;
    }
    a.pump(&t).await;
    assert!(
        a.errors.contains(&("invalid_edits".to_string(), true)),
        "{:?}",
        a.errors
    );
    assert!(!member(&t, "ca") && member(&t, "cb"));
}

#[tokio::test]
async fn a_write_puts_back_only_the_failing_fields_and_tells_everyone_who_touched_them() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    a.edit(&t, |txn| {
        change(txn, "a", |w| {
            w.config.insert("text".into(), json!("Kept"));
        })
    })
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    b.edit(&t, |txn| {
        change(txn, "a", |w| bind(w, "visible", "nope/missing"))
    })
    .await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    let f = t.flush().await.unwrap();
    assert_eq!(f.rejected.len(), 1);
    sync_all(&t, &mut [&mut a, &mut b]).await;
    assert_eq!(
        a.rejected.len(),
        1,
        "an earlier author of the object is told too"
    );
    assert_eq!(b.rejected.len(), 1);
    assert!(
        b.rejected[0]
            .1
            .contains("put back: config.bindings.visible"),
        "{:?}",
        b.rejected
    );
    for c in [&a, &b] {
        let w = c.widget("a").unwrap();
        assert_eq!(w.config["text"], "Kept");
        assert!(w.config.get("bindings").is_none());
    }
    // The valid change is written at the next write.
    t.flush().await;
    assert_eq!(t.rows().await["a"].1["text"], "Kept");
}

#[tokio::test]
async fn a_downgraded_or_revoked_editor_is_sent_away_at_its_next_message() {
    let t = T::new().await;
    t.user("dan", "demo").await;
    t.user("eve", "admin").await;
    let mut d = Client::new(&t, "cd", "dan");
    let mut e = Client::new(&t, "ce", "eve");
    d.join(&t).await;
    e.join(&t).await;
    d.edit(&t, |txn| change(txn, "a", |w| w.x = 10.0)).await;
    assert_eq!(server_rows(&t).widgets[0].x, 10.0);
    // Dan becomes a viewer: his next edit is refused and he leaves.
    let dan = t
        .state
        .storage
        .get_user_by_username("dan")
        .await
        .unwrap()
        .unwrap();
    let generation = *crate::storage::ACCESS.borrow();
    t.state
        .storage
        .update_user_role(dan.id, "viewer")
        .await
        .unwrap();
    assert_ne!(
        *crate::storage::ACCESS.borrow(),
        generation,
        "open sockets are told to check again"
    );
    let data = d.edit_offline(|txn| change(txn, "a", |w| w.x = 20.0));
    d.sync(&t, data).await;
    d.pump(&t).await;
    assert_eq!(d.errors.last().unwrap(), &("forbidden".to_string(), false));
    assert!(!member(&t, "cd"));
    assert_eq!(server_rows(&t).widgets[0].x, 10.0, "not applied");
    // Flushing and presence are refused the same way.
    d.send(
        &t,
        ClientMessage::DesignFlush {
            screen: "s1".into(),
        },
    )
    .await;
    d.pump(&t).await;
    assert_eq!(d.errors.last().unwrap().0, "forbidden");
    // Eve is disabled: her socket closes at her next message.
    let eve = t
        .state
        .storage
        .get_user_by_username("eve")
        .await
        .unwrap()
        .unwrap();
    t.state
        .storage
        .set_user_disabled(eve.id, true)
        .await
        .unwrap();
    let data = e.edit_offline(|txn| change(txn, "a", |w| w.x = 30.0));
    let after = e
        .send_after(
            &t,
            ClientMessage::DesignSync {
                screen: "s1".into(),
                data,
            },
        )
        .await;
    assert_eq!(after, After::Close);
    assert!(!member(&t, "ce"));
    assert_eq!(server_rows(&t).widgets[0].x, 10.0);
}

#[tokio::test]
async fn dropped_members_join_again_and_their_edits_arrive() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    // Flooding: dropped, the last edits not applied.
    for i in 0..crate::collab::MAX_RATE + 5 {
        let data = a.edit_offline(|txn| change(txn, "c", |w| w.x = 1000.0 + f64::from(i)));
        a.send(
            &t,
            ClientMessage::DesignSync {
                screen: "s1".into(),
                data,
            },
        )
        .await;
    }
    a.pump(&t).await;
    assert!(a.errors.contains(&("rate_limited".to_string(), false)));
    assert!(!member(&t, "ca"));
    let last = a.widget("c").unwrap().x;
    assert_ne!(server_rows(&t).widgets[2].x, last);
    // Joining again with its epoch: the sync exchange brings its edits.
    a.join(&t).await;
    assert!(!a.resync);
    assert_eq!(server_rows(&t).widgets[2].x, last);
    // Messages after leaving: not_joined; joining again brings them too.
    a.send(
        &t,
        ClientMessage::DesignLeave {
            screen: "s1".into(),
        },
    )
    .await;
    let data = a.edit_offline(|txn| change(txn, "c", |w| w.x = 4242.0));
    a.sync(&t, data).await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("not_joined".to_string(), false));
    a.join(&t).await;
    assert_eq!(server_rows(&t).widgets[2].x, 4242.0);
    // A member heard from for too long leaves; it joins again the same way.
    t.state
        .collab
        .session("s1")
        .unwrap()
        .idle_for_tests("ca", crate::collab::IDLE_MEMBER);
    t.state.collab.tick(&t.state).await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("idle".to_string(), false));
    assert!(!member(&t, "ca"));
    a.join(&t).await;
    assert!(member(&t, "ca"));
}

#[tokio::test]
async fn writes_acknowledge_each_connections_messages() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    // The join: our step 1 and our step 2 reply.
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 1.0)).await;
    a.edit(&t, |txn| change(txn, "a", |w| w.y = 2.0)).await;
    let f = t.flush().await.unwrap();
    a.pump(&t).await;
    let (users, acked) = a.acks.last().unwrap().clone();
    assert_eq!(users, vec!["admin".to_string()]);
    assert_eq!(acked, Some(4));
    assert_eq!(a.flushed, vec![f.seq]);
    // A message that changes nothing is acknowledged without a write.
    let same = wire::update(
        a.doc
            .transact()
            .encode_state_as_update_v1(&voltrus_collab::yrs::StateVector::default()),
    );
    a.sync(&t, same).await;
    t.state.collab.tick(&t.state).await;
    a.pump(&t).await;
    assert_eq!(a.acks.last().unwrap(), &(vec![], Some(5)));
    assert_eq!(a.flushed.len(), 1, "no write");
}

#[tokio::test]
async fn shutdown_writes_what_the_session_holds_and_takes_nothing_more() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 5.0)).await;
    t.state.collab.shutdown(&t.state).await;
    assert_eq!(t.rows().await["a"].0, 5.0);
    let data = a.edit_offline(|txn| change(txn, "a", |w| w.x = 6.0));
    a.sync(&t, data).await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("shutdown".to_string(), false));
    assert_eq!(server_rows(&t).widgets[0].x, 5.0, "kept by the client");
}

#[tokio::test]
async fn a_new_stacking_order_is_logged_under_who_reordered() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    // Admin brings a to the front (every row's z is renumbered); bob moves c.
    a.edit(&t, |txn| {
        let s = document::read(txn);
        let top = s.widgets["c"].1.clone();
        let key = voltrus_collab::fractional::key_between(Some(&top), None);
        let w = s.widgets["a"].0.clone();
        document::put_widget(txn, "a", Some(&w), Some(&key));
    })
    .await;
    b.edit(&t, |txn| change(txn, "c", |w| w.y = 999.0)).await;
    sync_all(&t, &mut [&mut a, &mut b]).await;
    let f = t.flush().await.unwrap();
    assert_eq!(f.users, vec!["admin".to_string(), "bob".to_string()]);
    let rows = t.rows().await;
    assert_eq!(
        (
            rows["b"].1["z"].clone(),
            rows["c"].1["z"].clone(),
            rows["a"].1["z"].clone()
        ),
        (json!(0), json!(1), json!(2))
    );
    let who: BTreeMap<String, String> = t
        .history()
        .await
        .iter()
        .take(3)
        .map(|c| {
            (
                c["object_id"].as_str().unwrap().to_string(),
                c["username"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(who["a"], "admin");
    assert_eq!(who["b"], "admin", "renumbered by admin's reorder");
    assert_eq!(who["c"], "bob");
}

#[tokio::test]
async fn versions_reads_and_publish_see_the_live_edits() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    let (_, status) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    let (_, diff) = t
        .call("GET", "/api/v1/screens/s1/diff?yaml=false", None)
        .await;
    let shown = diff["to"]["seq"].as_i64().unwrap();
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 77.0)).await;
    // Reads write the session first.
    let (s, st) = t
        .call("GET", "/api/v1/screens/s1/state?at=draft", None)
        .await;
    assert_eq!(s, StatusCode::OK);
    let a_row = st["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "a")
        .unwrap()
        .clone();
    assert_eq!(a_row["x"], 77.0);
    let (_, blame) = t.call("GET", "/api/v1/screens/s1/blame", None).await;
    assert!(blame.to_string().contains("designer"), "{blame}");
    // A version holds the edit.
    a.edit(&t, |txn| change(txn, "a", |w| w.x = 78.0)).await;
    let (s, v) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "live"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, got) = t
        .call("GET", "/api/v1/screens/s1/versions/live", None)
        .await;
    let x = got["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "a")
        .unwrap()["x"]
        .clone();
    assert_eq!(x, 78.0);
    // Publishing against the summary shown before the edits: the draft moved.
    let _ = status;
    let (s, body) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"base_seq": shown, "message": "go"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{body}");
}

#[tokio::test]
async fn mentions_notify_once_and_only_who_can_open_the_comment() {
    let t = T::new().await;
    t.user("bob", "demo").await;
    t.user("vic", "viewer").await;
    let notes = |t: &T, user: &'static str| {
        let storage = t.state.storage.clone();
        async move {
            storage
                .design_tx(move |c| {
                    Ok(c.query_row(
                        "SELECT COUNT(*) FROM user_notifications n JOIN users u ON u.id = n.user_id
                         WHERE u.username = ?1 AND n.notification_type = 'mention'",
                        [user],
                        |r| r.get::<_, i64>(0),
                    )?)
                })
                .await
                .unwrap()
        }
    };
    let (s, c) = t
        .call(
            "POST",
            "/api/v1/screens/s1/comments",
            Some(json!({"body": "@bob @vic look"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{c}");
    let id = c["id"].as_i64().unwrap();
    assert_eq!((notes(&t, "bob").await, notes(&t, "vic").await), (1, 0));
    // Edits that drop and bring back the name notify nobody again.
    for body in ["look", "@bob look again", "@bob @bob once more"] {
        let (s, _) = t
            .call(
                "PATCH",
                &format!("/api/v1/screens/s1/comments/{id}"),
                Some(json!({ "body": body })),
            )
            .await;
        assert_eq!(s, StatusCode::OK);
    }
    assert_eq!(notes(&t, "bob").await, 1);
    // A screen that does not exist.
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/nope/comments",
            Some(json!({"body": "x"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    // Too many comments a minute.
    let mut last = StatusCode::OK;
    for i in 0..crate::storage::design_comments::MAX_COMMENTS_PER_WINDOW {
        last = t
            .call(
                "POST",
                "/api/v1/screens/s1/comments",
                Some(json!({"body": format!("n{i}")})),
            )
            .await
            .0;
        if last != StatusCode::OK {
            break;
        }
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn presence_belongs_to_the_connection_that_set_it() {
    let t = T::new().await;
    t.user("bob", "admin").await;
    let mut a = Client::new(&t, "ca", "admin");
    let mut b = Client::new(&t, "cb", "bob");
    a.join(&t).await;
    b.join(&t).await;
    a.send(
        &t,
        ClientMessage::DesignAwareness {
            screen: "s1".into(),
            data: presence(21, 1, json!({"cursor": {"x": 1, "y": 2}})),
        },
    )
    .await;
    b.pump(&t).await;
    assert!(b.presence[&21].contains("admin"));
    // Bob can neither take over nor clear A's client.
    for json in [json!({"cursor": {"x": 9, "y": 9}}), Value::Null] {
        let data = wire::encode_awareness(&[Presence {
            client: 21,
            clock: 5,
            json: json.to_string(),
        }]);
        b.send(
            &t,
            ClientMessage::DesignAwareness {
                screen: "s1".into(),
                data,
            },
        )
        .await;
    }
    a.pump(&t).await;
    assert!(
        a.presence.get(&21).is_none(),
        "nothing was forwarded about 21"
    );
    let mut c = Client::new(&t, "cc", "admin");
    c.join(&t).await;
    assert!(c.presence[&21].contains("\"x\":1"), "{:?}", c.presence);
    // Updates over the rate are dropped.
    for i in 0..crate::collab::MAX_AWARENESS_RATE as u32 + 10 {
        a.send(
            &t,
            ClientMessage::DesignAwareness {
                screen: "s1".into(),
                data: presence(21, 10 + i, json!({"cursor": {"x": i, "y": 0}})),
            },
        )
        .await;
    }
    b.presence.clear();
    let mut n = 0;
    while let Ok(f) = b.rx.try_recv() {
        if let WsFrame::Message(ServerMessage::DesignAwareness { .. }) = f {
            n += 1;
        }
    }
    assert!(n <= crate::collab::MAX_AWARENESS_RATE, "{n}");
}

#[tokio::test]
async fn sessions_need_an_existing_screen_and_are_capped_per_connection() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    let join = |screen: &str| ClientMessage::DesignJoin {
        screen: screen.into(),
        protocol: voltrus_collab::PROTOCOL,
        epoch: None,
    };
    a.send(&t, join("nope")).await;
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("not_found".to_string(), false));
    assert!(t.state.collab.session("nope").is_none());
    for i in 0..crate::collab::MAX_SESSIONS_PER_CONN + 1 {
        let id = format!("x{i}");
        t.state
            .storage
            .create_device(&id, &id, 10 + i as i64)
            .await
            .unwrap();
        a.send(&t, join(&id)).await;
    }
    a.pump(&t).await;
    assert!(
        a.errors.contains(&("too_many_sessions".to_string(), false)),
        "{:?}",
        a.errors
    );
    let last = format!("x{}", crate::collab::MAX_SESSIONS_PER_CONN);
    assert!(t.state.collab.session(&last).is_none());
}

#[tokio::test]
async fn deleting_a_screen_ends_its_session_and_its_comments() {
    let t = T::new().await;
    let mut a = Client::new(&t, "ca", "admin");
    a.join(&t).await;
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/s1/comments",
            Some(json!({"body": "x"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = t.call("DELETE", "/api/v1/screens/s1", None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    a.pump(&t).await;
    assert_eq!(a.errors.last().unwrap(), &("deleted".to_string(), false));
    assert!(t.state.collab.session("s1").is_none());
    let left: i64 = t
        .state
        .storage
        .design_tx(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM design_comments WHERE screen_id = 's1'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(left, 0);
}
