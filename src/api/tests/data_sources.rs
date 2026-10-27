use super::*;
use http::StatusCode;

async fn find_source(app: &Router, cookie: &str, id: i64) -> serde_json::Value {
    let (status, list) =
        send_request(app, "GET", "/api/v1/data-sources", None, Some(cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    list.as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == id)
        .cloned()
        .expect("data source listed")
}

/// SCADA-15 B1: PUT used to answer ok while dropping source_config and device_id.
#[tokio::test]
async fn test_update_data_source_persists_source_config_and_device_id() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(serde_json::json!({"key": "temp", "name": "Temp", "source_type": "random_walk"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = created["id"].as_i64().unwrap();

    // The web sends source_config as a string of JSON.
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/data-sources/{id}"),
        None,
        Some(&cookie),
        Some(serde_json::json!({
            "source_type": "modbus",
            "source_config": "{\"register\":40001,\"low_warn\":5}",
            "device_id": "plc-1",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let source = find_source(&app, &cookie, id).await;
    assert_eq!(source["source_type"], "modbus");
    assert_eq!(
        source["source_config"],
        "{\"register\":40001,\"low_warn\":5}"
    );
    assert_eq!(source["device_id"], "plc-1");

    // An object is stored as its JSON text; a numeric device id as its digits.
    let (status, _) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/data-sources/{id}"),
        None,
        Some(&cookie),
        Some(serde_json::json!({"source_config": {"low_crit": 2}, "device_id": 7})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let source = find_source(&app, &cookie, id).await;
    assert_eq!(source["source_config"], "{\"low_crit\":2}");
    assert_eq!(source["device_id"], "7");
}

/// SCADA-16 B2: POST stored a string `source_config` as the JSON text of a
/// string (quoted and escaped), so the web lost the config of every source it
/// created or duplicated.
#[tokio::test]
async fn test_create_data_source_stores_source_config_like_put() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let create = |config: Option<serde_json::Value>, key: &'static str| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            let mut body =
                serde_json::json!({"key": key, "name": key, "source_type": "random_walk"});
            if let Some(config) = config {
                body["source_config"] = config;
            }
            let (status, created) = send_request(
                &app,
                "POST",
                "/api/v1/data-sources",
                None,
                Some(&cookie),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            find_source(&app, &cookie, created["id"].as_i64().unwrap()).await
        }
    };

    let from_string = create(Some(serde_json::json!("{\"volatility\":0.1}")), "a").await;
    assert_eq!(from_string["source_config"], "{\"volatility\":0.1}");

    let from_object = create(Some(serde_json::json!({"volatility": 0.2})), "b").await;
    assert_eq!(from_object["source_config"], "{\"volatility\":0.2}");

    let absent = create(None, "c").await;
    assert_eq!(absent["source_config"], "{}");
}
