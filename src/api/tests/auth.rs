use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn test_auth_me_unauthenticated() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["authenticated"], false);
}

#[tokio::test]
async fn test_seed_requires_auth() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/seed")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_login_with_seeded_user() {
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

    // Login with admin/admin
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

    assert_eq!(resp.status(), StatusCode::OK);

    assert!(extract_cookie_value(resp.headers(), "voltrus_token").is_some());
    assert!(extract_cookie_value(resp.headers(), "voltrus_refresh").is_some());

    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["ok"], true);
    assert_eq!(json["username"], "admin");
}

#[tokio::test]
async fn test_login_invalid_credentials() {
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

    let login_body = serde_json::json!({"username": "admin", "password": "wrong"});
    let resp = app
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

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_protected_route_requires_cookie() {
    let app = test_app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/system")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_full_auth_flow() {
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

    // Login
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

    // Access protected route with cookie
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/system")
                .header("cookie", format!("voltrus_token={}", cookie_val))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

/// S7: the `Set-Cookie` value of the access token is what a non-browser
/// client hands to the WebSocket `Auth` message, which does not decode
/// cookies. It must be usable as is: nothing in it needs percent-encoding.
#[tokio::test]
async fn test_login_token_cookie_works_unencoded_for_websocket_auth() {
    let state = seeded_admin_app_state().await;
    let app = create_router(state.clone());
    let raw = admin_cookie(&app).await;

    assert!(!raw.contains('%'), "token cookie is percent-encoded: {raw}");
    // Exactly what the WebSocket `Auth` path runs on the received token.
    let claims = crate::api::auth_middleware::verify_session(&state, &raw)
        .await
        .expect("raw Set-Cookie token must verify");
    assert_eq!(claims.username, "admin");
}
