//! SCADA-16 W7, B6, B7, B8: routes and bodies the web client uses for the protocol device
//! managers, simulation state and write policies.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn admin_app() -> (Router, String, Storage) {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie, storage)
}

/// B6: only POST was routed, so the OPC UA manager's list answered 405.
#[tokio::test]
async fn test_opcua_devices_list_is_routed() {
    let (app, cookie, _) = admin_app().await;

    let (status, list) = send_request(
        &app,
        "GET",
        "/api/v1/opcua/devices",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "GET /opcua/devices: {list}");
    assert_eq!(list, json!([]));

    // The body the web client now sends: the server's enum spellings.
    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/opcua/devices",
        None,
        Some(&cookie),
        Some(json!({
            "name": "srv", "url": "opc.tcp://127.0.0.1:4840",
            "security_policy": "None", "security_mode": "None",
            "username": null, "password": null, "interval_ms": 1000
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().unwrap();

    let (status, list) = send_request(
        &app,
        "GET",
        "/api/v1/opcua/devices",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let row = &list.as_array().unwrap()[0];
    assert_eq!(row["id"], id);
    assert_eq!(row["security_policy"], "None");
    assert_eq!(row["security_mode"], "None");
    assert_eq!(row["node_count"], 0);
    assert!(row.get("password").is_none());
}

/// W7: the web sent lowercase `none`/`sign`, which are not variants of the server's enums.
#[tokio::test]
async fn test_opcua_create_rejects_lowercase_enum_names() {
    let (app, cookie, _) = admin_app().await;
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/opcua/devices",
        None,
        Some(&cookie),
        Some(json!({"name": "srv", "url": "opc.tcp://h:1", "security_policy": "none", "security_mode": "sign"})),
    )
    .await;
    assert!(
        status.is_client_error(),
        "lowercase names must not deserialise, got {status}"
    );
}

/// W7: S7 and EtherNet/IP take `url` and tag addresses/symbols, return numeric ids and
/// `tag_count`; the web sent `ip`/`port`, `area`/`db_number`/`offset` and no `tag_name`.
#[tokio::test]
async fn test_s7_and_enip_accept_the_bodies_the_web_client_sends() {
    let (app, cookie, _) = admin_app().await;

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/s7/devices",
        None,
        Some(&cookie),
        Some(json!({"name": "S7", "url": "10.0.0.5:102", "rack": 0, "slot": 1, "enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().expect("numeric id");
    let (status, tag) = send_request(
        &app,
        "POST",
        &format!("/api/v1/s7/devices/{id}/tags"),
        None,
        Some(&cookie),
        Some(json!({"tag_address": "DB1.DBD0", "name": "flow", "display_name": "", "data_type": "float32"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tag}");
    let (_, list) =
        send_request(&app, "GET", "/api/v1/s7/devices", None, Some(&cookie), None).await;
    assert_eq!(list[0]["url"], "10.0.0.5:102");
    assert_eq!(list[0]["tag_count"], 1);
    let (_, tags) = send_request(
        &app,
        "GET",
        &format!("/api/v1/s7/devices/{id}/tags"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(tags[0]["tag_address"], "DB1.DBD0");

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/enip/devices",
        None,
        Some(&cookie),
        Some(json!({"name": "Logix", "url": "10.0.0.6:44818", "slot": 0, "assembly_instance": 0, "enabled": false})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().expect("numeric id");
    let (status, tag) = send_request(
        &app,
        "POST",
        &format!("/api/v1/enip/devices/{id}/tags"),
        None,
        Some(&cookie),
        Some(json!({"tag_name": "Program.Flow", "name": "flow", "display_name": "", "data_type": "dint32", "assembly_offset": 0, "assembly_length": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{tag}");
    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/enip/devices",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(list[0]["url"], "10.0.0.6:44818");
    assert_eq!(list[0]["tag_count"], 1);
    let (_, tags) = send_request(
        &app,
        "GET",
        &format!("/api/v1/enip/devices/{id}/tags"),
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(tags[0]["tag_name"], "Program.Flow");
}

/// B7: the Modbus manager fetches a device's simulation state with GET; only POST was routed.
#[tokio::test]
async fn test_get_device_simulation_state() {
    let (app, cookie, storage) = admin_app().await;
    storage.create_device("plc-1", "PLC 1", 0).await.unwrap();

    let uri = "/api/v1/devices/plc-1/simulate";
    let (status, state) = send_request(&app, "GET", uri, None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{state}");
    assert_eq!(
        state,
        json!({"simulation_mode": false, "sim_profile_id": null, "sim_config": null})
    );

    let (status, _) = send_request(
        &app,
        "POST",
        uri,
        None,
        Some(&cookie),
        Some(json!({"simulation_mode": true, "sim_config": "{\"power\":5}"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, state) = send_request(&app, "GET", uri, None, Some(&cookie), None).await;
    assert_eq!(state["simulation_mode"], true);
    assert_eq!(state["sim_config"], "{\"power\":5}");

    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/devices/nope/simulate",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// B8: a tag's write policy could only be PATCHed, never read back.
#[tokio::test]
async fn test_get_write_policy() {
    let (app, cookie, _) = admin_app().await;
    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(json!({"key": "temp", "name": "Temp", "source_type": "random_walk"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_i64().unwrap();
    let uri = format!("/api/v1/data-sources/{id}/write-policy");

    let (status, policy) = send_request(&app, "GET", &uri, None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{policy}");
    assert_eq!(policy["write_min"], serde_json::Value::Null);
    assert_eq!(policy["write_requires_reason"], false);

    let (status, _) = send_request(
        &app,
        "PATCH",
        &uri,
        None,
        Some(&cookie),
        Some(json!({"writable": true, "write_min": 1.5, "write_max": 90.0, "write_max_rate": 12, "write_requires_reason": true, "write_deadband": 0.25})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, policy) = send_request(&app, "GET", &uri, None, Some(&cookie), None).await;
    assert_eq!(
        policy,
        json!({"writable": true, "write_min": 1.5, "write_max": 90.0, "write_max_rate": 12,
               "write_requires_reason": true, "write_deadband": 0.25})
    );

    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/data-sources/999999/write-policy",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
