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
        storage: storage.clone(),
        poller_manager,
        started_at: std::time::Instant::now(),
        auth_secret: "test-secret".to_string(),
        cors_origin: None,
        cookie_secure: crate::models::CookieSecure::Never,
        public_demo: Default::default(),
        retention_config: crate::models::RetentionConfig::default(),
        flow_engine: None,
        login_rate_limiter: crate::security::LoginRateLimiter::new(),
        global_rate_limiter: crate::security::GlobalRateLimiter::new(),
        audit_logger: crate::security::AuditLogger::new().0,
        mcp_guard: Default::default(),
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

    // Delete flow (and its run log)
    storage
        .insert_flow_run("test_flow", 1, true, None, None)
        .await
        .unwrap();
    assert_eq!(
        storage.list_flow_runs("test_flow", 20).await.unwrap().len(),
        1
    );
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
    assert!(
        storage
            .list_flow_runs("test_flow", 20)
            .await
            .unwrap()
            .is_empty()
    );
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

// ── Flow graphs (v2) ──

use serde_json::{Value, json};

async fn post_flow(app: &Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    send_request(app, "POST", "/api/v1/flows", None, Some(cookie), Some(body)).await
}

fn flow_body(id: &str, trigger: Value, nodes: Value) -> Value {
    json!({"id": id, "name": id, "enabled": false, "trigger_config": trigger.to_string(), "nodes": nodes.to_string()})
}

async fn user_cookie(app: &Router, storage: &Storage, name: &str, role: &str) -> String {
    let hash = crate::api::auth::hash_password("pass-1234");
    storage.create_user(name, &hash, "", role).await.unwrap();
    format!(
        "voltrus_token={}",
        login_cookie(app, name, "pass-1234").await
    )
}

