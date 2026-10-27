use super::*;
use axum::body::Body;
use http::Request;
use std::collections::HashMap;
use tower::ServiceExt;

pub fn test_app() -> Router {
    let cache = DeviceCache::new();
    let storage = Storage::new_in_memory().unwrap();
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
    create_router(state)
}

pub async fn seed_cache(cache: &DeviceCache) {
    let metrics = HashMap::from([("voltage".to_string(), 220.5), ("current".to_string(), 5.2)]);
    cache.update("plts_1", metrics, false).await;
}

/// Extract a specific cookie value from potentially multiple Set-Cookie headers.
pub fn extract_cookie_value(headers: &http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|s| {
            let kv = s.split(';').next()?.trim();
            if kv.starts_with(&format!("{}=", name)) {
                Some(kv.trim_start_matches(&format!("{}=", name)).to_string())
            } else {
                None
            }
        })
}

pub async fn seed_and_get_cookie(app: &Router) -> String {
    // Seed directly via storage (bypass HTTP)
    let _state = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .ok();

    // We need to seed via storage - use the login approach with direct seed
    // For tests, we create a test-specific approach using the seed endpoint
    // after obtaining a token through direct storage seeding
    // Simplified: use a separate test helper
    let storage = Storage::new_in_memory().unwrap();
    let password_hash = crate::api::auth::hash_password("admin");
    storage
        .create_user("admin", &password_hash, "", "admin")
        .await
        .unwrap();

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

    extract_cookie_value(resp.headers(), "voltrus_token").expect("voltrus_token cookie not found")
}

/// Helper to create a seeded admin user and return (storage, cache, poller_manager) tuple.
pub fn seeded_admin_state() -> (Storage, DeviceCache, PollerManager) {
    let storage = Storage::new_in_memory().unwrap();
    let password_hash = crate::api::auth::hash_password("admin");
    // We can't await here in a non-async fn, so callers should use the inline pattern
    let _ = password_hash;
    let cache = DeviceCache::new();
    let poller_manager = PollerManager::new(
        cache.clone(),
        storage.clone(),
        crate::network::NetworkManager::new(vec![]),
    );
    (storage, cache, poller_manager)
}

/// Build an AppState with a seeded admin user (call from async context).
pub async fn test_app_with_seeded_admin() -> Router {
    create_router(seeded_admin_app_state().await)
}

/// AppState over an in-memory store with an `admin`/`admin` user.
pub async fn seeded_admin_app_state() -> AppState {
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
    AppState {
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
    }
}

pub async fn send_request(
    app: &Router,
    method: &str,
    uri: &str,
    api_key: Option<&str>,
    cookie: Option<&str>,
    body: Option<serde_json::Value>,
) -> (http::StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(key) = api_key {
        builder = builder.header("X-API-Key", key);
    }
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    let req = builder
        .header("content-type", "application/json")
        .body(Body::from(body.map(|v| v.to_string()).unwrap_or_default()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

/// Log in with `username`/`password` and return the access-token cookie value.
pub async fn login_cookie(app: &Router, username: &str, password: &str) -> String {
    let login_body = serde_json::json!({"username": username, "password": password});
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
    extract_cookie_value(resp.headers(), "voltrus_token").expect("voltrus_token cookie not found")
}

pub async fn admin_cookie(app: &Router) -> String {
    login_cookie(app, "admin", "admin").await
}

pub fn test_app_with_license_status(license_status: crate::license::LicenseStatus) -> Router {
    let cache = DeviceCache::new();
    let storage = Storage::new_in_memory().unwrap();
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
        license_status: std::sync::Arc::new(std::sync::RwLock::new(license_status)),
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
    create_router(state)
}
