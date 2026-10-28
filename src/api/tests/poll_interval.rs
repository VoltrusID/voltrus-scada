use super::*;
use http::StatusCode;
use serde_json::json;

fn device(id: &str, interval: serde_json::Value) -> serde_json::Value {
    json!({
        "id": id, "name": id, "ip": "127.0.0.1", "port": 1, "interval_ms": interval,
        "registers": [{"name": "v", "address": 40001, "register_type": "uint16"}]
    })
}

#[tokio::test]
async fn modbus_device_interval_must_be_1_to_3_600_000() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let post = |body| {
        send_request(
            &app,
            "POST",
            "/api/v1/modbus-devices",
            None,
            Some(&cookie),
            Some(body),
        )
    };

    for bad in [json!(0), json!(3_600_001), json!("fast"), json!(-5)] {
        let (status, body) = post(device("bad", bad.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
    }
    let (_, body) = post(device("bad", json!(0))).await;
    assert!(
        body["error"].as_str().unwrap().contains("1-3600000"),
        "{body}"
    );

    for (id, ok) in [("one", json!(1)), ("max", json!(3_600_000))] {
        let (status, body) = post(device(id, ok)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    // Left out, it defaults to one second.
    let mut no_interval = device("dflt", json!(null));
    no_interval.as_object_mut().unwrap().remove("interval_ms");
    let (status, _) = post(no_interval).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, got) = send_request(
        &app,
        "GET",
        "/api/v1/modbus-devices/dflt",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(got["device"]["interval_ms"], 1000, "{got}");
}

#[tokio::test]
async fn modbus_device_update_rejects_a_zero_interval() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/modbus-devices",
        None,
        Some(&cookie),
        Some(device("m1", json!(500))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send_request(
        &app,
        "PUT",
        "/api/v1/modbus-devices/m1",
        None,
        Some(&cookie),
        Some(json!({"interval_ms": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let (_, got) = send_request(
        &app,
        "GET",
        "/api/v1/modbus-devices/m1",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(
        got["device"]["interval_ms"], 500,
        "the stored interval is untouched"
    );

    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/modbus-devices/m1",
        None,
        Some(&cookie),
        Some(json!({"interval_ms": 250})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
