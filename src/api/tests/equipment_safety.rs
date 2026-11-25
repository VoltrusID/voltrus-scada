//! Equipment writes that would reach into what others own: another
//! instance's tags, registers and flows, a user's flow, a widget's own
//! settings, a stale Designer save; roles, snapshots and concurrency.

use super::*;
use http::StatusCode;
use serde_json::{Value, json};

struct T {
    app: axum::Router,
    cookie: String,
    state: AppState,
}

impl T {
    async fn new() -> Self {
        let state = seeded_admin_app_state().await;
        let app = create_router(state.clone());
        let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
        let t = T { app, cookie, state };
        let (status, body) = t
            .call(
                "POST",
                "/api/v1/equipment-types/library/install",
                Some(json!({"ids": ["pump"]})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        t
    }
    async fn call(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        send_request(&self.app, method, uri, None, Some(&self.cookie), body).await
    }
    async fn sql(&self, q: &str) -> Vec<Vec<Value>> {
        let q = q.to_string();
        self.state
            .storage
            .with_conn(move |c| {
                let mut stmt = c.prepare(&q)?;
                let n = stmt.column_count();
                let rows = stmt
                    .query_map([], |r| {
                        let mut out = Vec::new();
                        for i in 0..n {
                            let v: rusqlite::types::Value = r.get(i)?;
                            out.push(match v {
                                rusqlite::types::Value::Integer(i) => json!(i),
                                rusqlite::types::Value::Real(f) => json!(f),
                                rusqlite::types::Value::Text(t) => json!(t),
                                _ => Value::Null,
                            });
                        }
                        Ok(out)
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
            .unwrap()
    }
    /// An empty Modbus device `id`.
    async fn plc(&self, id: &str) {
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/modbus-devices",
                Some(json!({"id": id, "name": id, "ip": "127.0.0.1", "port": 1, "registers": []})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    async fn create(&self, body: Value) -> (StatusCode, Value) {
        self.call("POST", "/api/v1/equipment-instances", Some(body))
            .await
    }
    /// The pump type with a per-instance flow `trip` (stop on trip).
    async fn pump_with_flow(&self) {
        let (_, mut pump) = self.call("GET", "/api/v1/equipment-types/pump", None).await;
        pump["flows"] = json!([{
            "id": "trip", "name": "{name} trip stop", "enabled": false,
            "trigger": {"type": "tag_change", "tags": ["{ref:fault}"]},
            "nodes": [{"type": "write_out", "id": "n1", "tag": "{ref:cmd}", "policy": "immediate", "reason": "trip"}]
        }]);
        let (status, body) = self
            .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
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
}

fn symbol_row(id: &str, equipment: &str, extra: Value) -> Value {
    let mut config = json!({"equipment": equipment, "z": 1});
    for (k, v) in extra.as_object().cloned().unwrap_or_default() {
        config[k] = v;
    }
    json!({"equipment_id": id, "equipment_type": "symbol", "equipment_name": "", "x": 10, "y": 10,
           "width": 90, "height": 100, "rotation": 0, "sensor_bindings": "[]", "config": config.to_string()})
}

fn row_config(layout: &Value, id: &str) -> Value {
    let row = layout["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["equipment_id"] == id)
        .unwrap();
    serde_json::from_str(row["config"].as_str().unwrap()).unwrap()
}

fn error_of(body: &Value) -> String {
    body["error"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn another_instance_cannot_take_an_instances_prefix_or_tags() {
    let t = T::new().await;
    t.plc("plc").await;
    let (status, body) = t
        .create(json!({"name": "P-101A", "type": "pump", "device": "plc", "base": 0}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let before = t
        .sql("SELECT key, name, writable FROM data_sources WHERE device_id='plc' ORDER BY key")
        .await;

    // The same default prefix (p101a), the same prefix in another case, and
    // a name that differs only in case: each refused, naming P-101A.
    for (body, want) in [
        (
            json!({"name": "P101A", "type": "pump", "device": "plc", "base": 100}),
            "prefix 'p101a' is the prefix of equipment instance 'P-101A'",
        ),
        (
            json!({"name": "Pump 7", "type": "pump", "device": "plc", "prefix": "P101A", "base": 100}),
            "prefix 'P101A' is the prefix of equipment instance 'P-101A'",
        ),
        (
            json!({"name": "p-101a", "type": "pump", "device": "plc", "prefix": "other", "base": 100}),
            "differs from equipment instance 'P-101A' only in case",
        ),
        (
            json!({"name": "P-7", "type": "pump", "device": "plc", "base": 100,
                   "overrides": {"run": {"key": "p101a_run"}}}),
            "gives tag 'plc/p101a_run', which equipment instance 'P-101A' has too",
        ),
    ] {
        for dry in [true, false] {
            let (status, resp) = t
                .call(
                    "POST",
                    &format!("/api/v1/equipment-instances?dry_run={dry}"),
                    Some(body.clone()),
                )
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {resp}");
            assert!(error_of(&resp).contains(want), "{want}: {resp}");
        }
    }
    let after = t
        .sql("SELECT key, name, writable FROM data_sources WHERE device_id='plc' ORDER BY key")
        .await;
    assert_eq!(before, after, "P-101A's tags are untouched");
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM equipment_instances").await[0][0],
        json!(1)
    );
    // The type can still be changed: no clash was stored.
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["tags"][3]["unit"] = json!("rpm");
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn registers_never_overlap_and_move_only_with_their_instance() {
    let t = T::new().await;
    t.plc("plc").await;
    let (status, body) = t
        .create(json!({"name": "P-201", "type": "pump", "device": "plc", "base": 100}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    // P-202 at base 102 would put its run register (float32, two
    // registers) over P-201's fault register: refused, in the dry run too.
    for dry in [true, false] {
        let (status, resp) = t
            .call(
                "POST",
                &format!("/api/v1/equipment-instances?dry_run={dry}"),
                Some(json!({"name": "P-202", "type": "pump", "device": "plc", "base": 102})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        let e = error_of(&resp);
        assert!(
            e.contains("register 'p202_run'") && e.contains("overlaps register 'p201_fault'"),
            "{e}"
        );
    }
    // One register off is an overlap too (float32 is two registers wide).
    let (status, resp) = t
        .create(json!({"name": "P-203", "type": "pump", "device": "plc", "base": 107}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
    assert!(error_of(&resp).contains("'p201_speed'"), "{resp}");
    let (status, resp) = t
        .create(json!({"name": "P-203", "type": "pump", "device": "plc", "base": 108}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "right after P-201: {resp}");

    // Changing P-201's base moves its own registers; the dry run shows it.
    let moved = json!({"name": "P-201", "type": "pump", "device": "plc", "base": 200});
    let (status, dry) = t
        .call(
            "PUT",
            "/api/v1/equipment-instances/P-201?dry_run=true",
            Some(moved.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    assert_eq!(
        dry["changes"]["registers_moved"],
        json!([
            "plc/p201_run@100->200",
            "plc/p201_fault@102->202",
            "plc/p201_cmd@104->204",
            "plc/p201_speed@106->206"
        ]),
        "{dry}"
    );
    assert_eq!(
        t.sql("SELECT address FROM modbus_registers WHERE name='p201_run'")
            .await[0][0],
        json!(100),
        "the dry run moved nothing"
    );
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-instances/P-201", Some(moved))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let regs = t
        .sql("SELECT name, address FROM modbus_registers WHERE name LIKE 'p201_%' ORDER BY address")
        .await;
    assert_eq!(regs[0], vec![json!("p201_run"), json!(200)]);
    assert_eq!(regs[3], vec![json!("p201_speed"), json!(206)]);

    // Moving onto P-203's registers is refused.
    let (status, resp) = t
        .call(
            "PUT",
            "/api/v1/equipment-instances/P-201",
            Some(json!({"name": "P-201", "type": "pump", "device": "plc", "base": 110})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
    assert!(error_of(&resp).contains("cannot move"), "{resp}");

    // A register of that name someone else put at another address is not
    // taken over: the instance's base must agree with it.
    t.sql(
        "INSERT INTO modbus_registers (device_id, name, address, register_type, scale, sort_order, zero_based)
         VALUES ('plc', 'p301_run', 50, 'float32', 1, 99, 1)",
    )
    .await;
    let (status, resp) = t
        .create(json!({"name": "P-301", "type": "pump", "device": "plc", "base": 300}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
    assert!(
        error_of(&resp).contains("'p301_run' on device 'plc' is at address 50"),
        "{resp}"
    );
    let (status, resp) = t
        .create(json!({"name": "P-301", "type": "pump", "device": "plc", "base": 50}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{resp}");
}

#[tokio::test]
async fn a_widgets_own_settings_survive_saves_and_type_changes() {
    let t = T::new().await;
    t.plc("plc").await;
    t.create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/data-sources",
            Some(json!({"key": "ambient", "name": "Ambient", "device_id": "plc", "min_val": 0, "max_val": 50})),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let extra = t
        .sql("SELECT id FROM data_sources WHERE key='ambient'")
        .await[0][0]
        .clone();
    let uri = "/api/v1/screens/s1/scada-layout";
    // writable: false overrides the type's symbol_config (writable: true);
    // a second tag (not one of the instance's) is the widget's own.
    let mut row = symbol_row("a", "P-1", json!({"writable": false}));
    row["sensor_bindings"] = json!(format!("[{extra}]"));
    let (status, saved) = t
        .call(
            "PUT",
            uri,
            Some(json!({"equipment": [row], "connections": []})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let layout = t.layout("s1").await;
    let c = row_config(&layout, "a");
    assert_eq!(c["writable"], false, "the override survives the save");
    assert_eq!(c["showStatus"], true, "a default the widget lacks is set");
    let run = t
        .sql("SELECT id FROM data_sources WHERE key='p1_run'")
        .await[0][0]
        .clone();
    assert_eq!(
        layout["equipment"][0]["sensor_bindings"],
        json!(format!("[{run},{extra}]")),
        "primary first, the widget's other tag kept"
    );

    // The type changes its defaults: a widget value equal to the old default
    // follows; the override stays.
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["symbol_config"] = json!({"showStatus": false, "writable": true, "showValue": true});
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let layout = t.layout("s1").await;
    let c = row_config(&layout, "a");
    assert_eq!(
        c["writable"], false,
        "the override survives the type update"
    );
    assert_eq!(c["showStatus"], false, "the old default follows the type");
    assert_eq!(c["showValue"], true);
    assert_eq!(
        layout["equipment"][0]["sensor_bindings"],
        json!(format!("[{run},{extra}]"))
    );
}

#[tokio::test]
async fn a_stale_designer_save_is_refused_after_a_type_change() {
    let t = T::new().await;
    t.plc("plc").await;
    t.create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    let uri = "/api/v1/screens/s1/scada-layout";
    let (status, _) = t
        .call(
            "PUT",
            uri,
            Some(json!({"equipment": [symbol_row("a", "P-1", json!({}))], "connections": []})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // The Designer loaded the screen...
    let opened = t.layout("s1").await;
    // ...then someone changes the type, which rebinds the symbol.
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["faceplate"] = Value::Null;
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let now = t.layout("s1").await;
    assert_ne!(now["revision"], opened["revision"]);
    assert!(row_config(&now, "a").get("faceplate").is_none());
    // The Designer's save from the old revision is refused, not merged over.
    let (status, body) = t
        .call(
            "PUT",
            uri,
            Some(json!({"equipment": opened["equipment"], "connections": [], "base_revision": opened["revision"]})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["revision"], now["revision"]);
}

#[tokio::test]
async fn a_users_flow_is_never_overwritten_by_an_instance() {
    let t = T::new().await;
    t.pump_with_flow().await;
    t.plc("plc").await;
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/flows",
            Some(json!({"id": "p1-trip", "name": "Mine", "enabled": false,
                        "trigger_config": json!({"type": "modbus", "device_id": "*"}).to_string(),
                        "nodes": "[]", "created_at": 0, "updated_at": 0})),
        )
        .await;
    assert!(status.is_success(), "{status} {body}");
    let (status, resp) = t
        .create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
    assert!(
        error_of(&resp).contains("flow 'p1-trip' exists and was not made by this instance"),
        "{resp}"
    );
    assert_eq!(
        t.sql("SELECT name FROM flows WHERE id='p1-trip'").await[0][0],
        json!("Mine")
    );
    // With another prefix it is made, and its flow follows the instance.
    let (status, resp) = t
        .create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0, "prefix": "pa"}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{resp}");
    assert_eq!(resp["changes"]["flows"], json!(["pa-trip"]));
}

#[tokio::test]
async fn deleting_an_instance_unbinds_its_widgets_and_deletes_its_flows() {
    let t = T::new().await;
    t.pump_with_flow().await;
    t.plc("plc").await;
    let (status, body) = t
        .create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["changes"]["flows"], json!(["p1-trip"]));
    let uri = "/api/v1/screens/s1/scada-layout";
    let (status, _) = t
        .call(
            "PUT",
            uri,
            Some(json!({"equipment": [symbol_row("a", "P-1", json!({}))], "connections": []})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    let (status, dry) = t
        .call(
            "DELETE",
            "/api/v1/equipment-instances/P-1?dry_run=true",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    assert_eq!(dry["flows_deleted"], json!(["p1-trip"]));
    assert_eq!(
        dry["widgets_unbound"],
        json!([{"screen_id": "s1", "equipment_id": "a"}])
    );
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM equipment_instances").await[0][0],
        json!(1),
        "the dry run deleted nothing"
    );

    let (status, done) = t
        .call("DELETE", "/api/v1/equipment-instances/P-1", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM flows WHERE id='p1-trip'").await[0][0],
        json!(0)
    );
    let c = row_config(&t.layout("s1").await, "a");
    assert!(c.get("equipment").is_none(), "{c}");
    assert_eq!(c["unboundEquipment"], "P-1");
    assert_eq!(
        c["symbol"], "isa_pump_centrifugal",
        "the symbol stays drawn"
    );

    // A new instance of the same name does not take the old widget over.
    let before = t.layout("s1").await;
    let (status, body) = t
        .create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["changes"]["widgets"], json!([]));
    assert_eq!(body["changes"]["flows"], json!(["p1-trip"]));
    assert_eq!(t.layout("s1").await["revision"], before["revision"]);
}

#[tokio::test]
async fn a_new_prefix_removes_the_flows_of_the_old_one() {
    let t = T::new().await;
    t.pump_with_flow().await;
    t.plc("plc").await;
    t.create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    let body =
        json!({"name": "P-1", "type": "pump", "device": "plc", "base": 20, "prefix": "pump1"});
    let (status, dry) = t
        .call(
            "PUT",
            "/api/v1/equipment-instances/P-1?dry_run=true",
            Some(body.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    assert_eq!(dry["changes"]["flows"], json!(["pump1-trip"]));
    assert_eq!(dry["changes"]["flows_removed"], json!(["p1-trip"]));
    let (status, done) = t
        .call("PUT", "/api/v1/equipment-instances/P-1", Some(body))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let flows = t.sql("SELECT id FROM flows ORDER BY id").await;
    assert_eq!(flows, vec![vec![json!("pump1-trip")]]);
}

#[tokio::test]
async fn of_two_concurrent_creates_one_wins_and_the_other_gets_409() {
    let t = T::new().await;
    t.plc("plc").await;
    let body = json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0});
    let (a, b) = tokio::join!(t.create(body.clone()), t.create(body.clone()));
    let mut statuses = [a.0, b.0];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::CONFLICT],
        "{} / {}",
        a.1,
        b.1
    );
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM modbus_registers").await[0][0],
        json!(4)
    );
}

#[tokio::test]
async fn equipment_writes_take_a_snapshot_that_rolls_them_back() {
    let t = T::new().await;
    t.plc("plc").await;
    let (status, body) = t
        .create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let snaps = t.state.storage.list_config_snapshots(10).await.unwrap();
    let pre = snaps
        .iter()
        .find(|s| s.label.as_deref() == Some("pre-equipment"))
        .expect("a snapshot before the write");
    let data: Value = serde_json::from_str(&pre.snapshot_data).unwrap();
    for table in [
        "equipment_types",
        "equipment_instances",
        "modbus_registers",
        "scada_equipment",
    ] {
        assert!(data.get(table).is_some(), "{table} is in the snapshot");
    }
    assert!(t.state.storage.rollback_to_snapshot(pre.id).await.unwrap());
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM equipment_instances").await[0][0],
        json!(0)
    );
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM modbus_registers").await[0][0],
        json!(0)
    );
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM modbus_devices").await[0][0],
        json!(1),
        "the device itself was there before"
    );
}

#[tokio::test]
async fn a_type_change_skips_a_broken_instance_with_a_warning() {
    let t = T::new().await;
    t.plc("plc").await;
    t.plc("gone").await;
    for (n, dev) in [("P-1", "plc"), ("P-2", "gone")] {
        let (status, body) = t
            .create(json!({"name": n, "type": "pump", "device": dev, "base": 0}))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    t.sql("DELETE FROM modbus_devices WHERE id='gone'").await;
    t.sql("DELETE FROM data_sources WHERE device_id='gone'")
        .await;
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["tags"][3]["unit"] = json!("rpm");
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let warnings = body["warnings"].to_string();
    assert!(
        warnings.contains("P-2") && warnings.contains("skipped"),
        "{body}"
    );
    assert_eq!(
        t.sql("SELECT unit FROM data_sources WHERE key='p1_speed'")
            .await[0][0],
        json!("rpm")
    );
}

#[tokio::test]
async fn a_faceplate_a_type_opens_cannot_be_deleted() {
    let t = T::new().await;
    let (status, body) = t.call("DELETE", "/api/v1/faceplates/pump", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(error_of(&body).contains("pump"), "{body}");
    // `library` is a route, not a type id.
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["id"] = json!("library");
    let (status, body) = t.call("POST", "/api/v1/equipment-types", Some(pump)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("reserved"), "{body}");
}

#[tokio::test]
async fn viewers_and_operators_cannot_change_equipment() {
    let t = T::new().await;
    t.plc("plc").await;
    t.create(json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0}))
        .await;
    for (name, role) in [("olivia", "operator"), ("victor", "viewer")] {
        let hash = crate::api::auth::hash_password("password-123");
        t.state
            .storage
            .create_user(name, &hash, "", role)
            .await
            .unwrap();
        let cookie = format!(
            "voltrus_token={}",
            login_cookie(&t.app, name, "password-123").await
        );
        let (_, pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
        let inst = json!({"name": "P-1", "type": "pump", "device": "plc", "base": 0});
        let mut other = pump.clone();
        other["id"] = json!("pump2");
        for (method, uri, body) in [
            ("POST", "/api/v1/equipment-types", Some(other)),
            ("PUT", "/api/v1/equipment-types/pump", Some(pump.clone())),
            ("DELETE", "/api/v1/equipment-types/pump", None),
            (
                "POST",
                "/api/v1/equipment-types/library/install",
                Some(json!({})),
            ),
            (
                "POST",
                "/api/v1/equipment-instances",
                Some(json!({"name": "P-9", "type": "pump", "device": "plc", "base": 50})),
            ),
            ("PUT", "/api/v1/equipment-instances/P-1", Some(inst)),
            ("DELETE", "/api/v1/equipment-instances/P-1", None),
        ] {
            let (status, resp) = send_request(&t.app, method, uri, None, Some(&cookie), body).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{role} {method} {uri}: {resp}"
            );
        }
        // Reading is open to them.
        let (status, _) = send_request(
            &t.app,
            "GET",
            "/api/v1/equipment-instances",
            None,
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
    assert_eq!(
        t.sql("SELECT COUNT(*) FROM equipment_instances").await[0][0],
        json!(1)
    );
}

#[tokio::test]
async fn starter_setpoints_are_limited_to_their_tag_range_and_need_a_reason() {
    let t = T::new().await;
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/equipment-types/library/install",
            Some(json!({"ids": ["analog", "dosing_pump"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    t.plc("plc").await;
    for body in [
        json!({"name": "AT-1", "type": "analog", "device": "plc", "base": 0,
               "overrides": {"sp": {"min": 2, "max": 12}}}),
        json!({"name": "P-1", "type": "dosing_pump", "device": "plc", "base": 10}),
    ] {
        let (status, resp) = t.create(body).await;
        assert_eq!(status, StatusCode::CREATED, "{resp}");
    }
    let rows = t
        .sql(
            "SELECT key, writable, write_min, write_max, write_requires_reason FROM data_sources
             WHERE key IN ('at1_sp', 'p1_dose_sp') ORDER BY key",
        )
        .await;
    assert_eq!(
        rows,
        vec![
            vec![json!("at1_sp"), json!(1), json!(2.0), json!(12.0), json!(1)],
            vec![
                json!("p1_dose_sp"),
                json!(1),
                json!(0.0),
                json!(100.0),
                json!(1)
            ],
        ]
    );
}
