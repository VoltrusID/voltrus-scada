//! A 2D screen's equipment and connections saved together with a revision
//! check (the Designer's save), the old per-table routes taking the same
//! check, and property-binding refs following a tag rename.

use super::*;
use http::StatusCode;
use serde_json::{Value, json};

async fn app() -> (axum::Router, String) {
    let state = seeded_admin_app_state().await;
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie)
}

async fn call(
    app: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send_request(app, method, uri, None, Some(cookie), body).await
}

fn eq(id: &str, x: f64, config: Value) -> Value {
    json!({"equipment_id": id, "equipment_type": "label", "equipment_name": "", "x": x, "y": 0,
           "width": 100, "height": 40, "rotation": 0, "sensor_bindings": "[]", "config": config.to_string()})
}

fn conn(id: &str, from: &str, to: &str) -> Value {
    json!({"connection_id": id, "from_equipment": from, "to_equipment": to, "connection_type": "flow", "waypoints": "[]"})
}

#[tokio::test]
async fn layout_save_checks_the_base_revision() {
    let (app, cookie) = app().await;
    let uri = "/api/v1/screens/s1/scada-layout";

    let (status, empty) = call(&app, &cookie, "GET", uri, None).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    let rev0 = empty["revision"].as_str().unwrap().to_string();
    assert_eq!(rev0.len(), 64);
    assert_eq!(empty["equipment"], json!([]));

    // Both tables in one save, from the revision read.
    let (status, saved) = call(
        &app,
        &cookie,
        "PUT",
        uri,
        Some(json!({
            "equipment": [eq("a", 0.0, json!({})), eq("b", 200.0, json!({}))],
            "connections": [conn("c1", "a", "b")],
            "base_revision": rev0,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    let rev1 = saved["revision"].as_str().unwrap().to_string();
    assert_ne!(rev1, rev0);
    let (_, got) = call(&app, &cookie, "GET", uri, None).await;
    assert_eq!(got["revision"], rev1);
    assert_eq!(got["equipment"].as_array().unwrap().len(), 2);
    assert_eq!(got["connections"][0]["connection_id"], "c1");
    // The per-table GETs see the same rows.
    let (_, old) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s1/scada-equipment",
        None,
    )
    .await;
    assert_eq!(old["items"], got["equipment"]);

    // A save from the stale revision is refused and writes nothing.
    let (status, conflict) = call(
        &app,
        &cookie,
        "PUT",
        uri,
        Some(json!({
            "equipment": [], "connections": [], "base_revision": rev0,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["code"], "conflict");
    assert_eq!(conflict["revision"], rev1);
    assert!(conflict["error"].as_str().unwrap().contains("changed"));
    let (_, after) = call(&app, &cookie, "GET", uri, None).await;
    assert_eq!(after, got);

    // Without a base revision it overwrites (an explicit "save anyway").
    let (status, forced) = call(
        &app,
        &cookie,
        "PUT",
        uri,
        Some(json!({
            "equipment": [eq("a", 10.0, json!({}))], "connections": [],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forced}");
    let (_, after) = call(&app, &cookie, "GET", uri, None).await;
    assert_eq!(after["revision"], forced["revision"]);
    assert_eq!(after["equipment"].as_array().unwrap().len(), 1);
    assert_eq!(after["connections"], json!([]));

    // Bad bodies are 400, not 500.
    let (status, _) = call(&app, &cookie, "PUT", uri, Some(json!({"equipment": []}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        &cookie,
        "PUT",
        uri,
        Some(json!({"equipment": [], "connections": [], "base_revision": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The overview screen id works like any other.
    let (status, _) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/__overview/scada-layout",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn per_table_saves_take_an_optional_base_revision() {
    let (app, cookie) = app().await;
    let (_, l) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s2/scada-layout",
        None,
    )
    .await;
    let rev0 = l["revision"].as_str().unwrap().to_string();

    // Without base_revision: as before, plus the new revision in the answer.
    let (status, r) = call(
        &app,
        &cookie,
        "PUT",
        "/api/v1/screens/s2/scada-equipment",
        Some(json!({"items": [eq("a", 0.0, json!({})), eq("b", 0.0, json!({}))]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["ok"], true);
    let rev1 = r["revision"].as_str().unwrap().to_string();

    // With a stale one: 409 from either table, nothing written.
    let (status, c) = call(
        &app,
        &cookie,
        "PUT",
        "/api/v1/screens/s2/scada-connections",
        Some(json!({"items": [conn("c", "a", "b")], "base_revision": rev0})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{c}");
    assert_eq!(c["revision"], rev1);
    let (status, _) = call(
        &app,
        &cookie,
        "PUT",
        "/api/v1/screens/s2/scada-equipment",
        Some(json!({"items": [], "base_revision": rev0})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, l) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s2/scada-layout",
        None,
    )
    .await;
    assert_eq!(l["revision"], rev1);
    assert_eq!(l["equipment"].as_array().unwrap().len(), 2);

    // With the current one: saved.
    let (status, r) = call(
        &app,
        &cookie,
        "PUT",
        "/api/v1/screens/s2/scada-connections",
        Some(json!({"items": [conn("c", "a", "b")], "base_revision": rev1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{r}");
    let (_, l) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s2/scada-layout",
        None,
    )
    .await;
    assert_eq!(l["revision"], r["revision"]);
}

#[tokio::test]
async fn renaming_a_tag_rewrites_the_binding_refs_that_name_it() {
    let (app, cookie) = app().await;
    let mut ids = Vec::new();
    for key in ["level", "flow"] {
        let (status, t) = call(
            &app,
            &cookie,
            "POST",
            "/api/v1/data-sources",
            Some(json!({"key": key, "name": key, "device_id": "plc_1"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{t}");
        ids.push(t["id"].as_i64().unwrap());
    }
    let bindings = json!({"bindings": {
        "visible": {"kind": "tag", "tag": "plc_1/level"},
        "label": {"kind": "expr", "expr": "text({plc_1/level}, 1) + \" {plc_1/level} \" + text({plc_1/flow})"},
        "color": {"kind": "map", "expr": "{ plc_1/level }", "cases": [{"op": ">=", "v": 1, "out": "#f00"}], "else": "#0f0"},
    }});
    let (status, _) = call(&app, &cookie, "PUT", "/api/v1/screens/s3/scada-layout", Some(json!({
        "equipment": [eq("w", 0.0, bindings), eq("plain", 0.0, json!({"label": "x"}))], "connections": [],
    }))).await;
    assert_eq!(status, StatusCode::OK);

    let (status, r) = call(
        &app,
        &cookie,
        "PUT",
        &format!("/api/v1/data-sources/{}", ids[0]),
        Some(json!({"key": "tank_level"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{r}");

    let (_, l) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s3/scada-layout",
        None,
    )
    .await;
    let w = l["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "w")
        .unwrap();
    let config: Value = serde_json::from_str(w["config"].as_str().unwrap()).unwrap();
    let b = &config["bindings"];
    assert_eq!(b["visible"]["tag"], "plc_1/tank_level");
    // Refs are rewritten; the same text inside a string literal is not a ref and stays.
    assert_eq!(
        b["label"]["expr"],
        "text({plc_1/tank_level}, 1) + \" {plc_1/level} \" + text({plc_1/flow})"
    );
    assert_eq!(b["color"]["expr"], "{plc_1/tank_level}");
    assert_eq!(b["color"]["cases"][0]["out"], "#f00");

    // Moving the tag to another device is a rename too.
    let (status, _) = call(
        &app,
        &cookie,
        "PUT",
        &format!("/api/v1/data-sources/{}", ids[1]),
        Some(json!({"device_id": "plc_2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, l) = call(
        &app,
        &cookie,
        "GET",
        "/api/v1/screens/s3/scada-layout",
        None,
    )
    .await;
    let w = l["equipment"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["equipment_id"] == "w")
        .unwrap();
    let config: Value = serde_json::from_str(w["config"].as_str().unwrap()).unwrap();
    assert!(
        config["bindings"]["label"]["expr"]
            .as_str()
            .unwrap()
            .ends_with("text({plc_2/flow})")
    );
}

#[tokio::test]
async fn export_warns_about_bindings_to_unknown_tags() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, _) = call(&app, &cookie, "PUT", "/api/v1/screens/s4/scada-layout", Some(json!({
        "equipment": [eq("w", 0.0, json!({"bindings": {"label": {"kind": "expr", "expr": "text({ghost/tag})"}}}))],
        "connections": [],
    }))).await;
    assert_eq!(status, StatusCode::OK);
    let doc = crate::project_yaml::export_project_yaml(&storage)
        .await
        .unwrap();
    assert!(
        doc.warnings
            .iter()
            .any(|w| w.contains("binding 'label' reads unknown tag 'ghost/tag'")),
        "{:?}",
        doc.warnings
    );
}
