//! Faceplate definitions are stored once; SCADA symbols open them with their
//! own tag per role, and a definition in use cannot be deleted.

use super::*;
use http::StatusCode;
use serde_json::json;

fn pump() -> serde_json::Value {
    json!({
        "id": "pump_std", "name": "Centrifugal pump", "kind": "pump",
        "roles": [
            {"role": "run", "label": "Running", "kind": "state", "required": true},
            {"role": "fault", "kind": "fault"},
            {"role": "cmd", "label": "Start / stop", "kind": "command"},
            {"role": "speed", "kind": "analog"}
        ]
    })
}

#[tokio::test]
async fn faceplates_are_defined_once_and_opened_by_many_symbols() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let req = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move { send_request(&app, method, &uri, None, Some(&cookie), body).await }
    };

    let (status, body) = req("POST", "/api/v1/faceplates".into(), Some(pump())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, _) = req("POST", "/api/v1/faceplates".into(), Some(pump())).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Unknown fields and values outside the lists are named.
    let mut bad = pump();
    bad["id"] = json!("valve_std");
    bad["roles"][0]["kind"] = json!("lamp");
    let (status, body) = req("POST", "/api/v1/faceplates".into(), Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("roles[0].kind 'lamp' is not one of"),
        "{body}"
    );
    let mut bad = pump();
    bad["colour"] = json!("red");
    let (status, body) = req("POST", "/api/v1/faceplates".into(), Some(bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("colour"), "{body}");

    let (_, list) = req("GET", "/api/v1/faceplates".into(), None).await;
    assert_eq!(list[0]["roles"].as_array().unwrap().len(), 4);
    assert_eq!(list[0]["roles"][2]["label"], "Start / stop");

    // Three pumps on a screen: one definition, three mappings.
    let (_, screen) = req(
        "POST",
        "/api/v1/screens".into(),
        Some(json!({"name": "Pumps"})),
    )
    .await;
    let screen = screen["id"].as_str().unwrap().to_string();
    let items: Vec<serde_json::Value> = (1..=3)
        .map(|n| {
            json!({
                "equipment_id": format!("p{n}"), "equipment_type": "symbol", "equipment_name": format!("P-30{n}"),
                "x": 100 * n, "y": 40, "width": 60, "height": 60, "rotation": 0,
                "sensor_bindings": format!("[{n}]"),
                "config": json!({"symbol": "pump", "faceplate": "pump_std",
                    "faceplateMap": {"run": n, "fault": n + 10, "cmd": n + 20}, "z": n}).to_string()
            })
        })
        .collect();
    let (status, body) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        Some(json!({"items": items})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (_, one) = req("GET", "/api/v1/faceplates/pump_std".into(), None).await;
    assert_eq!(one["used_by"].as_array().unwrap().len(), 3, "{one}");
    let (status, body) = req("DELETE", "/api/v1/faceplates/pump_std".into(), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("3 symbol(s)"),
        "{body}"
    );

    // Changing the definition changes it for every instance; the id stays.
    let mut renamed = pump();
    renamed["name"] = json!("Pump (VFD)");
    let (status, _) = req("PUT", "/api/v1/faceplates/pump_std".into(), Some(renamed)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = req("PUT", "/api/v1/faceplates/other".into(), Some(pump())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = req(
        "PUT",
        format!("/api/v1/screens/{screen}/scada-equipment"),
        Some(json!({"items": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = req("DELETE", "/api/v1/faceplates/pump_std".into(), None).await;
    assert_eq!(status, StatusCode::OK);
}
