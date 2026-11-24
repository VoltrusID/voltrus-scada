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

/// Tags of two devices with the same name would share a data source key, and
/// with it alarm state: the name is refused on create and on rename.
#[cfg(feature = "s7")]
#[tokio::test]
async fn test_s7_device_names_must_be_unique() {
    let (app, cookie, _) = admin_app().await;
    let body = |name: &str| json!({"name": name, "url": "127.0.0.1:102", "rack": 0, "slot": 1, "interval_ms": 1000, "enabled": false});
    let (status, first) = send_request(
        &app,
        "POST",
        "/api/v1/s7/devices",
        None,
        Some(&cookie),
        Some(body("PLC1")),
    )
    .await;
    assert!(status.is_success(), "{first}");
    let (status, err) = send_request(
        &app,
        "POST",
        "/api/v1/s7/devices",
        None,
        Some(&cookie),
        Some(body("PLC1")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");

    let (status, second) = send_request(
        &app,
        "POST",
        "/api/v1/s7/devices",
        None,
        Some(&cookie),
        Some(body("PLC2")),
    )
    .await;
    assert!(status.is_success(), "{second}");
    let id = second["id"].as_i64().unwrap();
    let (status, err) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/s7/devices/{id}"),
        None,
        Some(&cookie),
        Some(json!({"name": "PLC1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
    // Keeping its own name is fine.
    let (status, ok) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/s7/devices/{id}"),
        None,
        Some(&cookie),
        Some(json!({"name": "PLC2"})),
    )
    .await;
    assert!(status.is_success(), "{ok}");
}

/// OPC UA and EtherNet/IP tags share the key format `<device>:<tag>`.
#[cfg(all(feature = "opcua", feature = "ethernet-ip"))]
#[tokio::test]
async fn test_opcua_and_enip_device_names_do_not_collide() {
    let (app, cookie, _) = admin_app().await;
    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/opcua/devices",
        None,
        Some(&cookie),
        Some(json!({
            "name": "Line1", "url": "opc.tcp://127.0.0.1:4840",
            "security_policy": "None", "security_mode": "None",
            "username": null, "password": null, "interval_ms": 1000
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let (status, err) = send_request(
        &app,
        "POST",
        "/api/v1/enip/devices",
        None,
        Some(&cookie),
        Some(json!({"name": "Line1", "url": "127.0.0.1:44818", "slot": 0, "interval_ms": 1000})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{err}");
}

/// An S7 tag reads its poller's values: the live data API from the cache
/// under `s7_<id>`, history from that device's telemetry, even when the
/// device's name has a ':' in it.
#[cfg(feature = "s7")]
#[tokio::test]
async fn test_s7_tag_reads_live_value_and_history_from_its_poller() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let cache = state.cache.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, dev) = send_request(
        &app,
        "POST",
        "/api/v1/s7/devices",
        None,
        Some(&cookie),
        Some(json!({"name": "10.0.0.5:102", "url": "10.0.0.5:102", "rack": 0, "slot": 1, "interval_ms": 1000, "enabled": false})),
    )
    .await;
    assert!(status.is_success(), "{dev}");
    let id = dev["id"].as_i64().unwrap();
    let (status, tag) = send_request(
        &app,
        "POST",
        &format!("/api/v1/s7/devices/{id}/tags"),
        None,
        Some(&cookie),
        Some(json!({"tag_address": "DB1.DBD0", "name": "temp", "display_name": "", "data_type": "REAL"})),
    )
    .await;
    assert!(status.is_success(), "{tag}");

    let device = format!("s7_{id}");
    cache
        .update(
            &device,
            std::collections::HashMap::from([("temp".to_string(), 42.5)]),
            false,
        )
        .await;
    cache
        .set_status(&device, crate::models::DeviceStatus::Online)
        .await;

    let (status, data) = send_request(&app, "GET", "/api/v1/data", None, Some(&cookie), None).await;
    assert_eq!(status, StatusCode::OK, "{data}");
    let row = data
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "s7_10.0.0.5:102:temp")
        .expect("the S7 tag in /data");
    assert_eq!(row["value"], 42.5, "{row}");
    assert_eq!(row["quality"], "good", "{row}");

    let now = crate::timeutil::now_secs();
    storage
        .insert_batch(&[
            (device.clone(), now - 120, "temp".to_string(), 40.0),
            (device.clone(), now - 60, "temp".to_string(), 41.0),
        ])
        .await
        .unwrap();
    let (status, hist) = send_request(
        &app,
        "GET",
        "/api/v1/history?sensor=s7_10.0.0.5:102:temp&range=5m",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{hist}");
    let values = hist["data"]["s7_10.0.0.5:102:temp"]["values"]
        .as_array()
        .expect("history values");
    let real: Vec<f64> = values.iter().filter_map(|v| v.as_f64()).collect();
    assert!(real.contains(&40.0) && real.contains(&41.0), "{hist}");
}
