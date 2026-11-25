//! A 2D SCADA screen built through the REST API loads in the editor's form
//! and saves back unchanged: what the routes return can be sent back as is,
//! and the project export of it re-imports to the same rows and the same text.
//! (The MCP path is `mcp_screen_from_a_project_import_round_trips` in mcp.rs;
//! the client's load/save is `client/tests/scadaRoundTrip.test.ts`.)

use super::*;
use http::StatusCode;
use serde_json::{Value, json};

/// Numbers as f64, so 120 and 120.0 compare equal (columns are REAL).
fn floats(v: Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.into_iter().map(floats).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, floats(v))).collect()),
        v => v,
    }
}

/// Rows as the routes return them, with `config` and `sensor_bindings` parsed
/// so that JSON key order does not matter.
pub(crate) fn canonical(items: &Value) -> Vec<Value> {
    let mut rows: Vec<Value> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let mut r = r.clone();
            for k in ["config", "sensor_bindings", "waypoints"] {
                if let Some(Value::String(s)) = r.get(k) {
                    r[k] = serde_json::from_str(s).unwrap_or(Value::String(s.clone()));
                }
            }
            floats(r)
        })
        .collect();
    rows.sort_by_key(|r| {
        r["equipment_id"]
            .as_str()
            .or(r["connection_id"].as_str())
            .unwrap_or_default()
            .to_string()
    });
    rows
}

#[tokio::test]
async fn scada_screen_made_over_rest_saves_back_identically() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let req = |method: &'static str, uri: String, body: Option<Value>| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move { send_request(&app, method, &uri, None, Some(&cookie), body).await }
    };

    let mut tags = Vec::new();
    for key in ["p301a_run", "p301a_cmd", "outlet_pressure"] {
        let (status, t) = req(
            "POST",
            "/api/v1/data-sources".into(),
            Some(json!({"key": key, "name": key, "device_id": "plc_1"})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{t}");
        tags.push(t["id"].as_i64().unwrap());
    }
    let (_, screen) = req(
        "POST",
        "/api/v1/screens".into(),
        Some(json!({"name": "Pump station"})),
    )
    .await;
    let screen = screen["id"].as_str().unwrap().to_string();

    // The shapes the editor's save writes: config and bindings as JSON text,
    // stacking order in config.z, rotation in the column and in config.
    let equipment = json!({"items": [
        {"equipment_id": "w_pump", "equipment_type": "symbol", "equipment_name": "P-301A",
         "x": 120, "y": 80, "width": 80, "height": 80, "rotation": 90,
         "sensor_bindings": format!("[{}]", tags[0]),
         "config": json!({"symbol": "pump", "rotation": 90, "label": "P-301A", "z": 0,
            "faceplateMap": {"run": tags[0], "cmd": tags[1]}}).to_string()},
        {"equipment_id": "w_pt", "equipment_type": "big_number", "equipment_name": "Outlet",
         "x": 300, "y": 80, "width": 140, "height": 70, "rotation": 0,
         "sensor_bindings": format!("[{}]", tags[2]),
         "config": json!({"decimals": 2, "z": 1, "bindings": {
            "label": {"kind": "expr", "expr": "\"Out \" + text({plc_1/outlet_pressure}, 1)"},
            "visible": {"kind": "tag", "tag": "plc_1/p301a_run"},
            "color": {"kind": "map", "expr": "{plc_1/p301a_run}", "cases": [{"op": ">=", "v": 1, "out": "#22c55e"}], "else": "#8b939e"}
         }}).to_string()}
    ]});
    let connections = json!({"items": [
        {"connection_id": "c1", "from_equipment": "w_pump", "to_equipment": "w_pt",
         "connection_type": "flow", "waypoints": "[[200,120],[300,120]]"}
    ]});
    let (status, body) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        Some(equipment.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-connections"),
        Some(connections.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, eq1) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        None,
    )
    .await;
    let (_, cn1) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-connections"),
        None,
    )
    .await;
    // Everything sent is read back: no field dropped or rewritten.
    let sent = canonical(&equipment["items"]);
    let got = canonical(&eq1["items"]);
    assert_eq!(got.len(), sent.len());
    for (g, s) in got.iter().zip(&sent) {
        for (k, v) in s.as_object().unwrap() {
            assert_eq!(&g[k], v, "{k} of {}", s["equipment_id"]);
        }
    }
    let got = canonical(&cn1["items"]);
    for (k, v) in canonical(&connections["items"])[0].as_object().unwrap() {
        assert_eq!(&got[0][k], v, "{k}");
    }

    // The editor saves what it loaded: the rows do not change.
    let (status, _) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        Some(json!({"items": eq1["items"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-connections"),
        Some(json!({"items": cn1["items"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, eq2) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        None,
    )
    .await;
    let (_, cn2) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-connections"),
        None,
    )
    .await;
    assert_eq!(canonical(&eq2["items"]), canonical(&eq1["items"]));
    assert_eq!(canonical(&cn2["items"]), canonical(&cn1["items"]));

    // Project text: export, re-import (replace), export again: same text and rows.
    let doc = crate::project_yaml::export_project_yaml(&storage)
        .await
        .unwrap();
    let text = crate::project_yaml::to_yaml(&doc);
    assert!(text.contains("faceplateMap:"), "{text}");
    assert!(
        text.contains("run: plc_1/p301a_run"),
        "faceplate tags are written by ref: {text}"
    );
    assert!(
        text.contains("plc_1/outlet_pressure"),
        "property bindings are exported by ref: {text}"
    );
    assert!(
        !doc.warnings.iter().any(|w| w.contains("binding")),
        "no binding warnings for known refs: {:?}",
        doc.warnings
    );
    let reparsed = crate::project_yaml::parse_project_yaml(&text).unwrap();
    crate::project_yaml::apply_import(&storage, &reparsed, crate::project_yaml::Mode::Replace)
        .await
        .unwrap();
    let again = crate::project_yaml::export_project_yaml(&storage)
        .await
        .unwrap();
    assert_eq!(crate::project_yaml::to_yaml(&again), text);
    let (_, eq3) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        None,
    )
    .await;
    let (_, cn3) = req(
        "GET",
        format!("/api/v1/screens/{screen}/scada-connections"),
        None,
    )
    .await;
    assert_eq!(canonical(&eq3["items"]), canonical(&eq1["items"]));
    assert_eq!(canonical(&cn3["items"]), canonical(&cn1["items"]));
}
