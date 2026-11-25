//! Equipment types and instances: CRUD, the starter library, one-line
//! instantiation (binding by key pattern, creating registers from a base),
//! symbols bound on save, and a changed type reaching every instance.

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
        T { app, cookie, state }
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
    async fn tag_id(&self, device: &str, key: &str) -> i64 {
        self.sql(&format!(
            "SELECT id FROM data_sources WHERE device_id='{device}' AND key='{key}'"
        ))
        .await[0][0]
            .as_i64()
            .unwrap()
    }
    /// A Modbus device `wtp` with the WTP showcase's pump registers and tags.
    async fn wtp(&self) {
        let keys = ["p101a_run", "p101b_run", "p101b_fault"];
        let regs: Vec<Value> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| json!({"name": k, "offset": 4 + 2 * i, "register_type": "float32"}))
            .collect();
        let (status, body) = self
            .call(
                "POST",
                "/api/v1/modbus-devices",
                Some(json!({"id": "wtp", "name": "WTP PLC", "ip": "127.0.0.1", "port": 1, "registers": regs})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        for k in keys {
            let (status, body) = self
                .call(
                    "POST",
                    "/api/v1/data-sources",
                    Some(json!({"key": k, "name": k, "device_id": "wtp", "source_type": "modbus", "min_val": 0, "max_val": 1})),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
        }
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

#[tokio::test]
async fn types_are_created_listed_changed_and_deleted() {
    let t = T::new().await;

    // The starter library: seven types; installing twice skips the second time.
    let (status, lib) = t.call("GET", "/api/v1/equipment-types/library", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(lib.as_array().unwrap().len(), 7);
    assert_eq!(lib[0]["type"]["id"], "pump");
    assert_eq!(lib[0]["faceplate"]["id"], "pump");
    let (status, r) = t
        .call(
            "POST",
            "/api/v1/equipment-types/library/install",
            Some(json!({"ids": ["pump", "valve"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["installed"], json!(["pump", "valve"]));
    let (_, r) = t
        .call(
            "POST",
            "/api/v1/equipment-types/library/install",
            Some(json!({})),
        )
        .await;
    assert_eq!(r["skipped"], json!(["pump", "valve"]), "{r}");
    assert_eq!(r["installed"].as_array().unwrap().len(), 5);
    let (_, fp) = t.call("GET", "/api/v1/faceplates/dosing_pump", None).await;
    assert_eq!(fp["kind"], "pump", "the faceplates come along");
    let (status, _) = t
        .call(
            "POST",
            "/api/v1/equipment-types/library/install",
            Some(json!({"ids": ["teapot"]})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (_, list) = t.call("GET", "/api/v1/equipment-types", None).await;
    assert_eq!(list.as_array().unwrap().len(), 7);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|x| x["instance_count"] == 0)
    );

    // A new type; the same id again is a conflict.
    let mut blower = json!({
        "id": "blower", "name": "Air blower", "category": "Pumps", "symbol": "isa_blower",
        "faceplate": "pump",
        "tags": [{"role": "run", "kind": "state", "required": true}],
        "bindings": {"primary": "run"}
    });
    let (status, body) = t
        .call("POST", "/api/v1/equipment-types", Some(blower.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        body["tags"][0]["key"], "{prefix}_{role}",
        "the default key pattern"
    );
    let (status, _) = t
        .call("POST", "/api/v1/equipment-types", Some(blower.clone()))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Problems are named by field.
    let mut bad = blower.clone();
    bad["id"] = json!("fan");
    bad["tags"][0]["kind"] = json!("lamp");
    bad["bindings"]["fault"] = json!("trip");
    let (status, body) = t.call("POST", "/api/v1/equipment-types", Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let e = body["error"].as_str().unwrap();
    assert!(
        e.contains("tags[0].kind 'lamp'") && e.contains("bindings.fault"),
        "{e}"
    );
    let mut bad = blower.clone();
    bad["id"] = json!("fan");
    bad["faceplate"] = json!("nope");
    let (status, body) = t.call("POST", "/api/v1/equipment-types", Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("unknown faceplate 'nope'"),
        "{body}"
    );
    let mut bad = blower.clone();
    bad["colour"] = json!("red");
    let (status, body) = t.call("POST", "/api/v1/equipment-types", Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("colour"), "{body}");

    // Read-modify-write.
    let (_, mut got) = t.call("GET", "/api/v1/equipment-types/blower", None).await;
    got["description"] = json!("Aeration");
    let (status, body) = t
        .call("PUT", "/api/v1/equipment-types/blower", Some(got))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["affected"], json!([]));
    let (_, got) = t.call("GET", "/api/v1/equipment-types/blower", None).await;
    assert_eq!(got["description"], "Aeration");
    blower["id"] = json!("other");
    let (status, _) = t
        .call("PUT", "/api/v1/equipment-types/blower", Some(blower))
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = t.call("GET", "/api/v1/equipment-types/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = t
        .call("DELETE", "/api/v1/equipment-types/blower", None)
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = t
        .call("DELETE", "/api/v1/equipment-types/blower", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_instance_is_one_line() {
    let t = T::new().await;
    t.wtp().await;
    t.call(
        "POST",
        "/api/v1/equipment-types/library/install",
        Some(json!({"ids": ["pump"]})),
    )
    .await;

    // Dry run: what would be bound, nothing written.
    let p101b = json!({"name": "P-101B", "type": "pump", "device": "wtp"});
    let (status, dry) = t
        .call(
            "POST",
            "/api/v1/equipment-instances?dry_run=true",
            Some(p101b.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    assert_eq!(dry["dry_run"], true);
    let tags: Vec<(String, String)> = dry["changes"]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| {
            (
                x["ref"].as_str().unwrap().into(),
                x["action"].as_str().unwrap().into(),
            )
        })
        .collect();
    assert!(
        tags.contains(&("wtp/p101b_fault".into(), "update".into())),
        "{dry}"
    );
    let (status, _) = t
        .call("GET", "/api/v1/equipment-instances/P-101B", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a dry run writes nothing");
    let warn = t
        .sql("SELECT crit_val FROM data_sources WHERE key='p101b_fault'")
        .await;
    assert_eq!(warn[0][0], json!(0.0));

    let (status, made) = t
        .call("POST", "/api/v1/equipment-instances", Some(p101b.clone()))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let roles = &made["resolved"]["roles"];
    let status_of = |role: &str| {
        roles
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["role"] == role)
            .unwrap()["status"]
            .clone()
    };
    assert_eq!(status_of("run"), "bound");
    assert_eq!(status_of("fault"), "bound");
    assert_eq!(status_of("cmd"), "fallback");
    assert_eq!(status_of("speed"), "missing");
    let run = t.tag_id("wtp", "p101b_run").await;
    let fault = t.tag_id("wtp", "p101b_fault").await;
    let w = &made["resolved"]["widget"];
    assert_eq!(w["sensor_id"], run);
    assert_eq!(w["config"]["faultSensorId"], fault);
    assert_eq!(
        w["config"]["faceplateMap"],
        json!({"run": run, "fault": fault, "cmd": run})
    );
    assert_eq!(w["config"]["symbol"], "isa_pump_centrifugal");
    // The type's alarm reached the tag.
    let crit = t
        .sql("SELECT crit_val, decimals FROM data_sources WHERE key='p101b_fault'")
        .await;
    assert_eq!(crit[0], vec![json!(1.0), json!(0)]);

    let (status, _) = t
        .call("POST", "/api/v1/equipment-instances", Some(p101b))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A required role with no tag is refused, naming it.
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/equipment-instances",
            Some(json!({"name": "P-999", "type": "pump", "device": "wtp"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let e = body["error"].as_str().unwrap();
    assert!(
        e.contains("required role 'run'") && e.contains("p999_run"),
        "{e}"
    );
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/equipment-instances",
            Some(json!({"name": "P-1", "type": "nope", "device": "wtp"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = t
        .call(
            "POST",
            "/api/v1/equipment-instances",
            Some(json!({"name": "P-1", "type": "pump", "device": "ghost"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("ghost"), "{body}");

    // With a base on a Modbus device the registers and tags are made.
    t.call(
        "POST",
        "/api/v1/modbus-devices",
        Some(json!({"id": "plc2", "name": "PLC 2", "ip": "127.0.0.1", "port": 1, "registers": []})),
    )
    .await;
    let (status, made) = t
        .call(
            "POST",
            "/api/v1/equipment-instances",
            Some(
                json!({"name": "P-201", "type": "pump", "device": "plc2", "base": 100,
                        "overrides": {"speed": {"unit": "rpm", "max": 1450}}}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(
        made["changes"]["registers_added"].as_array().unwrap().len(),
        4,
        "{made}"
    );
    let regs = t
        .sql("SELECT name, address FROM modbus_registers WHERE device_id='plc2' ORDER BY address")
        .await;
    assert_eq!(
        regs,
        vec![
            vec![json!("p201_run"), json!(100)],
            vec![json!("p201_fault"), json!(102)],
            vec![json!("p201_cmd"), json!(104)],
            vec![json!("p201_speed"), json!(106)],
        ]
    );
    let speed = t
        .sql("SELECT name, unit, max_val, source_type FROM data_sources WHERE device_id='plc2' AND key='p201_speed'")
        .await;
    assert_eq!(
        speed[0],
        vec![
            json!("P-201 Speed"),
            json!("rpm"),
            json!(1450.0),
            json!("modbus")
        ]
    );
    let cmd = t
        .sql("SELECT writable FROM data_sources WHERE key='p201_cmd'")
        .await;
    assert_eq!(cmd[0][0], json!(1), "the type's write policy");

    let (_, list) = t
        .call("GET", "/api/v1/equipment-instances?type=pump", None)
        .await;
    assert_eq!(list.as_array().unwrap().len(), 2);
    let (_, types) = t.call("GET", "/api/v1/equipment-types", None).await;
    assert_eq!(types[0]["instance_count"], 2);

    // Changing an instance: skip the fault role.
    let (status, body) = t
        .call(
            "PUT",
            "/api/v1/equipment-instances/P-101B",
            Some(json!({"name": "P-101B", "type": "pump", "device": "wtp", "overrides": {"fault": {"skip": true}}})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body["resolved"]["widget"]["remove"]
            .as_array()
            .unwrap()
            .contains(&json!("faultSensorId"))
    );

    let (status, _) = t.call("DELETE", "/api/v1/equipment-types/pump", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "instances keep their type");
    for n in ["P-101B", "P-201"] {
        let (status, _) = t
            .call("DELETE", &format!("/api/v1/equipment-instances/{n}"), None)
            .await;
        assert_eq!(status, StatusCode::OK);
    }
    // The tags stay.
    assert!(t.tag_id("plc2", "p201_run").await > 0);
    let (status, _) = t.call("DELETE", "/api/v1/equipment-types/pump", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn symbols_bind_on_save_and_follow_a_changed_type() {
    let t = T::new().await;
    t.wtp().await;
    t.call(
        "POST",
        "/api/v1/equipment-types/library/install",
        Some(json!({"ids": ["pump"]})),
    )
    .await;
    for n in ["P-101A", "P-101B"] {
        let (status, body) = t
            .call(
                "POST",
                "/api/v1/equipment-instances",
                Some(json!({"name": n, "type": "pump", "device": "wtp"})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    let run_a = t.tag_id("wtp", "p101a_run").await;
    let run_b = t.tag_id("wtp", "p101b_run").await;
    let fault_b = t.tag_id("wtp", "p101b_fault").await;

    // A symbol saved with only `config.equipment` is bound by the server.
    let uri = "/api/v1/screens/s1/scada-layout";
    let (_, empty) = t.call("GET", uri, None).await;
    let (status, saved) = t
        .call(
            "PUT",
            uri,
            Some(json!({
                "equipment": [symbol_row("a", "P-101A", json!({})), symbol_row("b", "P-101B", json!({"label": "Intake B"})),
                              symbol_row("c", "P-404", json!({}))],
                "connections": [],
                "base_revision": empty["revision"],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let (_, layout) = t.call("GET", uri, None).await;
    assert_eq!(layout["revision"], saved["revision"]);
    let row = |id: &str| {
        layout["equipment"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["equipment_id"] == id)
            .unwrap()
            .clone()
    };
    let cfg = |r: &Value| serde_json::from_str::<Value>(r["config"].as_str().unwrap()).unwrap();
    assert_eq!(row("a")["sensor_bindings"], json!(format!("[{run_a}]")));
    let b = cfg(&row("b"));
    assert_eq!(b["label"], "Intake B");
    assert_eq!(b["faultSensorId"], fault_b);
    assert_eq!(b["faceplateMap"]["cmd"], run_b);
    assert_eq!(b["writable"], true);
    assert_eq!(cfg(&row("a"))["label"], "P-101A");
    assert!(cfg(&row("a")).get("faultSensorId").is_none());
    // An unknown instance is stored as it is.
    assert_eq!(row("c")["sensor_bindings"], "[]");
    assert_eq!(cfg(&row("c"))["equipment"], "P-404");

    // Saving the bound rows back changes nothing: same revision.
    let (status, again) = t
        .call(
            "PUT",
            uri,
            Some(json!({"equipment": layout["equipment"], "connections": [], "base_revision": layout["revision"]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["revision"], layout["revision"]);
    // The per-table route binds too.
    let (status, _) = t
        .call(
            "PUT",
            "/api/v1/screens/s2/scada-equipment",
            Some(json!({"items": [symbol_row("x", "P-101B", json!({}))]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (_, s2) = t
        .call("GET", "/api/v1/screens/s2/scada-equipment", None)
        .await;
    assert_eq!(
        s2["items"][0]["sensor_bindings"],
        json!(format!("[{run_b}]"))
    );

    // Change the type: a warning limit on the fault tag, a value badge on the symbols.
    let (_, mut pump) = t.call("GET", "/api/v1/equipment-types/pump", None).await;
    pump["tags"][1]["alarm"] = json!({"warn": 0.5, "crit": 1});
    pump["tags"][0]["name"] = json!("{name} running");
    pump["symbol_config"]["showValue"] = json!(true);
    let (status, dry) = t
        .call(
            "PUT",
            "/api/v1/equipment-types/pump?dry_run=true",
            Some(pump.clone()),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{dry}");
    let affected = dry["affected"].as_array().unwrap();
    assert_eq!(affected.len(), 2, "{dry}");
    let b = affected.iter().find(|a| a["instance"] == "P-101B").unwrap();
    let fault = b["tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["key"] == "p101b_fault")
        .unwrap();
    assert_eq!(fault["action"], "update");
    assert_eq!(fault["fields"], json!(["alarm"]));
    let widgets: Vec<String> = b["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            format!(
                "{}/{}",
                w["screen_id"].as_str().unwrap(),
                w["equipment_id"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(widgets, vec!["s1/b", "s2/x"]);
    let warn = t
        .sql("SELECT warn_val FROM data_sources WHERE key='p101b_fault'")
        .await;
    assert_eq!(warn[0][0], json!(0.0), "the dry run wrote nothing");

    let (status, done) = t
        .call("PUT", "/api/v1/equipment-types/pump", Some(pump))
        .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    let warn = t
        .sql("SELECT warn_val FROM data_sources WHERE key='p101b_fault'")
        .await;
    assert_eq!(warn[0][0], json!(0.5));
    let names = t
        .sql("SELECT name FROM data_sources WHERE key IN ('p101a_run','p101b_run') ORDER BY key")
        .await;
    assert_eq!(
        names,
        vec![vec![json!("P-101A running")], vec![json!("P-101B running")]]
    );
    let (_, layout) = t.call("GET", uri, None).await;
    let row = layout["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["equipment_id"] == "a")
        .unwrap()
        .clone();
    assert_eq!(cfg(&row)["showValue"], true, "the symbols follow the type");
    assert_eq!(cfg(&row)["z"], 1, "and keep their own config");
}
