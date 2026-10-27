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
