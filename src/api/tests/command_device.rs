use super::*;
use http::StatusCode;

/// Create a writable data source on `device_id` and return its id.
async fn writable_source(
    app: &Router,
    cookie: &str,
    key: &str,
    source_type: &str,
    device_id: &str,
) -> i64 {
    let (status, created) = send_request(
        app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(cookie),
        Some(serde_json::json!({
            "key": key, "name": key, "source_type": source_type, "device_id": device_id,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_i64().unwrap();
    let (status, _) = send_request(
        app,
        "PATCH",
        &format!("/api/v1/data-sources/{id}/write-policy"),
        None,
        Some(cookie),
        Some(serde_json::json!({"writable": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn write_and_read_back(app: &Router, cookie: &str, id: i64) -> serde_json::Value {
    let (status, accepted) = send_request(
        app,
        "POST",
        &format!("/api/v1/tags/{id}/write"),
        None,
        Some(cookie),
        Some(serde_json::json!({"value": 12.5})),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let command_id = accepted["command_id"].as_str().unwrap();
    let (status, command) = send_request(
        app,
        "GET",
        &format!("/api/v1/commands/{command_id}"),
        None,
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    command
}

/// SCADA-16 S10: a write to a tag of a Modbus device with a text id (here one
/// defined only in the configuration file, so absent from `modbus_devices`)
/// answered 500 "Device not found for data source", and a device that was in
/// the table was queued as device id 0 with nothing saying which one.
#[tokio::test]
async fn test_write_to_modbus_text_device_id_records_which_device() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let id = writable_source(&app, &cookie, "pv_power", "modbus", "inverter-1").await;
    let command = write_and_read_back(&app, &cookie, id).await;

    assert_eq!(command["protocol"], "modbus");
    assert_eq!(command["device_ref"], "inverter-1");
    assert_eq!(
        command["device_id"], 0,
        "the numeric id stays 0 for text ids"
    );

    // The device filter finds the command by the text id.
    let (status, list) = send_request(
        &app,
        "GET",
        "/api/v1/commands?device=inverter-1",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/commands?device=inverter-2",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert!(list.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_write_to_numeric_device_keeps_numeric_id() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let id = writable_source(&app, &cookie, "valve", "s7", "3").await;
    let command = write_and_read_back(&app, &cookie, id).await;

    assert_eq!(command["device_id"], 3);
    assert_eq!(command["device_ref"], "3");

    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/commands?device=3",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}