/// A writable tag with a write range of 0-100.
async fn writable_tag(storage: &Storage) -> i64 {
    storage.create_device("plc-1", "PLC 1", 0).await.unwrap();
    let id = storage
        .create_data_source(
            "setpoint", "setpoint", "", "", 0.0, 100.0, 0.0, 0.0, 0.0, 0, 2, "modbus", "{}",
            "plc-1", None, "line",
        )
        .await
        .unwrap();
    storage
        .update_write_policy(
            id,
            &crate::models::WritePolicyUpdate {
                writable: Some(true),
                write_min: Some(Some(0.0)),
                write_max: Some(Some(100.0)),
                write_max_rate: None,
                write_requires_reason: None,
                write_deadband: None,
            },
        )
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn flow_v1_is_stored_and_read_as_v2() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let (status, _) = post_flow(
        &app,
        &cookie,
        flow_body(
            "legacy",
            json!({"type": "modbus", "device_id": "*"}),
            json!([
                {"id": "f", "type": "filter", "condition": "metrics.temp > 0"},
                {"id": "a", "type": "alarm_out", "severity": "warning", "source_key": "t", "message": "m"},
            ]),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, flow) = send_request(
        &app,
        "GET",
        "/api/v1/flows/legacy",
        None,
        Some(&cookie),
        None,
    )
    .await;
    let nodes: Value = serde_json::from_str(flow["nodes"].as_str().unwrap()).unwrap();
    assert_eq!(nodes["version"], 2);
    assert_eq!(nodes["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(
        nodes["edges"],
        json!([{"from": "trigger", "port": 0, "to": "f"}, {"from": "f", "port": 0, "to": "a"}])
    );
    let (_, list) = send_request(&app, "GET", "/api/v1/flows", None, Some(&cookie), None).await;
    let listed: Value = serde_json::from_str(list["flows"][0]["nodes"].as_str().unwrap()).unwrap();
    assert_eq!(listed["version"], 2);

    // Objects instead of JSON text are taken too.
    let (status, _) = post_flow(
        &app,
        &cookie,
        json!({"id": "objects", "name": "o", "trigger_config": {"type": "manual"},
               "nodes": {"version": 2, "nodes": [], "edges": []}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn invalid_flows_are_refused_with_the_node_id() {
    let app = test_app_with_seeded_admin().await;
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let manual = json!({"type": "manual"});
    let graph = |nodes: Value, edges: Value| json!({"version": 2, "nodes": nodes, "edges": edges});

    let (status, body) = post_flow(
        &app,
        &cookie,
        flow_body(
            "bad_port",
            manual.clone(),
            graph(
                json!([{"id": "f", "type": "filter", "condition": "x > 1"}, {"id": "d", "type": "debug"}]),
                json!([{"from": "f", "port": 5, "to": "d"}]),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["node_id"], "f");
    assert!(body["error"].as_str().unwrap().contains("port 5"), "{body}");

    let (status, body) = post_flow(
        &app,
        &cookie,
        flow_body(
            "cycle",
            manual.clone(),
            graph(
                json!([{"id": "a", "type": "debug"}, {"id": "b", "type": "debug"}]),
                json!([{"from": "a", "to": "b"}, {"from": "b", "to": "a"}]),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().unwrap().contains("cycle"), "{body}");

    let (status, body) = post_flow(
        &app,
        &cookie,
        flow_body(
            "bad_config",
            manual.clone(),
            graph(
                json!([{"id": "s", "type": "switch", "rules": []}]),
                json!([]),
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["node_id"], "s");

    let (status, body) = post_flow(
        &app,
        &cookie,
        flow_body(
            "bad_trigger",
            json!({"type": "schedule", "cron": "every tuesday"}),
            graph(json!([]), json!([])),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["node_id"].is_null());

    let (status, _) = post_flow(&app, &cookie, flow_body("test", manual.clone(), json!([]))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "reserved id");

    let (status, _) = post_flow(&app, &cookie, flow_body("twice", manual.clone(), json!([]))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_flow(&app, &cookie, flow_body("twice", manual.clone(), json!([]))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // PUT validates too, and nothing invalid was stored.
    let (status, _) = send_request(
        &app,
        "PUT",
        "/api/v1/flows/twice",
        None,
        Some(&cookie),
        Some(flow_body(
            "twice",
            manual,
            graph(
                json!([{"id": "a", "type": "debug"}]),
                json!([{"from": "a", "to": "trigger"}]),
            ),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, list) = send_request(&app, "GET", "/api/v1/flows", None, Some(&cookie), None).await;
    assert_eq!(list["flows"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn viewers_read_flows_and_the_catalogue_but_do_not_change_them() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let viewer = user_cookie(&app, &storage, "viewer", "viewer").await;

    let (status, cat) = send_request(
        &app,
        "GET",
        "/api/v1/flows/node-types",
        None,
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let types: Vec<&str> = cat["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["type"].as_str().unwrap())
        .collect();
    for t in [
        "filter",
        "switch",
        "delay",
        "rate_limit",
        "on_change",
        "notify",
        "debug",
        "write_out",
    ] {
        assert!(types.contains(&t), "{t}");
    }
    let filter = &cat["nodes"].as_array().unwrap()[0];
    assert_eq!(filter["outputs"], json!(["pass", "reject"]));
    assert_eq!(filter["side_effect"], false);
    assert_eq!(filter["fields"][0]["key"], "condition");
    assert_eq!(filter["fields"][0]["kind"], "expression");
    let triggers: Vec<&str> = cat["triggers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["type"].as_str().unwrap())
        .collect();
    for t in [
        "tag_change",
        "alarm",
        "schedule",
        "webhook",
        "manual",
        "modbus",
        "mqtt",
        "timer",
    ] {
        assert!(triggers.contains(&t), "{t}");
    }

    let (status, _) = send_request(&app, "GET", "/api/v1/flows", None, Some(&viewer), None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send_request(
        &app,
        "GET",
        "/api/v1/flows/x/runs",
        None,
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post_flow(
        &app,
        &viewer,
        flow_body("v", json!({"type": "manual"}), json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows/test",
        None,
        Some(&viewer),
        Some(json!({"flow": {"trigger_config": {"type": "manual"}, "nodes": []}})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_runs_are_dry_and_real_runs_write_through_the_policy() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let tag = writable_tag(&storage).await;

    let nodes = json!({"version": 2,
    "nodes": [
        {"id": "w", "type": "write_out", "data_source_id": tag, "value_expr": "v"},
        // Nothing listens on port 9: a real request would fail.
        {"id": "h", "type": "http_out", "url": "http://127.0.0.1:9/hook", "headers": {"Authorization": "secret"}},
        {"id": "n", "type": "notify", "channel": "webhook", "to": "http://127.0.0.1:9/n", "message": "v is {{v}}"},
        {"id": "a", "type": "alarm_out", "severity": "warning", "source_key": "k", "message": "m"},
        {"id": "o", "type": "data_source_out", "value_field": "v", "source_key": "out"},
        {"id": "d", "type": "debug"},
    ],
    "edges": [
        {"from": "trigger", "to": "w"},
        {"from": "trigger", "to": "h"},
        {"from": "trigger", "to": "n"},
        {"from": "trigger", "to": "a"},
        {"from": "trigger", "to": "o"},
        {"from": "w", "to": "d"},
    ]});
    let trigger = json!({"type": "manual"});
    // The web editor sends both as JSON text.
    let (status, r) = send_request(
        &app,
        "POST",
        "/api/v1/flows/test",
        None,
        Some(&cookie),
        Some(json!({
            "flow": {"trigger_config": trigger.to_string(), "nodes": nodes.to_string()},
            "input": {"payload": {"v": 42}},
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{r}");
    assert_eq!(r["ok"], true, "{r}");
    let trace = r["trace"].as_array().unwrap();
    let entry = |id: &str| trace.iter().find(|t| t["node_id"] == id).unwrap().clone();
    for id in ["w", "h", "n", "a", "o"] {
        assert_eq!(entry(id)["side_effect_skipped"], true, "{id}");
    }
    assert_eq!(entry("d")["side_effect_skipped"], false);
    assert_eq!(entry("w")["output"]["would_write"]["value"], 42.0);
    assert_eq!(
        entry("h")["output"]["would_post"]["headers"],
        json!(["Authorization"])
    );
    assert_eq!(entry("n")["output"]["would_notify"]["message"], "v is 42");
    assert!(
        r["outputs_reached"]
            .as_array()
            .unwrap()
            .contains(&json!("d"))
    );
    assert!(storage.get_queued_commands().await.unwrap().is_empty());
    assert!(storage.list_alarms(None, 10).await.unwrap().is_empty());

    // Out of range: refused in the test run too, still nothing queued.
    let (_, r) = send_request(
        &app,
        "POST",
        "/api/v1/flows/test",
        None,
        Some(&cookie),
        Some(json!({"flow": {"trigger_config": trigger, "nodes": nodes}, "input": {"payload": {"v": 500}}})),
    )
    .await;
    assert_eq!(r["ok"], false);
    assert!(entry_error(&r, "w").contains("above maximum"), "{r}");
    assert!(storage.get_queued_commands().await.unwrap().is_empty());

    // A real run goes through enqueue_checked_write as flow:<id>.
    let write_only = json!({"version": 2,
        "nodes": [{"id": "w", "type": "write_out", "data_source_id": tag, "value_expr": "v"}],
        "edges": [{"from": "trigger", "to": "w"}]});
    let (status, _) = post_flow(
        &app,
        &cookie,
        flow_body("writer", json!({"type": "manual"}), write_only),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let run = |v: f64| {
        let app = app.clone();
        let cookie = cookie.clone();
        async move {
            send_request(
                &app,
                "POST",
                "/api/v1/flows/writer/run",
                None,
                Some(&cookie),
                Some(json!({"input": {"payload": {"v": v}}})),
            )
            .await
        }
    };
    let (status, r) = run(500.0).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(r["ok"], false);
    assert!(entry_error(&r, "w").contains("above maximum"), "{r}");
    assert!(storage.get_queued_commands().await.unwrap().is_empty());
    let (_, r) = run(55.0).await;
    assert_eq!(r["ok"], true, "{r}");
    let queued = storage.get_queued_commands().await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].created_by, "flow:writer");

    // Both runs are in the run log with their trace.
    let (_, runs) = send_request(
        &app,
        "GET",
        "/api/v1/flows/writer/runs",
        None,
        Some(&cookie),
        None,
    )
    .await;
    let runs = runs["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0]["success"], true);
    assert_eq!(runs[0]["trace"][0]["node_id"], "w");
    assert_eq!(runs[0]["trace"][0]["port"], 0);
    assert!(
        runs[1]["trace"][0]["error"]
            .as_str()
            .unwrap()
            .contains("above maximum")
    );
}

fn entry_error(r: &Value, id: &str) -> String {
    r["trace"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["node_id"] == id)
        .and_then(|t| t["error"].as_str())
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn webhook_needs_the_token_and_is_rate_limited() {
    let mut state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let engine = crate::flow_engine::engine::FlowEngine::new(storage.clone(), state.cache.clone());
    engine.spawn().await;
    state.flow_engine = Some(Arc::new(engine));
    let app = create_router(state);
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    let operator = user_cookie(&app, &storage, "op", "demo").await;

    let mut body = flow_body(
        "hooked",
        json!({"type": "webhook"}),
        json!({"version": 2, "nodes": [{"id": "d", "type": "debug"}], "edges": [{"from": "trigger", "to": "d"}]}),
    );
    body["enabled"] = json!(true);
    let (status, _) = post_flow(&app, &cookie, body).await;
    assert_eq!(status, StatusCode::OK);

    let hook = |token: Option<String>, body: &'static str| {
        let app = app.clone();
        async move {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/v1/flows/hooked/webhook")
                .header("content-type", "application/json");
            if let Some(t) = token {
                req = req.header("X-Flow-Token", t);
            }
            app.oneshot(req.body(Body::from(body)).unwrap())
                .await
                .unwrap()
                .status()
        }
    };

    // No token configured yet, none sent.
    assert_eq!(hook(None, "{}").await, StatusCode::UNAUTHORIZED);
    // Only an admin creates the token.
    let (status, _) = send_request(
        &app,
        "POST",
        "/api/v1/flows/hooked/webhook-token",
        None,
        Some(&operator),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, t) = send_request(
        &app,
        "POST",
        "/api/v1/flows/hooked/webhook-token",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = t["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 64);

    assert_eq!(hook(None, "{}").await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        hook(Some("wrong".into()), "{}").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        hook(Some(token.clone()), "not json").await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        hook(Some(token.clone()), r#"{"order": 7}"#).await,
        StatusCode::ACCEPTED
    );
    let mut runs = Vec::new();
    for _ in 0..40 {
        runs = storage.list_flow_runs("hooked", 5).await.unwrap();
        if !runs.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].trace.as_ref().unwrap()[0]["input"]["order"], 7);

    // A new token replaces the old one.
    let (_, t2) = send_request(
        &app,
        "POST",
        "/api/v1/flows/hooked/webhook-token",
        None,
        Some(&cookie),
        None,
    )
    .await;
    assert_ne!(t2["token"], t["token"]);
    assert_eq!(hook(Some(token), "{}").await, StatusCode::UNAUTHORIZED);

    // Every call counts toward the per-minute limit, wrong tokens too.
    let mut limited = false;
    for _ in 0..crate::api::flows::WEBHOOK_MAX_PER_MINUTE {
        if hook(Some("wrong".into()), "{}").await == StatusCode::TOO_MANY_REQUESTS {
            limited = true;
            break;
        }
    }
    assert!(limited);
}
