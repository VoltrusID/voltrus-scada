use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_flow_crud() {
    let storage = Storage::new_in_memory().unwrap();
    let password_hash = crate::api::auth::hash_password("admin");
    storage
        .create_user("admin", &password_hash, "", "admin")
        .await
        .unwrap();

    let cache = DeviceCache::new();
    let poller_manager = PollerManager::new(
        cache.clone(),
        storage.clone(),
        crate::network::NetworkManager::new(vec![]),
    );
    let state = AppState {
        cache,
        storage,
        poller_manager,
        started_at: std::time::Instant::now(),
        auth_secret: "test-secret".to_string(),
        cors_origin: None,
        cookie_secure: crate::models::CookieSecure::Never,
        retention_config: crate::models::RetentionConfig::default(),
        flow_engine: None,
        login_rate_limiter: crate::security::LoginRateLimiter::new(),
        global_rate_limiter: crate::security::GlobalRateLimiter::new(),
        audit_logger: crate::security::AuditLogger::new().0,
        intrusion_detector: crate::security::IntrusionDetector::new(),
        license_status: std::sync::Arc::new(std::sync::RwLock::new(
            crate::license::LicenseStatus::Community,
        )),
        ext: std::sync::Arc::new(crate::edition::Community),
        smtp_config: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        config_path: "config.yaml".to_string(),
        ws_manager: Arc::new(crate::ws_manager::WsManager::new()),
        mqtt_broker_state: None,
        network_manager: crate::network::NetworkManager::new(vec![]),
        health_monitor: Arc::new(crate::health::watchdog::HealthMonitor::new(
            crate::models::WatchdogConfig::default(),
        )),
        widget_registry: crate::widget_sdk::WidgetRegistry::new(std::path::PathBuf::from(
            "voltrus/widgets",
        )),
        modbus_pool: std::sync::Arc::new(crate::modbus::ModbusPool::new()),
    };
    let app = create_router(state);

    // Login to get auth cookie
    let login_body = serde_json::json!({"username": "admin", "password": "admin"});
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&login_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    let cookie_val = extract_cookie_value(resp.headers(), "voltrus_token")
        .expect("voltrus_token cookie not found");

    // Create flow
    let flow = serde_json::json!({
        "id": "test_flow",
        "name": "Test Flow",
        "enabled": true,
        "trigger_config": serde_json::json!({"type": "modbus", "device_id": "*"}).to_string(),
        "nodes": serde_json::json!([
            {"id": "n1", "type": "filter", "condition": "metrics.temp > 0"}
        ]).to_string(),
        "created_at": 0,
        "updated_at": 0,
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/flows")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::from(serde_json::to_string(&flow).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Get flow
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/flows/test_flow")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // Test flow
    let test_payload = serde_json::json!({
        "payload": {
            "metrics": {
                "temp": 25.0
            }
        }
    });

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/flows/test_flow/test")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::from(serde_json::to_string(&test_payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["success"], true);

    // Delete flow
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/flows/test_flow")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

/// SCADA-16 S13: start/stop changed only the running engine, so the stored
/// `enabled` (and `GET /flows`) went stale and a restart undid them.
#[tokio::test]
async fn test_flow_start_stop_persist_enabled() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let flow = serde_json::json!({
        "id": "f1",
        "name": "F1",
        "enabled": false,
        "trigger_config": serde_json::json!({"type": "modbus", "device_id": "*"}).to_string(),
        "nodes": "[]",
        "created_at": 0,
        "updated_at": 0,
    });
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows",
        None,
        Some(&cookie),
        Some(flow),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let enabled = |app: Router, cookie: String| async move {
        let (status, body) =
            send_request(&app, "GET", "/api/v1/flows/f1", None, Some(&cookie), None).await;
        assert_eq!(status, StatusCode::OK);
        body["enabled"].as_bool().unwrap()
    };

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows/f1/start",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        enabled(app.clone(), cookie.clone()).await,
        "start persists enabled"
    );
    let (_, list) = send_request(&app, "GET", "/api/v1/flows", None, Some(&cookie), None).await;
    assert_eq!(
        list["flows"][0]["enabled"], true,
        "GET /flows shows the started flow"
    );

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows/f1/stop",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        !enabled(app.clone(), cookie.clone()).await,
        "stop persists enabled"
    );

    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows/missing/start",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown flow is not a silent 204"
    );
}
