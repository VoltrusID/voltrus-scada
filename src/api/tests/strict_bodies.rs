//! Create and update routes reject unknown fields and enumerated values
//! outside their list, naming the field, instead of storing or ignoring them.

use super::*;
use http::StatusCode;
use serde_json::json;

async fn admin_app() -> (Router, String) {
    let state = seeded_admin_app_state().await;
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie)
}

fn error_of(body: &serde_json::Value) -> &str {
    body["error"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn data_source_routes_reject_unknown_fields() {
    let (app, cookie) = admin_app().await;
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(json!({"key": "t1", "name": "T1", "warn_limit": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        error_of(&body).contains("unknown field 'warn_limit'"),
        "{body}"
    );

    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        Some(json!({"key": "t1", "name": "T1", "chart_type": "gauge"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_i64().unwrap();

    // What GET returns can be sent back as is.
    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/data-sources",
        None,
        Some(&cookie),
        None,
    )
    .await;
    let row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .unwrap()
        .clone();
    let (status, body) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/data-sources/{id}"),
        None,
        Some(&cookie),
        Some(row),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/data-sources/{id}"),
        None,
        Some(&cookie),
        Some(json!({"on_delay": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        error_of(&body).contains("unknown field 'on_delay'"),
        "{body}"
    );
}

#[tokio::test]
async fn screen_routes_reject_unknown_fields() {
    let (app, cookie) = admin_app().await;
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/screens",
        None,
        Some(&cookie),
        Some(json!({"name": "Pumps", "title": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(error_of(&body).contains("unknown field 'title'"), "{body}");
}

#[tokio::test]
async fn a_screen_from_get_can_be_put_back() {
    let (app, cookie) = admin_app().await;
    let (status, created) = send_request(
        &app,
        "POST",
        "/api/v1/screens",
        None,
        Some(&cookie),
        Some(json!({"name": "Pumps"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let id = created["id"].as_str().unwrap().to_string();
    let (_, list) = send_request(&app, "GET", "/api/v1/screens", None, Some(&cookie), None).await;
    let mut row = list
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap()
        .clone();
    assert!(row.get("type").is_some(), "{row}");
    row["name"] = json!("Pumps 2");
    let (status, body) = send_request(
        &app,
        "PUT",
        &format!("/api/v1/screens/{id}"),
        None,
        Some(&cookie),
        Some(row),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn modbus_routes_reject_unknown_register_types() {
    let (app, cookie) = admin_app().await;
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/modbus-devices",
        None,
        Some(&cookie),
        Some(
            json!({"id": "m1", "name": "M1", "ip": "127.0.0.1", "registers": [
                {"name": "a", "address": 0},
                {"name": "b", "address": 2, "register_type": "uint32"}
            ]}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let msg = error_of(&body);
    assert!(
        msg.contains("register 2: register_type \"uint32\""),
        "{msg}"
    );
    assert!(
        msg.contains("int16, uint16, int32, float32, float64"),
        "{msg}"
    );

    let (_, list) = send_request(
        &app,
        "GET",
        "/api/v1/modbus-devices",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(list, json!([]), "nothing was stored");
}
