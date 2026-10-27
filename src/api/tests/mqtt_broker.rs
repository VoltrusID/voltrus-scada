use super::*;
use axum::body::Body;
use http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn test_mqtt_broker_status_requires_auth() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/mqtt-broker/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(!resp.status().is_success());
}

#[tokio::test]
async fn test_mqtt_broker_start_requires_auth() {
    let app = test_app();
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/mqtt-broker/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(!resp.status().is_success());
}

/// SCADA-15 B3: saving broker settings used to replace the user list with the
/// (empty or partial) list sent, deleting every existing broker login.
#[tokio::test]
async fn test_mqtt_config_save_keeps_existing_users() {
    let mut state = seeded_admin_app_state().await;
    let broker = crate::mqtt_broker::MqttBrokerState::new();
    state.mqtt_broker_state = Some(broker.clone());
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);

    let put = |body: serde_json::Value| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            let (status, _) = send_request(
                &app,
                "PUT",
                "/api/v1/mqtt-broker/config",
                None,
                Some(&cookie),
                Some(body),
            )
            .await;
            assert_eq!(status, http::StatusCode::OK);
        }
    };
    let users = || async {
        let cfg = broker.config.read().await;
        let mut users: Vec<(String, String)> = cfg
            .as_ref()
            .unwrap()
            .auth
            .users
            .iter()
            .map(|u| (u.username.clone(), u.password.clone()))
            .collect();
        users.sort();
        users
    };
    let pair = |u: &str, p: &str| (u.to_string(), p.to_string());

    put(serde_json::json!({"auth": {"enabled": true, "users": [
        {"username": "plc", "password": "a"},
        {"username": "hmi", "password": "b"},
    ]}}))
    .await;

    // What the web form sent when only the port changed.
    put(serde_json::json!({"port": 1884, "auth": {"enabled": true, "users": []}})).await;
    assert_eq!(users().await, vec![pair("hmi", "b"), pair("plc", "a")]);

    // A partial list adds new users and changes passwords without dropping others.
    put(serde_json::json!({"auth": {"enabled": true, "users": [
        {"username": "plc", "password": "c"},
        {"username": "scada", "password": "d"},
    ]}}))
    .await;
    assert_eq!(
        users().await,
        vec![pair("hmi", "b"), pair("plc", "c"), pair("scada", "d")]
    );
}

async fn broker_app() -> (Router, String) {
    let mut state = seeded_admin_app_state().await;
    state.mqtt_broker_state = Some(crate::mqtt_broker::MqttBrokerState::new());
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    (app, cookie)
}

/// SCADA-17 S12: enabling authentication before adding a user read back as
/// off (`auth_enabled` also needed a user), so the web form unticked it; and a
/// stored configuration could not be cleared.
#[tokio::test]
async fn test_mqtt_config_auth_reads_from_mode_and_config_can_be_cleared() {
    let (app, cookie) = broker_app().await;
    let status = || async {
        let (code, body) = send_request(
            &app,
            "GET",
            "/api/v1/mqtt-broker/status",
            None,
            Some(&cookie),
            None,
        )
        .await;
        assert_eq!(code, http::StatusCode::OK, "{body}");
        body["config"].clone()
    };
    assert!(status().await.is_null());

    let (code, body) = send_request(
        &app,
        "PUT",
        "/api/v1/mqtt-broker/config",
        None,
        Some(&cookie),
        Some(serde_json::json!({"port": 1884, "auth": {"enabled": true}})),
    )
    .await;
    assert_eq!(code, http::StatusCode::OK, "{body}");
    assert_eq!(body["config"]["auth_enabled"], true, "{body}");
    assert_eq!(status().await["auth_enabled"], true);
    assert_eq!(status().await["port"], 1884);

    let (code, body) = send_request(
        &app,
        "DELETE",
        "/api/v1/mqtt-broker/config",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(code, http::StatusCode::OK, "{body}");
    assert!(body["config"].is_null());
    assert!(status().await.is_null());
}

/// S12: the log is fed from the broker's own link; a message published through
/// `POST /publish` has to be in it, once.
#[tokio::test]
async fn test_mqtt_publish_appears_once_in_the_message_log() {
    let (app, cookie) = broker_app().await;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let call = |method: &'static str, uri: &'static str, body: Option<serde_json::Value>| {
        let (app, cookie) = (app.clone(), cookie.clone());
        async move { send_request(&app, method, uri, None, Some(&cookie), body).await }
    };
    let (code, body) = call(
        "PUT",
        "/api/v1/mqtt-broker/config",
        Some(serde_json::json!({"port": port})),
    )
    .await;
    assert_eq!(code, http::StatusCode::OK, "{body}");
    let (code, body) = call("POST", "/api/v1/mqtt-broker/start", None).await;
    assert_eq!(code, http::StatusCode::OK, "{body}");

    let (code, body) = call(
        "POST",
        "/api/v1/mqtt-broker/publish",
        Some(serde_json::json!({"topic": "plant/line1/temp", "payload": "21.5"})),
    )
    .await;
    assert_eq!(code, http::StatusCode::OK, "{body}");

    let mut seen = 0;
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (_, log) = call("GET", "/api/v1/mqtt-broker/messages", None).await;
        seen = log["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["topic"] == "plant/line1/temp")
            .count();
        if seen > 0 {
            break;
        }
    }
    // Leave time for an echo from the broker to show up as a second entry.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (_, log) = call("GET", "/api/v1/mqtt-broker/messages", None).await;
    let count = log["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["topic"] == "plant/line1/temp")
        .count();
    let _ = call("POST", "/api/v1/mqtt-broker/stop", None).await;
    assert_eq!(count, 1, "published message in the log (first seen {seen})");
}
