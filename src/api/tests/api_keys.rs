use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_api_key_crud() {
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
        cookie_secure: false,
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

    // Login to get admin cookie
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

    // Create API key
    let create_body = serde_json::json!({
        "name": "test-export-key",
        "permissions": "[\"data:read\", \"data:export\"]"
    });
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/api-keys")
                .header("content-type", "application/json")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::from(serde_json::to_string(&create_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["role"], "api");
    assert_eq!(json["name"], "test-export-key");
    assert!(json["key"].as_str().unwrap().len() == 64);
    let plain_key = json["key"].as_str().unwrap().to_string();
    let key_id = json["id"].as_i64().unwrap();

    // List API keys
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/api-keys")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Use API key to access protected route (X-API-Key header)
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/system")
                .header("X-API-Key", &plain_key)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Use API key via query parameter
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&format!("/api/v1/system?api_key={}", plain_key))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Invalid API key should fail
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/system")
                .header("X-API-Key", "invalid-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Revoke API key
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/api/v1/api-keys/{}", key_id))
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Revoked key should no longer work
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/system")
                .header("X-API-Key", &plain_key)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

async fn create_key(
    app: &Router,
    cookie: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    send_request(
        app,
        "POST",
        "/api/v1/api-keys",
        None,
        Some(&format!("voltrus_token={}", cookie)),
        Some(body),
    )
    .await
}

/// Keys created without a role default to "api": read-only, exactly the
/// pre-existing behavior.
#[tokio::test]
async fn test_api_key_default_role_is_read_only() {
    let app = test_app_with_seeded_admin().await;
    let cookie = admin_cookie(&app).await;

    let (status, json) =
        create_key(&app, &cookie, serde_json::json!({"name": "readonly-key"})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["role"], "api");
    let plain_key = json["key"].as_str().unwrap().to_string();

    // Reads keep working
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/data-sources",
        Some(&plain_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Operational writes stay blocked for plain keys
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        Some(&plain_key),
        None,
        Some(serde_json::json!({"key": "k", "name": "n"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin routes stay blocked
    let (status, _) =
        send_request(&app, "GET", "/api/v1/users", Some(&plain_key), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// "integration" keys pass the demo-allowed operational write routes but are
/// still denied on admin-only routes.
#[tokio::test]
async fn test_integration_key_write_access() {
    let app = test_app_with_seeded_admin().await;
    let cookie = admin_cookie(&app).await;

    let (status, json) = create_key(
        &app,
        &cookie,
        serde_json::json!({"name": "ci-key", "role": "integration"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["role"], "integration");
    let plain_key = json["key"].as_str().unwrap().to_string();

    // Representative operational write (data-source/tag config class)
    let (status, body) = send_request(
        &app,
        "POST",
        "/api/v1/data-sources",
        Some(&plain_key),
        None,
        Some(serde_json::json!({"key": "integration_tag", "name": "integration_tag"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body["id"].as_i64().unwrap() > 0);

    // Reads still fine
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/data-sources",
        Some(&plain_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Admin-only routes stay closed: users and API-key management
    let (status, _) =
        send_request(&app, "GET", "/api/v1/users", Some(&plain_key), None, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/api-keys",
        Some(&plain_key),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// Only "api" and "integration" are valid key roles; admin/demo/unknown are
/// rejected at creation and no key is stored.
#[tokio::test]
async fn test_api_key_create_rejects_privileged_and_unknown_roles() {
    let app = test_app_with_seeded_admin().await;
    let cookie = admin_cookie(&app).await;

    for role in ["admin", "demo", "root"] {
        let (status, json) = create_key(
            &app,
            &cookie,
            serde_json::json!({"name": "bad-key", "role": role}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "role {} should be rejected",
            role
        );
        assert!(json["error"].as_str().unwrap().contains(role));
    }

    let (status, json) = send_request(
        &app,
        "GET",
        "/api/v1/api-keys",
        None,
        Some(&format!("voltrus_token={}", cookie)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["keys"].as_array().unwrap().len(), 0);
}
