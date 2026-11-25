//! Designer version history (docs/version-history.md): the change log on
//! every write path, squashing and retention, versions, diff, revert with its
//! dangling-connection rules, draft and published with the approval step,
//! permissions, export and import options, and git sync to a bare repository.

use super::*;
use http::StatusCode;
use serde_json::{Value, json};

struct T {
    app: axum::Router,
    state: AppState,
    admin: String,
}

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
    - {equipment_id: b, equipment_type: symbol, x: 200, sensors: [plc_1/p1_run], config: {symbol: pump, z: 1, runSensorId: plc_1/p1_run}}
    connections:
    - {connection_id: c1, from_equipment: a, to_equipment: b, connection_type: flow, waypoints: []}
"#;

fn eq(id: &str, x: f64, config: Value) -> Value {
    json!({"equipment_id": id, "equipment_type": "label", "equipment_name": "", "x": x, "y": 0,
           "width": 100, "height": 40, "rotation": 0, "sensor_bindings": "[]", "config": config.to_string()})
}

fn conn(id: &str, from: &str, to: &str) -> Value {
    json!({"connection_id": id, "from_equipment": from, "to_equipment": to, "connection_type": "flow", "waypoints": "[]"})
}

impl T {
    async fn new() -> Self {
        let state = seeded_admin_app_state().await;
        let app = create_router(state.clone());
        let admin = format!("voltrus_token={}", admin_cookie(&app).await);
        T { app, state, admin }
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

    async fn settings(&self, v: Value) {
        let (s, b) = self
            .call("PUT", "/api/v1/settings/design-history", Some(v))
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
    }

    async fn import(&self, yaml: &str, query: &str) -> Value {
        let req = http::Request::builder()
            .method("POST")
            .uri(format!("/api/v1/project/import{query}"))
            .header("cookie", &self.admin)
            .header("content-type", "application/yaml")
            .body(axum::body::Body::from(yaml.to_string()))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(self.app.clone(), req)
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn history(&self, screen: &str, query: &str) -> Vec<Value> {
        let (s, b) = self
            .call(
                "GET",
                &format!("/api/v1/screens/{screen}/history{query}"),
                None,
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b["changes"].as_array().unwrap().clone()
    }

    async fn layout(&self, screen: &str) -> Value {
        self.call(
            "GET",
            &format!("/api/v1/screens/{screen}/scada-layout"),
            None,
        )
        .await
        .1
    }

    async fn put_layout(
        &self,
        screen: &str,
        equipment: Vec<Value>,
        connections: Vec<Value>,
    ) -> Value {
        let (s, b) = self
            .call(
                "PUT",
                &format!("/api/v1/screens/{screen}/scada-layout"),
                Some(json!({"equipment": equipment, "connections": connections})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        b
    }

    async fn ids(&self, screen: &str, query: &str) -> (Vec<String>, Vec<String>) {
        let b = self
            .call(
                "GET",
                &format!("/api/v1/screens/{screen}/scada-layout{query}"),
                None,
            )
            .await
            .1;
        let pick = |k: &str, id: &str| -> Vec<String> {
            b[k].as_array()
                .unwrap()
                .iter()
                .map(|r| r[id].as_str().unwrap().to_string())
                .collect()
        };
        (
            pick("equipment", "equipment_id"),
            pick("connections", "connection_id"),
        )
    }
}

fn kinds(changes: &[Value]) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = changes
        .iter()
        .map(|c| {
            (
                c["object_type"].as_str().unwrap().to_string(),
                c["object_id"].as_str().unwrap().to_string(),
                c["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn every_save_is_one_batch_with_who_and_what() {
    let t = T::new().await;
    t.settings(json!({"squash_window_secs": 0})).await;
    let saved = t
        .put_layout(
            "s1",
            vec![eq("a", 0.0, json!({})), eq("b", 10.0, json!({}))],
            vec![conn("c1", "a", "b")],
        )
        .await;
    let seq1 = saved["seq"].as_i64().unwrap();
    assert!(seq1 > 0);
    assert!(saved["batch"].is_string());
    let h = t.history("s1", "").await;
    assert_eq!(
        kinds(&h),
        vec![
            ("connection".into(), "c1".into(), "add".into()),
            ("widget".into(), "a".into(), "add".into()),
            ("widget".into(), "b".into(), "add".into()),
        ]
    );
    assert!(h.iter().all(|c| c["batch"] == saved["batch"]));
    assert_eq!(h[0]["username"], "admin");
    assert_eq!(h[0]["source"], "user");
    assert_eq!(h[0]["operation"], "save");
    assert!(
        h.iter()
            .all(|c| c["before"].is_null() && c["after"].is_object())
    );

    // Saving the same rows logs nothing; the seq stays.
    let again = t
        .put_layout(
            "s1",
            vec![eq("a", 0.0, json!({})), eq("b", 10.0, json!({}))],
            vec![conn("c1", "a", "b")],
        )
        .await;
    assert_eq!(again["seq"], seq1);
    assert!(again["batch"].is_null());

    // A change of one widget is one row with before and after.
    let s2 = t
        .put_layout(
            "s1",
            vec![eq("a", 50.0, json!({})), eq("b", 10.0, json!({}))],
            vec![conn("c1", "a", "b")],
        )
        .await;
    let h = t.history("s1", "?object=a").await;
    assert_eq!(h.len(), 2);
    assert_eq!(h[0]["kind"], "update");
    assert_eq!(h[0]["before"]["x"], json!(0.0));
    assert_eq!(h[0]["after"]["x"], json!(50.0));
    assert_eq!(h[0]["seq"], s2["seq"]);

    // The Designer names itself; the per-table route (old editor) is logged too.
    let req = http::Request::builder()
        .method("PUT")
        .uri("/api/v1/screens/s1/scada-equipment")
        .header("cookie", &t.admin)
        .header("content-type", "application/json")
        .header("x-voltrus-client", "designer")
        .body(axum::body::Body::from(
            json!({"items": [eq("a", 50.0, json!({})), eq("b", 99.0, json!({}))]}).to_string(),
        ))
        .unwrap();
    let resp = tower::ServiceExt::oneshot(t.app.clone(), req)
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let h = t.history("s1", "?object=b&limit=1").await;
    assert_eq!(h[0]["source"], "designer");
    assert_eq!(h[0]["after"]["x"], json!(99.0));

    // Blame: the last change per object, with who added it.
    let (_, blame) = t.call("GET", "/api/v1/screens/s1/blame", None).await;
    let b = blame["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["object_id"] == "b")
        .unwrap()
        .clone();
    assert_eq!(b["kind"], "update");
    assert_eq!(b["source"], "designer");
    assert_eq!(b["created_by"], "admin");
    assert_eq!(b["changes"], 2);

    // Filters by user and paging.
    assert!(t.history("s1", "?user=nobody").await.is_empty());
    let (_, page) = t
        .call("GET", "/api/v1/screens/s1/history?limit=2", None)
        .await;
    let next = page["next_before_seq"].as_i64().unwrap();
    let (_, rest) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s1/history?before_seq={next}"),
            None,
        )
        .await;
    assert_eq!(rest["changes"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn a_stale_base_seq_is_a_409_naming_who_changed_what() {
    let t = T::new().await;
    let first = t
        .put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    let base = first["seq"].as_i64().unwrap();
    let other = t.user("eve", "admin").await;
    let (s, b) = t
        .call_as(
            &other,
            "PUT",
            "/api/v1/screens/s1/scada-layout",
            Some(json!({"equipment": [eq("a", 1.0, json!({})), eq("x", 2.0, json!({}))], "connections": [], "base_seq": base})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, conflict) = t
        .call(
            "PUT",
            "/api/v1/screens/s1/scada-layout",
            Some(json!({"equipment": [eq("a", 5.0, json!({}))], "connections": [], "base_seq": base})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["seq"], b["seq"]);
    let since = conflict["changes_since"].as_array().unwrap();
    assert_eq!(since.len(), 1);
    assert_eq!(since[0]["username"], "eve");
    let objs: Vec<&str> = since[0]["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["object_id"].as_str().unwrap())
        .collect();
    assert_eq!(objs, ["a", "x"]);
    // Nothing was written; the revision check still works next to it.
    assert_eq!(
        t.layout("s1").await["equipment"].as_array().unwrap().len(),
        2
    );
    let (s, _) = t
        .call(
            "PUT",
            "/api/v1/screens/s1/scada-layout",
            Some(json!({"equipment": [], "connections": [], "base_seq": b["seq"], "base_revision": "stale"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = t
        .call(
            "PUT",
            "/api/v1/screens/s1/scada-layout",
            Some(json!({"equipment": [], "connections": [], "base_seq": -1})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn saves_within_the_window_by_one_author_squash() {
    let t = T::new().await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    t.put_layout("s1", vec![eq("a", 5.0, json!({}))], vec![])
        .await;
    t.put_layout("s1", vec![eq("a", 9.0, json!({}))], vec![])
        .await;
    let h = t.history("s1", "").await;
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0]["kind"], "add");
    assert_eq!(h[0]["after"]["x"], json!(9.0));
    assert_eq!(h[0]["squashed"], 2);
    // Another author is a row of its own; a removal is never squashed into
    // the add before it (both stay states one can go back to).
    let eve = t.user("eve", "admin").await;
    t.call_as(&eve, "PUT", "/api/v1/screens/s1/scada-layout",
        Some(json!({"equipment": [eq("a", 9.0, json!({})), eq("tmp", 0.0, json!({}))], "connections": []}))).await;
    t.call_as(
        &eve,
        "PUT",
        "/api/v1/screens/s1/scada-layout",
        Some(json!({"equipment": [eq("a", 9.0, json!({}))], "connections": []})),
    )
    .await;
    let h = t.history("s1", "").await;
    assert_eq!(kinds(&h[..2]).len(), 2, "{h:?}");
    assert_eq!(h.len(), 3, "{h:?}");
    assert_eq!(
        (h[0]["kind"].as_str(), h[1]["kind"].as_str()),
        (Some("remove"), Some("add"))
    );
    // A version pins its rows: the next save is a row of its own.
    t.call(
        "POST",
        "/api/v1/screens/s1/versions",
        Some(json!({"name": "pin"})),
    )
    .await;
    t.put_layout("s1", vec![eq("a", 11.0, json!({}))], vec![])
        .await;
    assert_eq!(t.history("s1", "").await.len(), 4);
}

#[tokio::test]
async fn retention_squashes_old_rows_per_hour_and_prunes() {
    let t = T::new().await;
    t.settings(json!({"squash_window_secs": 0, "squash_after_days": 30, "keep_days": 0}))
        .await;
    for x in [1.0, 2.0, 3.0, 4.0] {
        t.put_layout("s1", vec![eq("a", x, json!({}))], vec![])
            .await;
    }
    t.call(
        "POST",
        "/api/v1/screens/s1/versions",
        Some(json!({"name": "v-mid"})),
    )
    .await;
    for x in [5.0, 6.0] {
        t.put_layout("s1", vec![eq("a", x, json!({}))], vec![])
            .await;
    }
    assert_eq!(t.history("s1", "").await.len(), 6);
    // Age everything by 40 days, all within one hour.
    let old = crate::timeutil::now_millis() - 40 * 86_400_000;
    t.state
        .storage
        .with_conn(move |c| {
            c.execute("UPDATE design_changes SET at_ms = ?1", [old])?;
            Ok(())
        })
        .await
        .unwrap();
    let report = t.state.storage.design_maintain().await.unwrap();
    // 1..4 become one row (the version pins the boundary), 5..6 another.
    assert_eq!(report.squashed, 4, "{report:?}");
    let h = t.history("s1", "").await;
    assert_eq!(h.len(), 2, "{h:?}");
    assert_eq!(h[1]["kind"], "add");
    assert_eq!(h[1]["after"]["x"], json!(4.0));
    assert_eq!(h[0]["before"]["x"], json!(4.0));
    assert_eq!(h[0]["after"]["x"], json!(6.0));
    // The version still restores exactly.
    let (_, v) = t
        .call("GET", "/api/v1/screens/s1/versions/v-mid", None)
        .await;
    assert_eq!(v["equipment"][0]["x"], json!(4.0));
    // keep_days prunes old rows but not the one a version points at.
    t.settings(json!({"keep_days": 10})).await;
    let report = t.state.storage.design_maintain().await.unwrap();
    assert_eq!(report.deleted, 1, "{report:?}");
    assert_eq!(t.history("s1", "").await.len(), 1);
}

#[tokio::test]
async fn import_equipment_faceplates_rollback_and_renames_are_logged() {
    let t = T::new().await;
    let plan = t.import(PROJECT, "").await;
    let batch = plan["design_batch"].as_str().unwrap().to_string();
    let h = t.history("s1", "").await;
    assert_eq!(h.len(), 3);
    assert!(
        h.iter()
            .all(|c| c["batch"] == batch && c["operation"] == "import")
    );

    // Equipment types through the library (the import engine), faceplates on
    // their own route: project objects, screen "".
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/equipment-types/library/install",
            Some(json!({"ids": ["pump"]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, proj) = t
        .call(
            "GET",
            "/api/v1/design/history?screen=&object_type=equipment_type",
            None,
        )
        .await;
    let c = &proj["changes"][0];
    assert_eq!(c["object_id"], "pump");
    assert_eq!(c["kind"], "add");
    assert_eq!(c["operation"], "equipment");
    let (_, proj) = t
        .call(
            "GET",
            "/api/v1/design/history?screen=&object_type=faceplate&object=pump",
            None,
        )
        .await;
    assert_eq!(proj["changes"][0]["kind"], "add", "{proj}");
    let (s, _) = t
        .call("PUT", "/api/v1/faceplates/pump", Some(json!({"id": "pump", "name": "Pump (renamed)", "kind": "pump", "roles": [{"role": "run", "kind": "state"}]})))
        .await;
    assert_eq!(s, StatusCode::OK);
    let (_, proj) = t
        .call(
            "GET",
            "/api/v1/design/history?screen=&object_type=faceplate&object=pump",
            None,
        )
        .await;
    let latest = &proj["changes"][0];
    assert_eq!(latest["kind"], "update");
    assert_eq!(latest["operation"], "faceplate");
    assert_eq!(latest["after"]["name"], "Pump (renamed)");
    // Revert the faceplate to its first state.
    let to_seq = proj["changes"][1]["seq"].as_i64().unwrap();
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/design/revert",
            Some(json!({"object_type": "faceplate", "object_id": "pump", "to_seq": to_seq})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, fp) = t.call("GET", "/api/v1/faceplates/pump", None).await;
    assert_ne!(fp["name"], "Pump (renamed)");

    // A tag rename rewrites the draft (logged) and the published copy (not).
    t.call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    let mut layout = t.layout("s1").await;
    let mut b_row = layout["equipment"][1].clone();
    let mut config: Value = serde_json::from_str(b_row["config"].as_str().unwrap()).unwrap();
    config["bindings"] = json!({"label": {"kind": "tag", "tag": "plc_1/level"}});
    b_row["config"] = json!(config.to_string());
    layout["equipment"][1] = b_row;
    t.put_layout(
        "s1",
        layout["equipment"].as_array().unwrap().clone(),
        layout["connections"].as_array().unwrap().clone(),
    )
    .await;
    t.call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    let level = t
        .state
        .storage
        .list_data_sources(None)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.key == "level")
        .unwrap();
    let (s, b) = t
        .call(
            "PUT",
            &format!("/api/v1/data-sources/{}", level.id),
            Some(json!({"key": "tank_level"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let h = t.history("s1", "?object=b&limit=1").await;
    assert_eq!(h[0]["operation"], "tag_rename");
    assert!(
        h[0]["after"]["config"]
            .as_str()
            .unwrap()
            .contains("plc_1/tank_level")
    );
    let (_, published) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    assert_eq!(published["view"], "published");
    assert!(
        published["equipment"][1]["config"]
            .as_str()
            .unwrap()
            .contains("plc_1/tank_level")
    );

    // A snapshot rollback rewrites screens: logged as one batch.
    let (_, snaps) = t.call("GET", "/api/v1/config/snapshots", None).await;
    let pre_import = snaps
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["label"] == "pre-import")
        .map(|s| s["id"].as_i64().unwrap())
        .min()
        .unwrap();
    let (s, b) = t
        .call(
            "POST",
            &format!("/api/v1/config/snapshots/{pre_import}/rollback"),
            None,
        )
        .await;
    assert!(s.is_success(), "{s} {b}");
    let h = t.history("s1", "").await;
    let rb: Vec<&Value> = h.iter().filter(|c| c["operation"] == "rollback").collect();
    assert_eq!(rb.len(), 3, "{h:?}");
    assert!(
        rb.iter()
            .all(|c| c["kind"] == "remove" && c["username"] == "admin")
    );
}

#[tokio::test]
async fn versions_and_diff() {
    let t = T::new().await;
    t.import(PROJECT, "").await;
    let (s, v1) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"message": "first cut"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v1}");
    assert_eq!(v1["version"]["name"], "v1");
    assert_eq!(v1["version"]["widgets"], 2);
    let (s, dup) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "v1"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(dup["code"], "name_taken");

    let mut layout = t.layout("s1").await;
    layout["equipment"][0]["x"] = json!(77.0);
    let mut eqs = layout["equipment"].as_array().unwrap().clone();
    eqs.push(eq("c", 300.0, json!({"z": 2})));
    t.put_layout("s1", eqs, vec![]).await;
    let (s, v2) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "moved", "message": "moved a"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (_, list) = t.call("GET", "/api/v1/screens/s1/versions", None).await;
    let names: Vec<&str> = list["versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["moved", "v1"]);

    let (s, d) = t
        .call("GET", "/api/v1/screens/s1/diff?from=v1&to=moved", None)
        .await;
    assert_eq!(s, StatusCode::OK, "{d}");
    let w = &d["diff"]["widgets"];
    assert_eq!(w["added"][0]["equipment_id"], "c");
    assert_eq!(w["changed"][0]["id"], "a");
    assert_eq!(w["changed"][0]["fields"], json!(["x"]));
    assert_eq!(
        d["diff"]["connections"]["removed"][0]["connection_id"],
        "c1"
    );
    assert_eq!(
        d["diff"]["summary"],
        json!({"added": 1, "removed": 1, "changed": 1})
    );
    // The YAML names tags by ref, not id.
    let from_yaml = d["yaml"]["from"].as_str().unwrap();
    assert!(from_yaml.contains("plc_1/p1_run"), "{from_yaml}");
    assert!(
        from_yaml.contains("runSensorId: plc_1/p1_run"),
        "{from_yaml}"
    );
    // By id, by seq, draft; unknown state is 404.
    let id1 = v1["version"]["id"].as_i64().unwrap();
    let (s, d) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s1/diff?from={id1}&to=draft&yaml=0"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(d.get("yaml").is_none());
    let upto = v2["version"]["upto_seq"].as_i64().unwrap();
    let (_, d) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s1/diff?from=seq:{upto}&to=moved"),
            None,
        )
        .await;
    assert_eq!(
        d["diff"]["summary"],
        json!({"added": 0, "removed": 0, "changed": 0})
    );
    let (s, _) = t
        .call("GET", "/api/v1/screens/s1/diff?from=nope", None)
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, st) = t.call("GET", "/api/v1/screens/s1/state?at=v1", None).await;
    assert_eq!(st["equipment"][0]["x"], json!(0.0));
}

#[tokio::test]
async fn reverting_objects_handles_connections_and_groups() {
    let t = T::new().await;
    t.settings(json!({"squash_window_secs": 0})).await;
    let s0 = t
        .put_layout(
            "s1",
            vec![
                eq("a", 0.0, json!({"group": "g1"})),
                eq("b", 10.0, json!({"group": "g1"})),
                eq("c", 20.0, json!({})),
            ],
            vec![conn("c1", "a", "b"), conn("c2", "b", "c")],
        )
        .await;
    let seq0 = s0["seq"].as_i64().unwrap();
    // Add d (with a connection), then delete b and its connections.
    t.put_layout(
        "s1",
        vec![
            eq("a", 0.0, json!({"group": "g1"})),
            eq("b", 10.0, json!({"group": "g1"})),
            eq("c", 20.0, json!({})),
            eq("d", 30.0, json!({})),
        ],
        vec![
            conn("c1", "a", "b"),
            conn("c2", "b", "c"),
            conn("c3", "c", "d"),
        ],
    )
    .await;
    t.put_layout(
        "s1",
        vec![
            eq("a", 0.0, json!({})),
            eq("c", 20.0, json!({})),
            eq("d", 30.0, json!({})),
        ],
        vec![conn("c3", "c", "d")],
    )
    .await;

    // Undoing d's add removes d and the connection ending on it, in one batch.
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"object": "d", "to_seq": seq0})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["removed_connections"], json!(["c3"]));
    assert_eq!(
        t.ids("s1", "").await,
        (vec!["a".into(), "c".into()], vec![])
    );
    let h = t
        .history("s1", &format!("?batch={}", r["batch"].as_str().unwrap()))
        .await;
    assert_eq!(
        kinds(&h),
        vec![
            ("connection".into(), "c3".into(), "remove".into()),
            ("widget".into(), "d".into(), "remove".into())
        ]
    );
    assert_eq!(h[0]["operation"], "revert");

    // Restoring c1 alone would dangle on b: refused, naming b.
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"object": "c1", "object_type": "connection", "to_seq": seq0})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{r}");
    assert_eq!(r["code"], "dangling_connection");
    assert_eq!(r["missing"], json!(["b"]));
    assert_eq!(t.ids("s1", "").await.0, vec!["a", "c"], "nothing written");

    // Together with b it works; b's group has no other member (a was
    // ungrouped meanwhile), so b comes back ungrouped.
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"objects": [
                {"object_id": "c1", "object_type": "connection", "to_seq": seq0},
                {"object_id": "b", "to_seq": seq0},
            ]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["ungrouped"], json!(["b"]));
    assert_eq!(
        t.ids("s1", "").await,
        (vec!["a".into(), "b".into(), "c".into()], vec!["c1".into()])
    );
    let b_cfg = t.layout("s1").await["equipment"][1]["config"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!b_cfg.contains("group"), "{b_cfg}");

    // Unknown states and stale bases.
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"to": "no-such-version"})),
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"to_seq": seq0, "base_seq": 1})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, _) = t
        .call("POST", "/api/v1/screens/s1/revert", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);

    // The whole screen back to seq0: a snapshot restore.
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"to_seq": seq0, "message": "back to the start"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        t.ids("s1", "").await,
        (
            vec!["a".into(), "b".into(), "c".into()],
            vec!["c1".into(), "c2".into()]
        )
    );
    let a_cfg = t.layout("s1").await["equipment"][0]["config"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(a_cfg.contains("g1"));
    let h = t.history("s1", "?limit=1").await;
    assert!(h[0]["note"].as_str().unwrap().contains("back to the start"));
}

#[tokio::test]
async fn operators_see_published_and_editors_the_draft() {
    let t = T::new().await;
    let op = t.user("olga", "operator").await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    // Never published: everyone sees the draft (as before phase 3).
    let (s, b) = t
        .call_as(&op, "GET", "/api/v1/screens/s1/scada-equipment", None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["view"], "draft");
    assert_eq!(b["items"].as_array().unwrap().len(), 1);

    let (s, p) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"name": "first", "message": "go live"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "published");
    assert_eq!(p["version"]["current"], true);
    t.put_layout(
        "s1",
        vec![eq("a", 0.0, json!({})), eq("draft_only", 5.0, json!({}))],
        vec![],
    )
    .await;

    // Operators (any route) see the published rows; editors the draft unless asked.
    for uri in [
        "/api/v1/screens/s1/scada-equipment",
        "/api/v1/screens/s1/scada-layout",
    ] {
        let (_, b) = t.call_as(&op, "GET", uri, None).await;
        assert_eq!(b["view"], "published", "{uri}");
    }
    assert_eq!(t.ids("s1", "").await.0, vec!["a", "draft_only"]);
    assert_eq!(t.ids("s1", "?published=1").await.0, vec!["a"]);
    let (_, st) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(st["published"]["name"], "first");
    assert_eq!(st["published"]["draft_differs"], true);
    assert_eq!(st["changes_since_published"], 1);

    // Operators may not read history or publish.
    let (s, _) = t
        .call_as(&op, "GET", "/api/v1/screens/s1/history", None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call_as(&op, "POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Publish an older version explicitly; a stale base is refused.
    let (s, _) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"base_seq": 1})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    let (s, p) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"version": "first"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");

    // Publishing off: everyone sees the draft and publish is refused.
    t.settings(json!({"publishing": false})).await;
    let (_, b) = t
        .call_as(&op, "GET", "/api/v1/screens/s1/scada-layout", None)
        .await;
    assert_eq!(b["view"], "draft");
    let (s, b) = t
        .call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert_eq!(b["code"], "publishing_off");

    // Deleting the screen forgets its published copy.
    t.settings(json!({"publishing": true})).await;
    t.call("POST", "/api/v1/screens", Some(json!({"name": "Temp"})))
        .await;
    t.put_layout("temp", vec![eq("z", 0.0, json!({}))], vec![])
        .await;
    t.call("POST", "/api/v1/screens/temp/publish", Some(json!({})))
        .await;
    let (s, _) = t.call("DELETE", "/api/v1/screens/temp", None).await;
    assert_eq!(s, StatusCode::OK);
    let (_, st) = t
        .call("GET", "/api/v1/screens/temp/design-status", None)
        .await;
    assert!(st["published"].is_null());
}

#[tokio::test]
async fn approval_needs_a_second_admin() {
    let t = T::new().await;
    t.settings(json!({"approval": "required"})).await;
    let op = t.user("olga", "operator").await;
    let second = t.user("bob", "admin").await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;

    let (s, p) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"message": "please review"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{p}");
    assert_eq!(p["status"], "pending");
    assert_eq!(p["version"]["requested_by"], "admin");
    let name = p["version"]["name"].as_str().unwrap().to_string();
    let (_, b) = t
        .call_as(&op, "GET", "/api/v1/screens/s1/scada-layout", None)
        .await;
    assert_eq!(
        b["view"], "draft",
        "nothing published yet (falls back to the draft)"
    );
    let (_, st) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(st["pending"][0]["name"], name.as_str());

    // The requester may not approve their own request.
    let (s, _) = t
        .call(
            "POST",
            &format!("/api/v1/screens/s1/versions/{name}/approve"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, r) = t
        .call_as(
            &second,
            "POST",
            &format!("/api/v1/screens/s1/versions/{name}/approve"),
            Some(json!({"note": "ok"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["status"], "published");
    assert_eq!(r["version"]["reviewed_by"], "bob");
    assert_eq!(r["version"]["published_by"], "bob");
    let (_, b) = t
        .call_as(&op, "GET", "/api/v1/screens/s1/scada-layout", None)
        .await;
    assert_eq!(b["view"], "published");
    // Approving twice: not pending any more.
    let (s, _) = t
        .call_as(
            &second,
            "POST",
            &format!("/api/v1/screens/s1/versions/{name}/approve"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);

    // A rejected request publishes nothing.
    t.put_layout("s1", vec![eq("a", 1.0, json!({}))], vec![])
        .await;
    let (_, p) = t
        .call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    let name = p["version"]["name"].as_str().unwrap().to_string();
    let (s, r) = t
        .call_as(
            &second,
            "POST",
            &format!("/api/v1/screens/s1/versions/{name}/reject"),
            Some(json!({"note": "not yet"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["version"]["status"], "rejected");
    assert_eq!(
        t.ids("s1", "?published=1").await,
        (vec!["a".into()], vec![])
    );
    let (_, l) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    assert_eq!(l["equipment"][0]["x"], json!(0.0));
}

#[tokio::test]
async fn keys_and_viewers_follow_the_configure_tier() {
    let t = T::new().await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    let (_, key) = t
        .call(
            "POST",
            "/api/v1/api-keys",
            Some(json!({"name": "ro", "role": "api"})),
        )
        .await;
    let ro = key["key"].as_str().unwrap().to_string();
    let (_, key) = t
        .call(
            "POST",
            "/api/v1/api-keys",
            Some(json!({"name": "int", "role": "integration"})),
        )
        .await;
    let int = key["key"].as_str().unwrap().to_string();
    let int_id = key["id"].as_i64().unwrap();
    let (s, _) = send_request(
        &t.app,
        "GET",
        "/api/v1/screens/s1/history",
        Some(&ro),
        None,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A read-only key sees what operators see.
    let (_, b) = send_request(
        &t.app,
        "GET",
        "/api/v1/screens/s1/scada-layout",
        Some(&ro),
        None,
        None,
    )
    .await;
    assert_eq!(b["equipment"].as_array().unwrap().len(), 1);
    // An integration key writes, and is logged as a key.
    let (s, _) = send_request(
        &t.app,
        "PUT",
        "/api/v1/screens/s1/scada-equipment",
        Some(&int),
        None,
        Some(json!({"items": [eq("a", 3.0, json!({}))]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let h = t.history("s1", "?limit=1").await;
    assert_eq!(h[0]["source"], "api_key");
    assert_eq!(h[0]["username"], format!("api_key:{int_id}"));
    // Approval and settings are for admins.
    let (s, _) = send_request(
        &t.app,
        "PUT",
        "/api/v1/settings/design-history",
        Some(&int),
        None,
        Some(json!({"publishing": false})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = send_request(&t.app, "GET", "/api/v1/design/git", Some(&int), None, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_audit_entry_of_a_save_names_its_batch() {
    let mut state = seeded_admin_app_state().await;
    let (logger, mut rx) = crate::security::AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (_, b) = send_request(
        &app,
        "PUT",
        "/api/v1/screens/s1/scada-layout",
        None,
        Some(&cookie),
        Some(json!({"equipment": [eq("a", 0.0, json!({}))], "connections": []})),
    )
    .await;
    let batch = b["batch"].as_str().unwrap();
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    let e = events
        .iter()
        .find(|e| e.action == "PUT /api/v1/screens/{id}/scada-layout")
        .expect("audited");
    assert!(
        e.details
            .as_deref()
            .unwrap()
            .contains(&format!("batch={batch}")),
        "{e:?}"
    );
    send_request(
        &app,
        "POST",
        "/api/v1/screens/s1/publish",
        None,
        Some(&cookie),
        Some(json!({"message": "m"})),
    )
    .await;
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(
        events.iter().any(|e| e.action == "design_publish"
            && e.details.as_deref().unwrap().contains("version=v1")),
        "{events:?}"
    );
}

#[tokio::test]
async fn export_and_import_options() {
    let t = T::new().await;
    t.import(PROJECT, "").await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "shipped"})),
    )
    .await;
    let mut layout = t.layout("s1").await;
    layout["equipment"][0]["x"] = json!(500.0);
    t.put_layout(
        "s1",
        layout["equipment"].as_array().unwrap().clone(),
        layout["connections"].as_array().unwrap().clone(),
    )
    .await;

    let export = |q: &'static str| {
        let app = t.app.clone();
        let cookie = t.admin.clone();
        async move {
            let req = http::Request::builder()
                .uri(format!("/api/v1/project/export{q}"))
                .header("cookie", cookie)
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = tower::ServiceExt::oneshot(app, req).await.unwrap();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            String::from_utf8(bytes.to_vec()).unwrap()
        }
    };
    let draft = export("").await;
    let published = export("?published=1").await;
    assert!(draft.contains("x: 500"), "{draft}");
    assert!(!published.contains("x: 500"), "{published}");
    assert!(draft.contains("published_version: shipped"));
    // The draft export imports back as "unchanged" (published_version is ignored).
    let req = http::Request::builder()
        .method("POST")
        .uri("/api/v1/project/import?dry_run=true")
        .header("cookie", &t.admin)
        .body(axum::body::Body::from(draft.clone()))
        .unwrap();
    let resp = tower::ServiceExt::oneshot(t.app.clone(), req)
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let plan: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(plan["totals"]["update"], 0, "{plan}");

    // An import never publishes unless asked; with publish=1 it does.
    let changed = PROJECT.replace("x: 200", "x: 222");
    t.import(&changed, "").await;
    let (_, l) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    assert_eq!(l["equipment"][1]["x"], json!(200.0));
    let plan = t
        .import(&PROJECT.replace("x: 200", "x: 333"), "?publish=1")
        .await;
    assert_eq!(plan["published"], json!(["s1"]));
    let (_, l) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    assert_eq!(l["equipment"][1]["x"], json!(333.0));
}

fn git_ok() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fresh folder inside the server's data folder, and its path relative
/// to it.
fn data_tmp(t: &T) -> (tempfile::TempDir, String) {
    let data = crate::design_git::data_dir(&t.state.storage);
    std::fs::create_dir_all(&data).unwrap();
    let tmp = tempfile::tempdir_in(&data).unwrap();
    let rel = tmp
        .path()
        .strip_prefix(&data)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    (tmp, rel)
}

#[tokio::test]
async fn git_sync_pushes_the_published_project_to_a_bare_repo() {
    if !git_ok() {
        eprintln!("git not installed: skipped");
        return;
    }
    let t = T::new().await;
    // A local remote and the working copy live in the data folder.
    let (tmp, rel) = data_tmp(&t);
    let bare = tmp.path().join("plant.git");
    git(
        tmp.path(),
        &["init", "-q", "--bare", bare.to_str().unwrap()],
    );
    let work = format!("{rel}/work");

    // Refused settings.
    let (s, _) = t
        .call(
            "PUT",
            "/api/v1/design/git",
            Some(json!({"enabled": true, "remote": "--upload-pack=x"})),
        )
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/design/git",
            Some(json!({
                "enabled": true, "remote": bare.to_str().unwrap(), "branch": "main",
                "path": "sites/plant.yaml", "push_on_publish": false,
                "work_dir": work, "secret": "not-shown",
            })),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["has_secret"], true);
    let (_, g) = t.call("GET", "/api/v1/design/git", None).await;
    assert!(
        !g.to_string().contains("not-shown"),
        "the secret is never returned"
    );
    let stored = t
        .state
        .storage
        .get_settings(&[crate::design_git::SECRET_KEY])
        .await
        .unwrap();
    assert!(stored[0].value.starts_with("enc:v1:"), "encrypted at rest");

    t.import(PROJECT, "").await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "r1"})),
    )
    .await;
    let (s, o) = t
        .call(
            "POST",
            "/api/v1/design/git/sync",
            Some(json!({"message": "Publish s1 r1"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{o}");
    assert_eq!(o["committed"], true);
    assert_eq!(o["pushed"], true);
    let log = git(&bare, &["log", "--format=%an|%s", "main"]);
    assert_eq!(log.trim(), "admin|Publish s1 r1");
    let file = git(&bare, &["show", "main:sites/plant.yaml"]);
    assert!(
        file.contains("format: voltrus-project/1") && file.contains("published_version: r1"),
        "{file}"
    );

    // Nothing changed: no new commit.
    let (_, o) = t.call("POST", "/api/v1/design/git/sync", None).await;
    assert_eq!(
        o["committed"],
        false,
        "{o} {}",
        git(&bare, &["diff", "main~1", "main"])
    );

    // A draft edit is not pushed (the file is the published state)...
    let mut layout = t.layout("s1").await;
    layout["equipment"][0]["x"] = json!(640.0);
    t.put_layout(
        "s1",
        layout["equipment"].as_array().unwrap().clone(),
        vec![],
    )
    .await;
    let (_, o) = t.call("POST", "/api/v1/design/git/sync", None).await;
    assert_eq!(
        o["committed"],
        false,
        "{o} {}",
        git(&bare, &["diff", "main~1", "main"])
    );
    // ...until it is published; push_on_publish syncs in the background.
    t.call(
        "PUT",
        "/api/v1/design/git",
        Some(json!({"push_on_publish": true})),
    )
    .await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "r2", "message": "moved a"})),
    )
    .await;
    let mut log = String::new();
    for _ in 0..100 {
        log = git(&bare, &["log", "--format=%s", "main"]);
        if log.lines().count() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(log.lines().next(), Some("Publish s1 r2: moved a"), "{log}");
    assert!(git(&bare, &["show", "main:sites/plant.yaml"]).contains("x: 640"));
    let (_, g) = t.call("GET", "/api/v1/design/git", None).await;
    assert_eq!(g["status"]["ok"], true, "{g}");
}

/// A seq inside a batch means the state after that whole batch: a connection
/// saved with its widgets restores with them whichever row's seq is named.
#[tokio::test]
async fn a_seq_inside_a_batch_means_after_the_batch() {
    // The default squash window: the second save does not absorb the first
    // (it changes objects the first one touched together with others only
    // when it changes all of them, and then the whole batch goes).
    let t = T::new().await;
    t.put_layout(
        "s1",
        vec![eq("p", 0.0, json!({})), eq("q", 1.0, json!({}))],
        vec![conn("c", "p", "q")],
    )
    .await;
    let rows = t.history("s1", "").await;
    let first = rows
        .iter()
        .map(|c| c["seq"].as_i64().unwrap())
        .min()
        .unwrap();
    t.put_layout("s1", vec![], vec![]).await;
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"objects": [
                {"object_id": "c", "object_type": "connection", "to_seq": first},
                {"object_id": "p", "to_seq": first},
                {"object_id": "q", "to_seq": first},
            ]})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(
        t.ids("s1", "").await,
        (vec!["p".into(), "q".into()], vec!["c".into()])
    );
    // A revision-only stale save gets the recent changes, not the whole log.
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/screens/s1/scada-layout",
            Some(json!({"equipment": [], "connections": [], "base_revision": "stale"})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(b["changes_since"].as_array().unwrap().len() <= 50);
}

// ── Review fixes: consistency of squashing, publish rules, git hardening ──

/// A key of `role` created by the admin: (key, id).
async fn key(t: &T, role: &str) -> (String, i64) {
    let (s, k) = t
        .call(
            "POST",
            "/api/v1/api-keys",
            Some(json!({"name": format!("k-{role}"), "role": role})),
        )
        .await;
    assert!(s.is_success(), "{k}");
    (
        k["key"].as_str().unwrap().to_string(),
        k["id"].as_i64().unwrap(),
    )
}

impl T {
    async fn import_as(&self, cookie: &str, yaml: &str, query: &str) -> (StatusCode, Value) {
        let req = http::Request::builder()
            .method("POST")
            .uri(format!("/api/v1/project/import{query}"))
            .header("cookie", cookie)
            .header("content-type", "application/yaml")
            .body(axum::body::Body::from(yaml.to_string()))
            .unwrap();
        let resp = tower::ServiceExt::oneshot(self.app.clone(), req)
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn published_x(&self, screen: &str, i: usize) -> Value {
        let (_, l) = self
            .call(
                "GET",
                &format!("/api/v1/screens/{screen}/scada-layout?published=1"),
                None,
            )
            .await;
        l["equipment"][i]["x"].clone()
    }
}

/// A live squash absorbs a whole batch or nothing: moving one of three
/// objects saved together leaves the first batch whole, and a seq the
/// squash removed is refused (410) instead of rebuilding a state that never
/// existed. Squashing never lowers the head.
#[tokio::test]
async fn squashing_keeps_batches_whole_and_refuses_lost_seqs() {
    let t = T::new().await;
    let first = t
        .put_layout(
            "s1",
            vec![eq("p", 0.0, json!({})), eq("q", 1.0, json!({}))],
            vec![conn("c", "p", "q")],
        )
        .await;
    let first_seq = first["seq"].as_i64().unwrap();
    // Within the window: only p moves, the first batch (p, q, c) stays.
    t.put_layout(
        "s1",
        vec![eq("p", 5.0, json!({})), eq("q", 1.0, json!({}))],
        vec![conn("c", "p", "q")],
    )
    .await;
    assert_eq!(t.history("s1", "").await.len(), 4);
    let (s, st) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s1/state?at=seq:{first_seq}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{st}");
    assert_eq!(st["equipment"][0]["x"], json!(0.0));
    assert_eq!(st["connections"].as_array().unwrap().len(), 1);

    // A save of one object, then another of it: the second absorbs the first.
    let one = t
        .put_layout("s2", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    let one_seq = one["seq"].as_i64().unwrap();
    t.put_layout("s2", vec![eq("a", 1.0, json!({}))], vec![])
        .await;
    let h = t.history("s2", "").await;
    assert_eq!(h.len(), 1, "{h:?}");
    assert_eq!(h[0]["kind"], "add");
    assert_eq!(h[0]["squashed"], 1);
    let (s, b) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s2/state?at=seq:{one_seq}"),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::GONE, "{b}");
    assert_eq!(b["code"], "state_gone");
    // Removing it is never squashed: added, then deleted.
    t.put_layout("s2", vec![], vec![]).await;
    assert_eq!(t.history("s2", "").await.len(), 2);
    // Moved and moved back within the window: the rows cancel out, and the
    // head still moves on.
    t.put_layout("s3", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    t.state
        .storage
        .with_conn(|c| {
            c.execute(
                "UPDATE design_changes SET at_ms = at_ms - 3600000 WHERE screen_id = 's3'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let two = t
        .put_layout("s3", vec![eq("a", 1.0, json!({}))], vec![])
        .await;
    let back = t
        .put_layout("s3", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    assert_eq!(t.history("s3", "").await.len(), 1);
    assert!(back["seq"].as_i64().unwrap() > two["seq"].as_i64().unwrap());
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/screens/s3/scada-layout",
            Some(json!({"equipment": [], "connections": [], "base_seq": back["seq"]})),
        )
        .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the head the save answered is current: {b}"
    );
}

/// Reverting a whole screen to a state follows the object rules too.
#[tokio::test]
async fn reverting_a_screen_drops_dangling_connections_and_lone_groups() {
    let t = T::new().await;
    t.settings(json!({"squash_window_secs": 0})).await;
    // A state written straight into the log that has a connection without
    // one of its widgets, and a group of one (an older log could).
    t.put_layout(
        "s1",
        vec![eq("p", 0.0, json!({"group": "g"})), eq("q", 1.0, json!({}))],
        vec![conn("c", "p", "q")],
    )
    .await;
    let v = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "broken"})),
        )
        .await
        .1;
    let id = v["version"]["id"].as_i64().unwrap();
    t.state
        .storage
        .with_conn(move |c| {
            let snap = json!({"equipment": [{"screen_id": "s1", "equipment_id": "p", "equipment_type": "label",
                "equipment_name": "", "x": 0.0, "y": 0.0, "width": 100.0, "height": 40.0, "rotation": 0.0,
                "sensor_bindings": "[99999]", "config": "{\"group\":\"g\"}"}],
                "connections": [{"screen_id": "s1", "connection_id": "c", "from_equipment": "p", "to_equipment": "q",
                "waypoints": "[]", "connection_type": "flow"}]});
            c.execute(
                "UPDATE design_versions SET snapshot = ?2 WHERE id = ?1",
                rusqlite::params![id, snap.to_string()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let (s, r) = t
        .call(
            "POST",
            "/api/v1/screens/s1/revert",
            Some(json!({"to": "broken"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{r}");
    assert_eq!(r["removed_connections"], json!(["c"]));
    assert_eq!(r["ungrouped"], json!(["p"]));
    assert_eq!(r["missing_tags"], json!([99999]));
    assert_eq!(t.ids("s1", "").await, (vec!["p".into()], vec![]));
}

/// Daily squashing merges one author's rows only, and states at or below
/// what it merged are refused afterwards.
#[tokio::test]
async fn hourly_squash_keeps_authors_and_sources_apart() {
    let t = T::new().await;
    t.settings(json!({"squash_window_secs": 0})).await;
    let bob = t.user("bob", "admin").await;
    let mut seqs = Vec::new();
    for (who, x) in [
        (&t.admin, 1.0),
        (&bob, 2.0),
        (&t.admin, 3.0),
        (&t.admin, 4.0),
    ] {
        let (s, b) = t
            .call_as(
                who,
                "PUT",
                "/api/v1/screens/s1/scada-layout",
                Some(json!({"equipment": [eq("a", x, json!({}))], "connections": []})),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        seqs.push(b["seq"].as_i64().unwrap());
    }
    let old = crate::timeutil::now_millis() - 40 * 86_400_000;
    t.state
        .storage
        .with_conn(move |c| {
            c.execute("UPDATE design_changes SET at_ms = ?1", [old])?;
            Ok(())
        })
        .await
        .unwrap();
    let report = t.state.storage.design_maintain().await.unwrap();
    assert_eq!(
        report.squashed, 2,
        "admin's three rows become one: {report:?}"
    );
    let h = t.history("s1", "").await;
    let who: Vec<&str> = h.iter().map(|c| c["username"].as_str().unwrap()).collect();
    assert_eq!(who, ["admin", "bob"], "{h:?}");
    let (s, b) = t
        .call(
            "GET",
            &format!("/api/v1/screens/s1/state?at=seq:{}", seqs[1]),
            None,
        )
        .await;
    assert_eq!(s, StatusCode::GONE, "{b}");
    let (_, hist) = t.call("GET", "/api/v1/screens/s1/history", None).await;
    assert!(hist["floor"].as_i64().unwrap() >= seqs[3]);
    // Retention settings are checked.
    for bad in [
        json!({"squash_window_secs": 3601}),
        json!({"keep_days": 36_501}),
        json!({"squash_after_days": u64::MAX}),
    ] {
        let (s, _) = t
            .call("PUT", "/api/v1/settings/design-history", Some(bad.clone()))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
    }
}

/// Publish after import follows the site's switches: refused while
/// publishing is off, publish requests with approval required (from a
/// person only), nothing new when the draft is published already.
#[tokio::test]
async fn import_publish_follows_publishing_and_approval() {
    let t = T::new().await;
    t.settings(json!({"publishing": false})).await;
    let (s, b) = t.import_as(&t.admin, PROJECT, "?publish=1").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "publishing_off");
    assert!(t.history("s1", "").await.is_empty(), "nothing was written");

    t.settings(json!({"publishing": true})).await;
    let plan = t.import(PROJECT, "?publish=1").await;
    assert_eq!(plan["published"], json!(["s1"]));
    // The same document again: nothing to publish, no new version.
    let plan = t.import(PROJECT, "?publish=1").await;
    assert!(plan.get("published").is_none(), "{plan}");
    let (_, v) = t.call("GET", "/api/v1/screens/s1/versions", None).await;
    assert_eq!(v["versions"].as_array().unwrap().len(), 1);

    t.settings(json!({"approval": "required"})).await;
    let plan = t
        .import(&PROJECT.replace("x: 200", "x: 300"), "?publish=1")
        .await;
    assert!(plan.get("published").is_none(), "{plan}");
    assert_eq!(plan["publish_requested"], json!(["s1"]));
    assert_eq!(t.published_x("s1", 1).await, json!(200.0));
    let (_, st) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(st["pending"][0]["requested_by"], "admin");

    // Keys and demo accounts may not file requests.
    let (int, _) = key(&t, "integration").await;
    let req = http::Request::builder()
        .method("POST")
        .uri("/api/v1/project/import?publish=1")
        .header("X-API-Key", &int)
        .header("content-type", "application/yaml")
        .body(axum::body::Body::from(PROJECT.to_string()))
        .unwrap();
    let resp = tower::ServiceExt::oneshot(t.app.clone(), req)
        .await
        .unwrap();
    assert!(
        matches!(resp.status(), StatusCode::FORBIDDEN),
        "{}",
        resp.status()
    );
}

/// With approval required a publish request needs a person: API keys and
/// demo accounts are refused; nobody approves what they wrote, through a
/// key of theirs either; two approvals at once publish once.
#[tokio::test]
async fn approval_is_by_someone_who_wrote_none_of_it() {
    let t = T::new().await;
    t.settings(json!({"approval": "required"})).await;
    let bob = t.user("bob", "admin").await;
    let carol = t.user("carol", "admin").await;
    let demo = t.user("dora", "demo").await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;

    let (int, _) = key(&t, "integration").await;
    let (s, b) = send_request(
        &t.app,
        "POST",
        "/api/v1/screens/s1/publish",
        Some(&int),
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(b["code"], "approval_needs_person");
    let (s, _) = t
        .call_as(&demo, "POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    // Bob's key (created by bob) changes the screen; admin requests.
    let (s, k) = t
        .call_as(
            &bob,
            "POST",
            "/api/v1/api-keys",
            Some(json!({"name": "bobs", "role": "integration"})),
        )
        .await;
    assert!(s.is_success(), "{k}");
    let bobs = k["key"].as_str().unwrap().to_string();
    let (s, _) = send_request(
        &t.app,
        "PUT",
        "/api/v1/screens/s1/scada-equipment",
        Some(&bobs),
        None,
        Some(json!({"items": [eq("a", 7.0, json!({}))]})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, p) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"name": "r1"})),
        )
        .await;
    assert_eq!(p["status"], "pending", "{p}");
    // Bob wrote part of it through his key: not his to approve.
    let (s, b) = t
        .call_as(&bob, "POST", "/api/v1/screens/s1/versions/r1/approve", None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert_eq!(b["code"], "self_review");

    // Carol and a second reviewer at once: one publishes, one is told.
    let dave = t.user("dave", "admin").await;
    let (a, b) = tokio::join!(
        t.call_as(
            &carol,
            "POST",
            "/api/v1/screens/s1/versions/r1/approve",
            None
        ),
        t.call_as(
            &dave,
            "POST",
            "/api/v1/screens/s1/versions/r1/approve",
            None
        ),
    );
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CONFLICT],
        "{a:?} {b:?}"
    );
    assert_eq!(t.published_x("s1", 0).await, json!(7.0));

    // A version older than the published one needs confirm_older.
    let (_, old) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "older"})),
        )
        .await;
    assert_eq!(old["version"]["name"], "older");
    t.put_layout("s1", vec![eq("a", 9.0, json!({}))], vec![])
        .await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "r2"})),
    )
    .await;
    t.call_as(
        &carol,
        "POST",
        "/api/v1/screens/s1/versions/r2/approve",
        None,
    )
    .await;
    assert_eq!(t.published_x("s1", 0).await, json!(9.0));
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"version": "older"})),
    )
    .await;
    let (s, b) = t
        .call_as(
            &carol,
            "POST",
            "/api/v1/screens/s1/versions/older/approve",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "older_than_published");
    let (s, b) = t
        .call_as(
            &carol,
            "POST",
            "/api/v1/screens/s1/versions/older/approve",
            Some(json!({"confirm_older": true})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(t.published_x("s1", 0).await, json!(7.0));
}

/// Dropping approval (or turning publishing off) while approval is
/// required waits for a second admin.
#[tokio::test]
async fn relaxing_approval_needs_a_second_admin() {
    let t = T::new().await;
    t.settings(json!({"approval": "required"})).await;
    let bob = t.user("bob", "admin").await;
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/settings/design-history",
            Some(json!({"approval": "none"})),
        )
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    assert_eq!(b["status"], "pending");
    let (_, g) = t.call("GET", "/api/v1/settings/design-history", None).await;
    assert_eq!(g["approval"], "required");
    assert_eq!(g["pending_request"]["requested_by"], "admin");
    // Not by the requester; a second request waits its turn.
    let (s, _) = t
        .call("POST", "/api/v1/settings/design-history/approve", None)
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _) = t
        .call(
            "PUT",
            "/api/v1/settings/design-history",
            Some(json!({"publishing": false})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT);
    // Retention changes need nobody else.
    t.settings(json!({"keep_days": 400})).await;
    let (s, b) = t
        .call_as(
            &bob,
            "POST",
            "/api/v1/settings/design-history/approve",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, g) = t.call("GET", "/api/v1/settings/design-history", None).await;
    assert_eq!(g["approval"], "none");
    assert_eq!(g["pending_request"], Value::Null);
    // Without approval required, one admin decides.
    t.settings(json!({"publishing": false})).await;
}

/// A snapshot rollback restores what operators saw, not only the draft.
#[tokio::test]
async fn rollback_restores_the_published_screens() {
    let t = T::new().await;
    t.put_layout("s1", vec![eq("a", 1.0, json!({}))], vec![])
        .await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "one"})),
    )
    .await;
    let snap = t
        .state
        .storage
        .create_config_snapshot(Some("before-two"), None, "manual", Some("admin"))
        .await
        .unwrap();
    t.put_layout("s1", vec![eq("a", 2.0, json!({}))], vec![])
        .await;
    t.call(
        "POST",
        "/api/v1/screens/s1/publish",
        Some(json!({"name": "two"})),
    )
    .await;
    assert_eq!(t.published_x("s1", 0).await, json!(2.0));
    let (s, b) = t
        .call(
            "POST",
            &format!("/api/v1/config/snapshots/{snap}/rollback"),
            None,
        )
        .await;
    assert!(s.is_success(), "{s} {b}");
    assert_eq!(t.published_x("s1", 0).await, json!(1.0));
    let (_, st) = t
        .call("GET", "/api/v1/screens/s1/design-status", None)
        .await;
    assert_eq!(st["published"]["name"], "one");
}

/// A tag rename reaches saved versions: publishing an older one binds the
/// tag by its new name.
#[tokio::test]
async fn a_tag_rename_reaches_saved_versions() {
    let t = T::new().await;
    t.import(PROJECT, "").await;
    let mut layout = t.layout("s1").await;
    let mut config: Value =
        serde_json::from_str(layout["equipment"][1]["config"].as_str().unwrap()).unwrap();
    config["bindings"] = json!({"label": {"kind": "tag", "tag": "plc_1/level"}});
    layout["equipment"][1]["config"] = json!(config.to_string());
    t.put_layout(
        "s1",
        layout["equipment"].as_array().unwrap().clone(),
        layout["connections"].as_array().unwrap().clone(),
    )
    .await;
    t.call(
        "POST",
        "/api/v1/screens/s1/versions",
        Some(json!({"name": "bound"})),
    )
    .await;
    let level = t
        .state
        .storage
        .list_data_sources(None)
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.key == "level")
        .unwrap();
    let (s, b) = t
        .call(
            "PUT",
            &format!("/api/v1/data-sources/{}", level.id),
            Some(json!({"key": "tank_level"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"version": "bound"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, l) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    let c = l["equipment"][1]["config"].as_str().unwrap();
    assert!(
        c.contains("plc_1/tank_level") && !c.contains("plc_1/level\""),
        "{c}"
    );
}

/// A faceplate the published screen still opens is not deleted, though the
/// draft no longer uses it; deleting an equipment instance unbinds the
/// published widgets too.
#[tokio::test]
async fn published_screens_keep_their_faceplates_and_lose_deleted_instances() {
    let t = T::new().await;
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/faceplates",
            Some(json!({"id": "fp", "name": "FP", "kind": "pump", "roles": [{"role": "run", "kind": "state"}]})),
        )
        .await;
    assert!(s.is_success(), "{b}");
    t.put_layout("s1", vec![eq("a", 0.0, json!({"faceplate": "fp"}))], vec![])
        .await;
    t.call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    let (s, b) = t.call("DELETE", "/api/v1/faceplates/fp", None).await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert!(b.to_string().contains("s1/a"), "{b}");

    // An instance bound in the published copy only.
    t.state
        .storage
        .with_conn(|c| {
            c.execute(
                "UPDATE scada_equipment_published SET config = '{\"equipment\":\"P-1\"}' WHERE screen_id = 's1'",
                [],
            )?;
            c.execute(
                "INSERT INTO equipment_instances (name, type_id, device_id, created_at, updated_at) VALUES ('P-1', 'none', 'd', 0, 0)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let out = t
        .state
        .storage
        .delete_equipment_instance(
            "P-1",
            false,
            &crate::storage::DesignActor::system("equipment"),
        )
        .await
        .unwrap();
    assert!(out.is_some());
    let (_, l) = t
        .call("GET", "/api/v1/screens/s1/scada-layout?published=1", None)
        .await;
    let c = l["equipment"][0]["config"].as_str().unwrap();
    assert!(
        c.contains("unboundEquipment") && !c.contains("\"equipment\""),
        "{c}"
    );
}

/// Version names never read as another state, and `id:`/`name:` pick one.
#[tokio::test]
async fn version_references_are_unambiguous() {
    let t = T::new().await;
    t.put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    for bad in ["12", "draft", "published", "seq:3", "id:1"] {
        let (s, b) = t
            .call(
                "POST",
                "/api/v1/screens/s1/versions",
                Some(json!({"name": bad})),
            )
            .await;
        assert_eq!(s, StatusCode::CONFLICT, "{bad}: {b}");
    }
    let (_, v) = t
        .call(
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "r"})),
        )
        .await;
    let id = v["version"]["id"].as_i64().unwrap();
    for r in [
        format!("{id}"),
        format!("id:{id}"),
        "name:r".into(),
        "r".into(),
    ] {
        let (s, b) = t
            .call("GET", &format!("/api/v1/screens/s1/versions/{r}"), None)
            .await;
        assert_eq!(s, StatusCode::OK, "{r}: {b}");
        assert_eq!(b["version"]["id"], id);
    }
    // With publishing off the published state says it shows the draft.
    t.settings(json!({"publishing": false})).await;
    let (_, st) = t
        .call("GET", "/api/v1/screens/s1/state?at=published", None)
        .await;
    assert_eq!(st["state"]["publishing"], false);
    assert_eq!(st["state"]["shows"], "draft");
}

/// The publish is checked against what was signed: a save after it was
/// proposed is refused (409), as is a stale base, before any signature.
#[tokio::test]
async fn a_publish_refuses_a_draft_that_moved() {
    let t = T::new().await;
    let saved = t
        .put_layout("s1", vec![eq("a", 0.0, json!({}))], vec![])
        .await;
    let base = saved["seq"].as_i64().unwrap();
    t.put_layout(
        "s1",
        vec![eq("a", 1.0, json!({})), eq("b", 1.0, json!({}))],
        vec![],
    )
    .await;
    let (s, b) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"base_seq": base})),
        )
        .await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");
    assert_eq!(b["code"], "conflict");
    let (_, v) = t.call("GET", "/api/v1/screens/s1/versions", None).await;
    assert!(
        v["versions"].as_array().unwrap().is_empty(),
        "nothing recorded"
    );
}

/// Git sync settings refuse credentials in the remote, a token over
/// http://, a working copy or a local remote outside the data folder.
#[tokio::test]
async fn git_settings_refuse_unsafe_remotes_and_folders() {
    let t = T::new().await;
    let outside = tempfile::tempdir().unwrap();
    for (body, what) in [
        (
            json!({"remote": "https://bob:tok@git.example/x.git"}),
            "userinfo",
        ),
        (
            json!({"remote": "http://git.example/x.git", "auth": "token"}),
            "http token",
        ),
        (
            json!({"remote": "https://git.example/x.git", "work_dir": "/etc"}),
            "absolute work_dir",
        ),
        (
            json!({"remote": "https://git.example/x.git", "work_dir": "../x"}),
            "work_dir escape",
        ),
        (
            json!({"remote": outside.path().to_str().unwrap()}),
            "local outside",
        ),
    ] {
        let (s, b) = t
            .call("PUT", "/api/v1/design/git", Some(body.clone()))
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{what}: {body} {b}");
    }
    // A quote is fine in a (relative) folder name: it is never put on a
    // command line unquoted.
    let (s, b) = t
        .call(
            "PUT",
            "/api/v1/design/git",
            Some(json!({"remote": "https://git.example/x.git", "work_dir": "it's"})),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// A remote whose tree has the project file (or a folder on its path) as a
/// symbolic link is refused, and nothing outside the repository is read or
/// written.
#[cfg(unix)]
#[tokio::test]
async fn git_sync_never_follows_symlinks_in_the_remote() {
    if !git_ok() {
        eprintln!("git not installed: skipped");
        return;
    }
    let t = T::new().await;
    let (tmp, rel) = data_tmp(&t);
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.yaml");
    std::fs::write(&victim, "keep me\n").unwrap();
    t.import(PROJECT, "").await;
    t.call("POST", "/api/v1/screens/s1/publish", Some(json!({})))
        .await;

    for (n, (path, link_at, target)) in [
        ("project.yaml", "project.yaml", victim.clone()),
        ("sites/plant.yaml", "sites", outside.path().to_path_buf()),
    ]
    .into_iter()
    .enumerate()
    {
        let bare = tmp.path().join(format!("evil{n}.git"));
        git(
            tmp.path(),
            &["init", "-q", "--bare", bare.to_str().unwrap()],
        );
        let src = tmp.path().join(format!("src{n}"));
        git(
            tmp.path(),
            &["init", "-q", "-b", "main", src.to_str().unwrap()],
        );
        std::os::unix::fs::symlink(&target, src.join(link_at)).unwrap();
        git(&src, &["add", "-A"]);
        git(
            &src,
            &[
                "-c",
                "user.name=x",
                "-c",
                "user.email=x@x",
                "commit",
                "-q",
                "-m",
                "evil",
            ],
        );
        git(&src, &["push", "-q", bare.to_str().unwrap(), "main"]);
        let (s, b) = t
            .call(
                "PUT",
                "/api/v1/design/git",
                Some(
                    json!({"enabled": true, "remote": bare.to_str().unwrap(), "branch": "main",
                    "path": path, "push_on_publish": false, "work_dir": format!("{rel}/work{n}")}),
                ),
            )
            .await;
        assert_eq!(s, StatusCode::OK, "{b}");
        let (s, o) = t.call("POST", "/api/v1/design/git/sync", None).await;
        assert_eq!(s, StatusCode::BAD_GATEWAY, "{path}: {o}");
        assert!(
            o["error"].as_str().unwrap().contains("symbolic link"),
            "{o}"
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me\n");
        assert!(!outside.path().join("plant.yaml").exists());
        // The working copy is bare: nothing was checked out.
        assert!(!tmp.path().join(format!("work{n}")).join(link_at).exists());
    }
}

/// The author of a version cannot approve its publish request, though
/// someone else filed it.
#[tokio::test]
async fn the_version_author_cannot_approve_it() {
    let t = T::new().await;
    t.settings(json!({"approval": "required"})).await;
    let bob = t.user("bob", "admin").await;
    let carol = t.user("carol", "admin").await;
    t.call_as(
        &carol,
        "PUT",
        "/api/v1/screens/s1/scada-layout",
        Some(json!({"equipment": [eq("a", 1.0, json!({}))], "connections": []})),
    )
    .await;
    // Bob records the version; carol's change is in it too.
    let (s, v) = t
        .call_as(
            &bob,
            "POST",
            "/api/v1/screens/s1/versions",
            Some(json!({"name": "bobs"})),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, p) = t
        .call(
            "POST",
            "/api/v1/screens/s1/publish",
            Some(json!({"version": "bobs"})),
        )
        .await;
    assert_eq!(p["status"], "pending", "{p}");
    for who in [&bob, &carol] {
        let (s, b) = t
            .call_as(
                who,
                "POST",
                "/api/v1/screens/s1/versions/bobs/approve",
                None,
            )
            .await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
        assert_eq!(b["code"], "self_review");
    }
    let dave = t.user("dave", "admin").await;
    let (s, b) = t
        .call_as(
            &dave,
            "POST",
            "/api/v1/screens/s1/versions/bobs/approve",
            None,
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
}
