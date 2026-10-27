//! MCP endpoint (`POST /mcp`), driven in-process through the router.

use super::*;
use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

const WRITE_TOOLS: &[&str] = &[
    "create_modbus_device",
    "delete_device",
    "create_tag",
    "update_tag",
    "delete_tag",
    "acknowledge_alarm",
    "create_screen",
    "add_tag_to_screen",
    "create_flow",
    "start_flow",
    "stop_flow",
    "write_tag",
    "instantiate_device_template",
    "save_device_as_template",
];

const READ_TOOLS: &[&str] = &[
    "server_info",
    "list_devices",
    "list_tags",
    "read_live_values",
    "read_history",
    "list_alarms",
    "alarm_summary",
    "list_screens",
    "list_flows",
    "get_command_status",
    "list_device_templates",
    "get_device_template",
    "list_template_catalog",
];

/// Admin sessions only; API and integration keys are refused.
const ADMIN_TOOLS: &[&str] = &[
    "import_catalog_template",
    "project_export_yaml",
    "project_plan_import",
    "project_apply_import",
];

/// Admin-only tools that change state (the others only read).
const ADMIN_WRITE_TOOLS: &[&str] = &["import_catalog_template", "project_apply_import"];

enum Auth<'a> {
    None,
    XApiKey(&'a str),
    Bearer(&'a str),
    Cookie(&'a str),
}

async fn post_raw(
    app: &Router,
    auth: Auth<'_>,
    extra: &[(&str, &str)],
    content_type: Option<&str>,
    body: String,
) -> (StatusCode, http::HeaderMap, Value) {
    let mut builder = Request::builder().method("POST").uri("/mcp");
    if let Some(ct) = content_type {
        builder = builder.header("content-type", ct);
    }
    builder = builder.header("accept", "application/json, text/event-stream");
    match auth {
        Auth::None => {}
        Auth::XApiKey(k) => builder = builder.header("X-API-Key", k),
        Auth::Bearer(k) => builder = builder.header("Authorization", format!("Bearer {k}")),
        Auth::Cookie(c) => builder = builder.header("cookie", c),
    }
    for (k, v) in extra {
        builder = builder.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, json)
}

/// One JSON-RPC request; returns the HTTP status and the JSON-RPC response.
async fn rpc(app: &Router, auth: Auth<'_>, method: &str, params: Value) -> (StatusCode, Value) {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let (status, _, json) =
        post_raw(app, auth, &[], Some("application/json"), body.to_string()).await;
    (status, json)
}

/// Call a tool and return the JSON-RPC response.
async fn call(app: &Router, auth: Auth<'_>, tool: &str, args: Value) -> Value {
    let (status, resp) = rpc(
        app,
        auth,
        "tools/call",
        json!({"name": tool, "arguments": args}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resp}");
    resp
}

/// A successful tool call's structured content.
async fn ok(app: &Router, auth: Auth<'_>, tool: &str, args: Value) -> Value {
    let resp = call(app, auth, tool, args).await;
    assert!(resp["error"].is_null(), "{tool}: {resp}");
    assert_eq!(resp["result"]["isError"], false, "{tool}: {resp}");
    resp["result"]["structuredContent"].clone()
}

/// A tool call that ran and failed; returns the error text.
async fn tool_err(app: &Router, auth: Auth<'_>, tool: &str, args: Value) -> String {
    let resp = call(app, auth, tool, args).await;
    assert_eq!(resp["result"]["isError"], true, "{tool}: {resp}");
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

async fn make_key(app: &Router, cookie: &str, role: &str) -> (i64, String) {
    let (status, json) = send_request(
        app,
        "POST",
        "/api/v1/api-keys",
        None,
        Some(&format!("voltrus_token={cookie}")),
        Some(json!({"name": format!("mcp-{role}"), "role": role})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    (
        json["id"].as_i64().unwrap(),
        json["key"].as_str().unwrap().to_string(),
    )
}

struct Fixture {
    app: Router,
    storage: Storage,
    api_key: String,
    integration_key: String,
    cookie: String,
}

impl Fixture {
    /// Cookie header value of the admin session.
    fn admin_cookie(&self) -> String {
        format!("voltrus_token={}", self.cookie)
    }
}

async fn fixture() -> Fixture {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = admin_cookie(&app).await;
    let (_, api_key) = make_key(&app, &cookie, "api").await;
    let (_, integration_key) = make_key(&app, &cookie, "integration").await;
    Fixture {
        app,
        storage,
        api_key,
        integration_key,
        cookie,
    }
}

#[tokio::test]
async fn mcp_unauthenticated_is_401() {
    let app = test_app_with_seeded_admin().await;
    let (status, headers, _) = post_raw(
        &app,
        Auth::None,
        &[],
        Some("application/json"),
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.get("www-authenticate").is_some());

    let (status, _) = rpc(&app, Auth::XApiKey("not-a-key"), "ping", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = rpc(&app, Auth::Bearer("not-a-key"), "ping", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn mcp_initialize_handshake() {
    let f = fixture().await;
    let (status, resp) = rpc(
        &f.app,
        Auth::Bearer(&f.api_key),
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(resp["result"]["serverInfo"]["name"], "voltrus-scada");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
    assert!(
        resp["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Voltrus")
    );

    // An older supported revision is echoed, an unknown one answered with the newest.
    let (_, resp) = rpc(
        &f.app,
        Auth::Bearer(&f.api_key),
        "initialize",
        json!({"protocolVersion": "2024-11-05"}),
    )
    .await;
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    let (_, resp) = rpc(
        &f.app,
        Auth::Bearer(&f.api_key),
        "initialize",
        json!({"protocolVersion": "1999-01-01"}),
    )
    .await;
    assert_eq!(resp["result"]["protocolVersion"], "2025-06-18");

    // The initialized notification carries no id and gets no body.
    let (status, _, body) = post_raw(
        &f.app,
        Auth::Bearer(&f.api_key),
        &[],
        Some("application/json"),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(body.is_null());

    let (_, resp) = rpc(&f.app, Auth::Bearer(&f.api_key), "ping", json!({})).await;
    assert_eq!(resp["result"], json!({}));
}

#[tokio::test]
async fn mcp_protocol_errors() {
    let f = fixture().await;
    let key = Auth::Bearer(&f.api_key);

    let (_, resp) = rpc(
        &f.app,
        Auth::Bearer(&f.api_key),
        "no/such/method",
        json!({}),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32601);

    let (status, resp) = rpc(
        &f.app,
        Auth::Bearer(&f.api_key),
        "tools/call",
        json!({"name": "nope"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["error"]["code"], -32602);

    let (status, _, resp) = post_raw(
        &f.app,
        key,
        &[],
        Some("application/json"),
        "{not json".into(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(resp["error"]["code"], -32700);

    let (status, _, _) = post_raw(
        &f.app,
        Auth::Bearer(&f.api_key),
        &[],
        Some("text/plain"),
        "{}".into(),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Browser origins that are not the server's own are refused.
    let (status, _, _) = post_raw(
        &f.app,
        Auth::Bearer(&f.api_key),
        &[
            ("origin", "https://evil.example"),
            ("host", "scada.local:3000"),
        ],
        Some("application/json"),
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = post_raw(
        &f.app,
        Auth::Bearer(&f.api_key),
        &[
            ("origin", "http://scada.local:3000"),
            ("host", "scada.local:3000"),
        ],
        Some("application/json"),
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // No SSE stream: GET answers 405.
    let resp = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/mcp")
                .header("X-API-Key", &f.api_key)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

    // Batch (revision 2025-03-26): one reply per request, none for notifications.
    let batch = json!([
        {"jsonrpc": "2.0", "id": "a", "method": "ping"},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": "b", "method": "ping"}
    ]);
    let (status, _, resp) = post_raw(
        &f.app,
        Auth::Bearer(&f.api_key),
        &[],
        Some("application/json"),
        batch.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn mcp_tools_list_has_every_tool_with_a_schema() {
    let f = fixture().await;
    let (_, resp) = rpc(&f.app, Auth::Bearer(&f.api_key), "tools/list", json!({})).await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in READ_TOOLS.iter().chain(WRITE_TOOLS).chain(ADMIN_TOOLS) {
        assert!(
            names.contains(expected),
            "missing tool {expected}: {names:?}"
        );
    }
    assert_eq!(
        names.len(),
        READ_TOOLS.len() + WRITE_TOOLS.len() + ADMIN_TOOLS.len(),
        "{names:?}"
    );
    // Users, roles, API keys, licence and security have no tool; the admin tools
    // are the project and catalog ones listed above.
    for name in &names {
        for forbidden in ["user", "role", "api_key", "license", "password"] {
            assert!(!name.contains(forbidden), "{name} must not be exposed");
        }
    }
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
        assert!(tool["inputSchema"]["properties"].is_object(), "{name}");
        assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{name}");
        assert!(tool["description"].as_str().unwrap().len() > 40, "{name}");
        let writes = WRITE_TOOLS.contains(&name) || ADMIN_WRITE_TOOLS.contains(&name);
        assert_eq!(tool["annotations"]["readOnlyHint"], !writes, "{name}");
    }
    // required[] names only declared properties.
    for tool in tools {
        let props = tool["inputSchema"]["properties"].as_object().unwrap();
        for r in tool["inputSchema"]["required"]
            .as_array()
            .into_iter()
            .flatten()
        {
            assert!(
                props.contains_key(r.as_str().unwrap()),
                "{}: {r}",
                tool["name"]
            );
        }
    }
}

#[tokio::test]
async fn mcp_read_tools_work_with_an_api_role_key() {
    let f = fixture().await;
    let info = ok(&f.app, Auth::Bearer(&f.api_key), "server_info", json!({})).await;
    assert_eq!(info["caller"]["role"], "api");
    assert_eq!(info["caller"]["can_write"], false);
    assert_eq!(info["version"], env!("CARGO_PKG_VERSION"));

    // Same key through X-API-Key.
    let tags = ok(&f.app, Auth::XApiKey(&f.api_key), "list_tags", json!({})).await;
    assert_eq!(tags["tags"], json!([]));
    ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_alarms",
        json!({"status": "active"}),
    )
    .await;
    ok(&f.app, Auth::Bearer(&f.api_key), "alarm_summary", json!({})).await;
    ok(&f.app, Auth::Bearer(&f.api_key), "list_screens", json!({})).await;
    ok(&f.app, Auth::Bearer(&f.api_key), "list_flows", json!({})).await;
    ok(&f.app, Auth::Bearer(&f.api_key), "list_devices", json!({})).await;
}

#[tokio::test]
async fn mcp_write_tool_refused_for_api_key_allowed_for_integration_key() {
    let f = fixture().await;
    let args = json!({"key": "mcp_tag", "name": "mcp_tag"});

    let resp = call(&f.app, Auth::Bearer(&f.api_key), "create_tag", args.clone()).await;
    assert_eq!(resp["error"]["code"], -32003, "{resp}");
    assert!(
        resp["error"]["message"]
            .as_str()
            .unwrap()
            .contains("integration")
    );
    // Nothing was created.
    let tags = ok(&f.app, Auth::Bearer(&f.api_key), "list_tags", json!({})).await;
    assert_eq!(tags["count"], 0);

    let created = ok(&f.app, Auth::Bearer(&f.integration_key), "create_tag", args).await;
    assert_eq!(created["tag"]["key"], "mcp_tag");
    let tags = ok(&f.app, Auth::Bearer(&f.api_key), "list_tags", json!({})).await;
    assert_eq!(tags["count"], 1);

    // Session cookies keep their role: admin may write.
    let cookie = format!("voltrus_token={}", f.cookie);
    let info = ok(&f.app, Auth::Cookie(&cookie), "server_info", json!({})).await;
    assert_eq!(info["caller"]["role"], "admin");
    ok(
        &f.app,
        Auth::Cookie(&cookie),
        "create_screen",
        json!({"name": "Cookie screen"}),
    )
    .await;
}

#[tokio::test]
async fn mcp_write_calls_are_audit_logged() {
    let mut state = seeded_admin_app_state().await;
    let (logger, mut rx) = crate::security::AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state);
    let cookie = admin_cookie(&app).await;
    let (key_id, api_key) = make_key(&app, &cookie, "api").await;
    let (int_id, int_key) = make_key(&app, &cookie, "integration").await;

    ok(
        &app,
        Auth::Bearer(&int_key),
        "create_screen",
        json!({"name": "Audited"}),
    )
    .await;
    let denied = call(
        &app,
        Auth::Bearer(&api_key),
        "create_screen",
        json!({"name": "Nope"}),
    )
    .await;
    assert_eq!(denied["error"]["code"], -32003);
    ok(&app, Auth::Bearer(&int_key), "list_screens", json!({})).await;

    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let write = events
        .iter()
        .find(|e| e.action == "mcp:create_screen")
        .unwrap_or_else(|| panic!("no mcp audit event in {events:?}"));
    assert_eq!(write.username, format!("api_key:{int_id}"));
    assert!(write.details.as_deref().unwrap().contains("outcome=ok"));
    let refused = events
        .iter()
        .find(|e| e.action == "mcp:create_screen:denied")
        .expect("denied call is logged");
    assert_eq!(refused.username, format!("api_key:{key_id}"));
    // Read tools are not audited.
    assert!(!events.iter().any(|e| e.action == "mcp:list_screens"));
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn mcp_create_modbus_device_then_list_devices() {
    let f = fixture().await;
    let key = &f.integration_key;
    let created = ok(
        &f.app,
        Auth::Bearer(key),
        "create_modbus_device",
        json!({
            "id": "plts_mcp",
            "name": "PLTS via MCP",
            "ip": "127.0.0.1",
            "port": 1502,
            "unit_id": 3,
            "interval_ms": 2000,
            "registers": [
                {"name": "voltage", "address": 40001, "register_type": "float32"},
                {"name": "state", "offset": 12, "register_type": "uint16", "register_area": "coil"}
            ]
        }),
    )
    .await;
    assert_eq!(created["id"], "plts_mcp");
    assert_eq!(created["registers"], 2);

    let listed = ok(&f.app, Auth::Bearer(&f.api_key), "list_devices", json!({})).await;
    let device = listed["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == "plts_mcp")
        .unwrap_or_else(|| panic!("device not listed: {listed}"));
    assert_eq!(device["ip"], "127.0.0.1");
    assert_eq!(device["port"], 1502);
    assert_eq!(device["unit_id"], 3);
    assert_eq!(device["register_count"], 2);
    assert_eq!(device["protocol"], "modbus");

    // One device with its register map.
    let one = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_devices",
        json!({"id": "plts_mcp"}),
    )
    .await;
    assert_eq!(one["registers"].as_array().unwrap().len(), 2);

    // Mistakes are tool errors, not HTTP failures.
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_modbus_device",
        json!({"id": "plts_mcp", "ip": "127.0.0.1"}),
    )
    .await;
    assert!(e.contains("already exists"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_modbus_device",
        json!({"id": "x", "ip": "h", "unit_id": 300}),
    )
    .await;
    assert!(e.contains("invalid arguments"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_modbus_device",
        json!({"id": "x", "ip": "h", "registers": [{"name": "r"}]}),
    )
    .await;
    assert!(e.contains("address or offset"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_modbus_device",
        json!({"id": "x", "ip": "h", "bogus": 1}),
    )
    .await;
    assert!(e.contains("unknown field"), "{e}");

    ok(
        &f.app,
        Auth::Bearer(key),
        "delete_device",
        json!({"id": "plts_mcp"}),
    )
    .await;
    let listed = ok(&f.app, Auth::Bearer(&f.api_key), "list_devices", json!({})).await;
    assert!(
        listed["devices"]
            .as_array()
            .unwrap()
            .iter()
            .all(|d| d["id"] != "plts_mcp")
    );
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "delete_device",
        json!({"id": "plts_mcp"}),
    )
    .await;
    assert!(e.contains("not found"), "{e}");
}

#[tokio::test]
async fn mcp_tag_lifecycle_live_values_and_history() {
    let f = fixture().await;
    let key = &f.integration_key;
    let created = ok(
        &f.app,
        Auth::Bearer(key),
        "create_tag",
        json!({
            "key": "boiler_temp", "name": "Boiler temp", "unit": "degC",
            "min_val": 0, "max_val": 200, "warn_val": 150, "crit_val": 180, "base_val": 90,
            "source_type": "constant", "source_config": {"low_warn": 20},
            "path": "plant/boiler", "on_delay_secs": 5
        }),
    )
    .await;
    let id = created["tag"]["id"].as_i64().unwrap();
    assert_eq!(created["tag"]["crit_val"], 180.0);
    assert_eq!(created["tag"]["path"], "plant/boiler");
    assert_eq!(created["tag"]["on_delay_secs"], 5);
    let config: Value =
        serde_json::from_str(created["tag"]["source_config"].as_str().unwrap()).unwrap();
    assert_eq!(config["low_warn"], 20);

    let listed = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_tags",
        json!({"query": "BOILER"}),
    )
    .await;
    assert_eq!(listed["total"], 1);
    assert_eq!(listed["tags"][0]["key"], "boiler_temp");
    let none = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_tags",
        json!({"query": "zzz"}),
    )
    .await;
    assert_eq!(none["count"], 0);

    // Live values: the constant tag reports its base value, normal status.
    let live = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "read_live_values",
        json!({"keys": ["boiler_temp", "ghost"]}),
    )
    .await;
    assert_eq!(live["count"], 1);
    assert_eq!(live["values"][0]["value"], 90.0);
    assert_eq!(live["values"][0]["status"], "normal");
    assert_eq!(live["unknown_keys"], json!(["ghost"]));

    // Limits updated, status follows.
    let updated = ok(
        &f.app,
        Auth::Bearer(key),
        "update_tag",
        json!({"id": id, "warn_val": 80, "unit": "C"}),
    )
    .await;
    assert_eq!(updated["tag"]["warn_val"], 80.0);
    assert_eq!(updated["tag"]["unit"], "C");
    let live = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "read_live_values",
        json!({}),
    )
    .await;
    assert_eq!(live["values"][0]["status"], "warning");

    // History: preset and absolute windows, all three aggregations.
    for agg in ["avg", "minmax", "sampling"] {
        let h = ok(
            &f.app,
            Auth::Bearer(&f.api_key),
            "read_history",
            json!({"keys": ["boiler_temp"], "range": "1h", "agg": agg}),
        )
        .await;
        assert_eq!(
            h["data"]["boiler_temp"]["count"].as_i64().unwrap() > 0,
            true,
            "{agg}: {h}"
        );
    }
    let now = crate::timeutil::now_secs();
    let h = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "read_history",
        json!({"keys": ["boiler_temp", "ghost"], "from": now - 600, "to": now}),
    )
    .await;
    assert!(
        h["data"]["boiler_temp"]["count"].as_i64().unwrap() > 0,
        "{h}"
    );
    assert_eq!(h["unknown_keys"], json!(["ghost"]));
    let e = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "read_history",
        json!({"keys": ["boiler_temp"], "from": now}),
    )
    .await;
    assert!(e.contains("from and to"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "read_history",
        json!({"keys": ["boiler_temp"], "range": "5y"}),
    )
    .await;
    assert!(e.contains("range must be"), "{e}");

    // Validation.
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_tag",
        json!({"key": "bad", "source_config": "not json"}),
    )
    .await;
    assert!(e.contains("source_config"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_tag",
        json!({"key": "bad", "min_val": 5, "max_val": 1}),
    )
    .await;
    assert!(e.contains("min_val"), "{e}");
    let e = tool_err(&f.app, Auth::Bearer(key), "update_tag", json!({"id": id})).await;
    assert!(e.contains("nothing to update"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "update_tag",
        json!({"id": 9999, "unit": "x"}),
    )
    .await;
    assert!(e.contains("not found"), "{e}");

    ok(&f.app, Auth::Bearer(key), "delete_tag", json!({"id": id})).await;
    let listed = ok(&f.app, Auth::Bearer(&f.api_key), "list_tags", json!({})).await;
    assert_eq!(listed["count"], 0);
    let e = tool_err(&f.app, Auth::Bearer(key), "delete_tag", json!({"id": id})).await;
    assert!(e.contains("not found"), "{e}");
}

#[tokio::test]
async fn mcp_create_tag_honours_the_naming_convention() {
    let f = fixture().await;
    let (status, _) = send_request(
        &f.app,
        "PUT",
        "/api/v1/settings/naming-convention",
        None,
        Some(&format!("voltrus_token={}", f.cookie)),
        Some(json!({"template": "custom", "custom_regex": "^[A-Z]{2}[0-9]{3}$"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let e = tool_err(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "create_tag",
        json!({"key": "k", "name": "bad name"}),
    )
    .await;
    assert!(e.contains("HTTP 400"), "{e}");
    ok(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "create_tag",
        json!({"key": "k", "name": "TT101"}),
    )
    .await;
}

#[tokio::test]
async fn mcp_write_tag_respects_write_policy() {
    let f = fixture().await;
    let key = &f.integration_key;
    let created = ok(
        &f.app,
        Auth::Bearer(key),
        "create_tag",
        json!({"key": "valve", "source_type": "modbus", "device_id": "plc_1"}),
    )
    .await;
    let id = created["tag"]["id"].as_i64().unwrap();

    // Not writable by default.
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": id, "value": 1}),
    )
    .await;
    assert!(e.contains("not writable") && e.contains("403"), "{e}");

    // The operator makes it writable, with a range and a reason requirement.
    let (status, _) = send_request(
        &f.app,
        "PATCH",
        &format!("/api/v1/data-sources/{id}/write-policy"),
        None,
        Some(&format!("voltrus_token={}", f.cookie)),
        Some(json!({"writable": true, "write_min": 0, "write_max": 100, "write_requires_reason": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": id, "value": 10}),
    )
    .await;
    assert!(e.contains("Reason is required"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": id, "value": 101, "reason": "test"}),
    )
    .await;
    assert!(e.contains("above maximum"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": id, "value": "ten", "reason": "test"}),
    )
    .await;
    assert!(e.contains("invalid arguments"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": 9999, "value": 1}),
    )
    .await;
    assert!(e.contains("not found"), "{e}");

    // Read-only keys cannot write at all.
    let resp = call(
        &f.app,
        Auth::Bearer(&f.api_key),
        "write_tag",
        json!({"tag_id": id, "value": 10, "reason": "test"}),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32003);

    let queued = ok(
        &f.app,
        Auth::Bearer(key),
        "write_tag",
        json!({"tag_id": id, "value": 42.5, "reason": "commissioning"}),
    )
    .await;
    assert_eq!(queued["status"], "pending");
    assert_eq!(queued["value"], 42.5);
    let command_id = queued["command_id"].as_str().unwrap();

    let status = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_command_status",
        json!({"command_id": command_id}),
    )
    .await;
    assert_eq!(status["command"]["data_source_id"], id);
    assert_eq!(status["command"]["value"], "42.5");
    assert!(
        status["command"]["created_by"]
            .as_str()
            .unwrap()
            .starts_with("api_key:")
    );
    let e = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_command_status",
        json!({"command_id": "nope"}),
    )
    .await;
    assert!(e.contains("404"), "{e}");
}

#[tokio::test]
async fn mcp_alarms_screens_and_flows() {
    let f = fixture().await;
    let key = &f.integration_key;

    // Screens
    let screen = ok(
        &f.app,
        Auth::Bearer(key),
        "create_screen",
        json!({"name": "Boiler house"}),
    )
    .await;
    assert_eq!(screen["screen"]["id"], "boiler_house");
    let tag = ok(
        &f.app,
        Auth::Bearer(key),
        "create_tag",
        json!({"key": "p1"}),
    )
    .await;
    let tag_id = tag["tag"]["id"].as_i64().unwrap();
    let added = ok(
        &f.app,
        Auth::Bearer(key),
        "add_tag_to_screen",
        json!({"screen_id": "boiler_house", "tag_id": tag_id}),
    )
    .await;
    assert_eq!(added["already_present"], false);
    let again = ok(
        &f.app,
        Auth::Bearer(key),
        "add_tag_to_screen",
        json!({"screen_id": "boiler_house", "tag_id": tag_id}),
    )
    .await;
    assert_eq!(again["already_present"], true);
    let on_screen = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_tags",
        json!({"screen": "boiler_house"}),
    )
    .await;
    assert_eq!(on_screen["count"], 1);
    let screens = ok(&f.app, Auth::Bearer(&f.api_key), "list_screens", json!({})).await;
    assert!(
        screens["screens"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["id"] == "boiler_house")
    );
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "add_tag_to_screen",
        json!({"screen_id": "nope", "tag_id": tag_id}),
    )
    .await;
    assert!(e.contains("screen 'nope' not found"), "{e}");
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "add_tag_to_screen",
        json!({"screen_id": "boiler_house", "tag_id": 9999}),
    )
    .await;
    assert!(e.contains("tag 9999 not found"), "{e}");

    // Flows
    let flow = ok(
        &f.app,
        Auth::Bearer(key),
        "create_flow",
        json!({
            "name": "High temp",
            "trigger": {"type": "modbus", "device_id": "*"},
            "nodes": [
                {"type": "threshold", "id": "t1", "field": "temp", "rules": [{"gt": 80, "set": {"hot": true}}]},
                {"type": "alarm_out", "id": "a1", "severity": "warning", "source_key": "temp", "message": "hot"}
            ]
        }),
    )
    .await;
    assert_eq!(flow["id"], "high_temp");
    assert_eq!(flow["enabled"], false);
    let flows = ok(&f.app, Auth::Bearer(&f.api_key), "list_flows", json!({})).await;
    assert_eq!(flows["count"], 1);
    assert_eq!(flows["flows"][0]["enabled"], false);

    let started = ok(
        &f.app,
        Auth::Bearer(key),
        "start_flow",
        json!({"id": "high_temp"}),
    )
    .await;
    assert_eq!(started["enabled"], true);
    let flows = ok(&f.app, Auth::Bearer(&f.api_key), "list_flows", json!({})).await;
    assert_eq!(flows["flows"][0]["enabled"], true);
    ok(
        &f.app,
        Auth::Bearer(key),
        "stop_flow",
        json!({"id": "high_temp"}),
    )
    .await;
    let flows = ok(&f.app, Auth::Bearer(&f.api_key), "list_flows", json!({})).await;
    assert_eq!(flows["flows"][0]["enabled"], false);
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "start_flow",
        json!({"id": "ghost"}),
    )
    .await;
    assert!(e.contains("not found"), "{e}");

    // A flow that would not run is refused before it is stored.
    let e = tool_err(
        &f.app,
        Auth::Bearer(key),
        "create_flow",
        json!({"name": "bad", "trigger": {"type": "nope"}, "nodes": []}),
    )
    .await;
    assert!(e.contains("invalid trigger"), "{e}");
    let e = tool_err(&f.app, Auth::Bearer(key), "create_flow", json!({"name": "bad", "trigger": {"type": "timer", "interval_ms": 1000}, "nodes": [{"type": "filter", "id": "x"}]})).await;
    assert!(e.contains("invalid nodes"), "{e}");
    let e = tool_err(&f.app, Auth::Bearer(key), "create_flow", json!({"name": "dup", "trigger": {"type": "timer", "interval_ms": 1000}, "nodes": [{"type": "dsl", "id": "x"}, {"type": "dsl", "id": "x"}]})).await;
    assert!(e.contains("unique"), "{e}");
    let flows = ok(&f.app, Auth::Bearer(&f.api_key), "list_flows", json!({})).await;
    assert_eq!(flows["count"], 1);
}

#[tokio::test]
async fn mcp_alarms_list_and_acknowledge() {
    let state = seeded_admin_app_state().await;
    let storage = state.storage.clone();
    let app = create_router(state);
    let cookie = admin_cookie(&app).await;
    let (_, api_key) = make_key(&app, &cookie, "api").await;
    let (int_id, int_key) = make_key(&app, &cookie, "integration").await;

    let id = storage
        .create_alarm(
            "boiler_temp",
            "critical",
            185.0,
            180.0,
            "Boiler too hot",
            0.0,
        )
        .await
        .unwrap();

    let active = ok(
        &app,
        Auth::Bearer(&api_key),
        "list_alarms",
        json!({"status": "active"}),
    )
    .await;
    assert_eq!(active["count"], 1);
    assert_eq!(active["alarms"][0]["id"], id);
    assert_eq!(active["alarms"][0]["message"], "Boiler too hot");
    let summary = ok(&app, Auth::Bearer(&api_key), "alarm_summary", json!({})).await;
    assert_eq!(summary["summary"]["active_critical"], 1);

    let resp = call(
        &app,
        Auth::Bearer(&api_key),
        "acknowledge_alarm",
        json!({"id": id}),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32003);
    let e = tool_err(
        &app,
        Auth::Bearer(&int_key),
        "acknowledge_alarm",
        json!({"id": 424242}),
    )
    .await;
    assert!(e.contains("not found"), "{e}");

    ok(
        &app,
        Auth::Bearer(&int_key),
        "acknowledge_alarm",
        json!({"id": id}),
    )
    .await;
    let active = ok(
        &app,
        Auth::Bearer(&api_key),
        "list_alarms",
        json!({"status": "active"}),
    )
    .await;
    assert_eq!(active["count"], 0);
    let acked = ok(
        &app,
        Auth::Bearer(&api_key),
        "list_alarms",
        json!({"status": "acknowledged", "priority": "critical"}),
    )
    .await;
    assert_eq!(acked["count"], 1);
    assert_eq!(
        acked["alarms"][0]["acknowledged_by"],
        format!("api_key:{int_id}")
    );
}

// ── Device templates ──

const BUILTIN_PM5560: &str = "schneider-electric-pm5560";

#[tokio::test]
async fn mcp_template_search_and_get_work_with_an_api_key() {
    let f = fixture().await;
    let key = Auth::Bearer(&f.api_key);

    let all = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_device_templates",
        json!({}),
    )
    .await;
    assert!(all["count"].as_u64().unwrap() >= 1, "{all}");
    let found = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_device_templates",
        json!({"q": "pm5560", "protocol": "modbus", "source": "builtin"}),
    )
    .await;
    let row = found["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["template_id"] == BUILTIN_PM5560)
        .unwrap_or_else(|| panic!("pm5560 not listed: {found}"));
    assert_eq!(row["source"], "builtin");
    let none = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_device_templates",
        json!({"manufacturer": "No Such Maker"}),
    )
    .await;
    assert_eq!(none["count"], 0);

    let t = ok(
        &f.app,
        key,
        "get_device_template",
        json!({"template_id": BUILTIN_PM5560}),
    )
    .await;
    assert_eq!(t["template_id"], BUILTIN_PM5560);
    assert_eq!(t["read_only"], true);
    assert!(!t["registers"].as_array().unwrap().is_empty());

    let missing = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_device_template",
        json!({"template_id": "no-such-template"}),
    )
    .await;
    assert!(missing.contains("No device template"), "{missing}");
    let bad = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_device_templates",
        json!({"colour": "red"}),
    )
    .await;
    assert!(bad.contains("invalid arguments"), "{bad}");
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn mcp_instantiate_template_with_an_integration_key_and_save_it_back() {
    let f = fixture().await;
    let args = json!({
        "template_id": BUILTIN_PM5560, "name": "Grid meter", "ip": "127.0.0.1", "port": 1, "unit_id": 3
    });

    // A read-only key may look, not create.
    let refused = call(
        &f.app,
        Auth::Bearer(&f.api_key),
        "instantiate_device_template",
        args.clone(),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32003, "{refused}");
    assert_eq!(f.storage.list_modbus_devices().await.unwrap().len(), 0);

    let template = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_device_template",
        json!({"template_id": BUILTIN_PM5560}),
    )
    .await;
    let registers = template["registers"].as_array().unwrap().len();

    let made = ok(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "instantiate_device_template",
        args.clone(),
    )
    .await;
    assert_eq!(made["id"], "grid-meter", "{made}");
    assert_eq!(made["protocol"], "modbus");
    assert_eq!(made["item_count"], registers);

    // It is a real device with the template's registers and the overrides.
    let device = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_devices",
        json!({"id": "grid-meter"}),
    )
    .await;
    assert_eq!(device["registers"].as_array().unwrap().len(), registers);
    assert_eq!(device["device"]["unit_id"], 3);
    assert_eq!(
        device["registers"][0]["name"],
        template["registers"][0]["name"]
    );

    // The same id again is the route's conflict, reported as a tool error.
    let again = tool_err(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "instantiate_device_template",
        args,
    )
    .await;
    assert!(
        again.contains("already exists") && again.contains("409"),
        "{again}"
    );
    let unknown = tool_err(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "instantiate_device_template",
        json!({"template_id": "no-such-template", "name": "X", "ip": "127.0.0.1"}),
    )
    .await;
    assert!(unknown.contains("No device template"), "{unknown}");

    // Save the configured device as a local template, then find it.
    let save = json!({
        "device_id": "grid-meter", "manufacturer": "Schneider Electric", "model": "PM5560 on site",
        "version": "1.0.1", "verified": true, "quirks": ["Port 1 is a test port"]
    });
    let refused = call(
        &f.app,
        Auth::Bearer(&f.api_key),
        "save_device_as_template",
        save.clone(),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32003, "{refused}");
    let saved = ok(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "save_device_as_template",
        save.clone(),
    )
    .await;
    assert_eq!(saved["source"], "local");
    assert_eq!(saved["version"], "1.0.1");
    let id = saved["template_id"].as_str().unwrap().to_string();

    let listed = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_device_templates",
        json!({"source": "local"}),
    )
    .await;
    assert_eq!(listed["count"], 1, "{listed}");
    let t = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_device_template",
        json!({"template_id": id, "source": "local"}),
    )
    .await;
    assert_eq!(t["registers"].as_array().unwrap().len(), registers);
    assert_eq!(t["verified"], true);
    assert_eq!(t["read_only"], false);
    let dup = tool_err(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "save_device_as_template",
        save,
    )
    .await;
    assert!(dup.contains("already exists"), "{dup}");
}

#[cfg(feature = "bacnet")]
#[tokio::test]
async fn mcp_bacnet_templates_keep_the_admin_rule_of_the_bacnet_routes() {
    let f = fixture().await;
    let (status, body) = send_request(
        &f.app,
        "POST",
        "/api/v1/device-templates",
        None,
        Some(&f.admin_cookie()),
        Some(json!({
            "format": "voltrus-device-template/1", "template_id": "ahu-1", "manufacturer": "Globex",
            "model": "AHU", "protocol": "bacnet", "version": "1.0.0",
            "objects": [{"object_type": "analogInput", "object_instance": 1, "name": "sat"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let args = json!({"template_id": "ahu-1", "name": "AHU 1", "ip": "10.0.0.5"});
    let refused = tool_err(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "instantiate_device_template",
        args.clone(),
    )
    .await;
    assert!(
        refused.contains("Admin access required") && refused.contains("403"),
        "{refused}"
    );
    assert_eq!(f.storage.list_bacnet_devices().await.unwrap().len(), 0);

    let made = ok(
        &f.app,
        Auth::Cookie(&f.admin_cookie()),
        "instantiate_device_template",
        args,
    )
    .await;
    assert_eq!(made["protocol"], "bacnet", "{made}");
    assert_eq!(made["item_count"], 1);
}

#[tokio::test]
async fn mcp_catalog_import_is_refused_for_keys_and_works_for_an_admin() {
    use super::device_templates::{entry, modbus_template, serve};
    use crate::device_templates::catalog::{SETTING_CATALOG_URL, sha256_hex};

    let f = fixture().await;
    let template = modbus_template("acme-cat", "1.0.0");
    let bytes = serde_json::to_vec_pretty(&template).unwrap();
    let index = json!({
        "format": "voltrus-template-catalog/1",
        "templates": [entry(&template, "templates/acme-cat.json", &sha256_hex(&bytes))]
    });
    let base = serve(std::collections::HashMap::from([
        (
            "/cat/index.json".to_string(),
            serde_json::to_vec(&index).unwrap(),
        ),
        ("/cat/templates/acme-cat.json".to_string(), bytes),
    ]))
    .await;
    let url = format!("{base}/cat/index.json");
    f.storage
        .upsert_setting(SETTING_CATALOG_URL, &url)
        .await
        .unwrap();

    // The tool takes no URL: it is not in the schema, and passing one is an error.
    let (_, listed) = rpc(&f.app, Auth::Bearer(&f.api_key), "tools/list", json!({})).await;
    for name in ["list_template_catalog", "import_catalog_template"] {
        let tool = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap();
        assert!(
            tool["inputSchema"]["properties"].get("url").is_none(),
            "{name}"
        );
    }
    let smuggled = tool_err(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_template_catalog",
        json!({"url": "http://127.0.0.1:1/evil.json"}),
    )
    .await;
    assert!(smuggled.contains("invalid arguments"), "{smuggled}");

    // Reading the configured catalog needs no more than an API key.
    let cat = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_template_catalog",
        json!({}),
    )
    .await;
    assert_eq!(cat["url"], url.as_str());
    assert_eq!(cat["templates"][0]["template_id"], "acme-cat");
    assert_eq!(cat["templates"][0]["installed"], false);

    // Importing is for an admin session.
    let args = json!({"template_id": "acme-cat"});
    for key in [&f.api_key, &f.integration_key] {
        let refused = call(
            &f.app,
            Auth::Bearer(key),
            "import_catalog_template",
            args.clone(),
        )
        .await;
        assert_eq!(refused["error"]["code"], -32003, "{refused}");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(
            message.contains("admin") && message.contains("integration keys"),
            "{message}"
        );
    }
    assert!(f.storage.list_device_templates().await.unwrap().is_empty());

    let imported = ok(
        &f.app,
        Auth::Cookie(&f.admin_cookie()),
        "import_catalog_template",
        args.clone(),
    )
    .await;
    assert_eq!(imported["template_id"], "acme-cat", "{imported}");
    assert_eq!(imported["source"], format!("catalog:{url}"));
    let stored = f.storage.list_device_templates().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].source, format!("catalog:{url}"));

    let cat = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "list_template_catalog",
        json!({}),
    )
    .await;
    assert_eq!(cat["templates"][0]["installed"], true);
    let again = tool_err(
        &f.app,
        Auth::Cookie(&f.admin_cookie()),
        "import_catalog_template",
        args,
    )
    .await;
    assert!(again.contains("already exists"), "{again}");
}

// ── Project ──

const PROJECT_YAML: &str = r#"
format: voltrus-project/1
device_templates:
- template_id: mcp-meter
  version: 1.0.0
  source: local
  manufacturer: Acme
  model: M1
  protocol: modbus
  registers:
  - {name: voltage, offset: 0}
tags:
- {key: mcp_v, name: MCP V, min: 0, max: 10, base: 5}
"#;

#[tokio::test]
async fn mcp_project_export_is_admin_only() {
    let f = fixture().await;
    ok(
        &f.app,
        Auth::Bearer(&f.integration_key),
        "create_tag",
        json!({"key": "exported_tag", "name": "Exported tag"}),
    )
    .await;

    for key in [&f.api_key, &f.integration_key] {
        let refused = call(&f.app, Auth::Bearer(key), "project_export_yaml", json!({})).await;
        assert_eq!(refused["error"]["code"], -32003, "{refused}");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("admin"), "{message}");
    }

    let cookie = f.admin_cookie();
    let out = ok(
        &f.app,
        Auth::Cookie(&cookie),
        "project_export_yaml",
        json!({}),
    )
    .await;
    assert_eq!(out["format"], "voltrus-project/1");
    let yaml = out["yaml"].as_str().unwrap();
    assert!(yaml.starts_with("format: voltrus-project/1\n"), "{yaml}");
    assert!(yaml.contains("exported_tag"));
    assert_eq!(out["bytes"], yaml.len());
    // What the HTTP route exports is what the tool returns, and it parses back.
    let (status, _, body) = {
        let resp = f
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/project/export")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
    };
    assert_eq!(status, StatusCode::OK);
    let strip = |s: &str| {
        s.lines()
            .filter(|l| !l.starts_with("generated_at"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(strip(&body), strip(yaml));
    crate::project_yaml::parse_project_yaml(yaml).unwrap();
}

#[tokio::test]
async fn mcp_project_import_tools_refuse_keys_and_plan_writes_nothing() {
    let f = fixture().await;
    let cookie = f.admin_cookie();
    let args = json!({"yaml": PROJECT_YAML});

    for tool in ["project_plan_import", "project_apply_import"] {
        for key in [&f.api_key, &f.integration_key] {
            let refused = call(&f.app, Auth::Bearer(key), tool, args.clone()).await;
            assert_eq!(refused["error"]["code"], -32003, "{tool}: {refused}");
        }
    }
    assert!(f.storage.list_device_templates().await.unwrap().is_empty());
    assert_eq!(f.storage.list_data_sources(None).await.unwrap().len(), 0);

    // The dry run reports the plan and writes nothing: no rows, no snapshot.
    let snapshots_before = f.storage.list_config_snapshots(1000).await.unwrap().len();
    let plan = ok(
        &f.app,
        Auth::Cookie(&cookie),
        "project_plan_import",
        args.clone(),
    )
    .await;
    assert_eq!(plan["dry_run"], true, "{plan}");
    assert_eq!(plan["mode"], "merge");
    assert_eq!(plan["totals"]["create"], 2);
    assert_eq!(plan["sections"]["device_templates"]["create"], 1);
    assert!(
        plan["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["section"] == "device_templates"
                && c["key"] == "mcp-meter/1.0.0/local"
                && c["action"] == "create")
    );
    assert!(f.storage.list_device_templates().await.unwrap().is_empty());
    assert_eq!(f.storage.list_data_sources(None).await.unwrap().len(), 0);
    assert_eq!(
        f.storage.list_config_snapshots(1000).await.unwrap().len(),
        snapshots_before
    );

    // Mistakes come back as tool errors with the location, and still write nothing.
    let invalid = tool_err(
        &f.app,
        Auth::Cookie(&cookie),
        "project_plan_import",
        json!({"yaml": PROJECT_YAML.replace("protocol: modbus", "protocol: modbus\n  colour: red")}),
    )
    .await;
    assert!(
        invalid.contains("device_templates[0]") && invalid.contains("colour"),
        "{invalid}"
    );
    let bad_mode = tool_err(
        &f.app,
        Auth::Cookie(&cookie),
        "project_plan_import",
        json!({"yaml": PROJECT_YAML, "mode": "wipe"}),
    )
    .await;
    assert!(bad_mode.contains("unknown import mode"), "{bad_mode}");
    let not_yaml = tool_err(
        &f.app,
        Auth::Cookie(&cookie),
        "project_plan_import",
        json!({"yaml": "- [unclosed"}),
    )
    .await;
    assert!(!not_yaml.is_empty());
    assert!(f.storage.list_device_templates().await.unwrap().is_empty());
}

#[tokio::test]
async fn mcp_project_apply_import_writes_and_replace_deletes() {
    let f = fixture().await;
    let cookie = f.admin_cookie();

    let snapshots_before = f.storage.list_config_snapshots(1000).await.unwrap().len();
    let applied = ok(
        &f.app,
        Auth::Cookie(&cookie),
        "project_apply_import",
        json!({"yaml": PROJECT_YAML}),
    )
    .await;
    assert_eq!(applied["dry_run"], false, "{applied}");
    assert_eq!(applied["totals"]["create"], 2);
    let templates = f.storage.list_device_templates().await.unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].template_id, "mcp-meter");
    assert_eq!(templates[0].port, Some(502));
    assert_eq!(f.storage.list_data_sources(None).await.unwrap().len(), 1);
    // Same pre-import snapshot as the HTTP route.
    assert_eq!(
        f.storage.list_config_snapshots(1000).await.unwrap().len(),
        snapshots_before + 1
    );

    // The tool and the template tools see the imported template.
    let t = ok(
        &f.app,
        Auth::Bearer(&f.api_key),
        "get_device_template",
        json!({"template_id": "mcp-meter"}),
    )
    .await;
    assert_eq!(t["registers"][0]["name"], "voltage");

    // Nothing left to do on a second plan, in either mode.
    for mode in ["merge", "replace"] {
        let plan = ok(
            &f.app,
            Auth::Cookie(&cookie),
            "project_plan_import",
            json!({"yaml": PROJECT_YAML, "mode": mode}),
        )
        .await;
        assert_eq!(plan["totals"]["create"], 0, "{mode}: {plan}");
        assert_eq!(plan["totals"]["update"], 0, "{mode}: {plan}");
        assert_eq!(plan["totals"]["delete"], 0, "{mode}: {plan}");
    }

    // Replace with a project that lists neither removes both.
    let replaced = ok(
        &f.app,
        Auth::Cookie(&cookie),
        "project_apply_import",
        json!({"yaml": "format: voltrus-project/1\n", "mode": "replace"}),
    )
    .await;
    assert_eq!(replaced["mode"], "replace");
    assert_eq!(replaced["totals"]["delete"], 2, "{replaced}");
    assert!(f.storage.list_device_templates().await.unwrap().is_empty());
    assert_eq!(f.storage.list_data_sources(None).await.unwrap().len(), 0);
}

#[tokio::test]
async fn mcp_admin_tool_calls_are_audit_logged() {
    let mut state = seeded_admin_app_state().await;
    let (logger, mut rx) = crate::security::AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state);
    let cookie = admin_cookie(&app).await;
    let (int_id, int_key) = make_key(&app, &cookie, "integration").await;
    let cookie = format!("voltrus_token={cookie}");

    let refused = call(
        &app,
        Auth::Bearer(&int_key),
        "project_export_yaml",
        json!({}),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32003);
    ok(
        &app,
        Auth::Cookie(&cookie),
        "project_export_yaml",
        json!({}),
    )
    .await;
    ok(
        &app,
        Auth::Cookie(&cookie),
        "project_plan_import",
        json!({"yaml": PROJECT_YAML}),
    )
    .await;

    let mut events = Vec::new();
    while let Ok(e) = rx.try_recv() {
        events.push(e);
    }
    let denied = events
        .iter()
        .find(|e| e.action == "mcp:project_export_yaml:denied")
        .unwrap_or_else(|| panic!("no denied event in {events:?}"));
    assert_eq!(denied.username, format!("api_key:{int_id}"));
    assert!(
        denied
            .details
            .as_deref()
            .unwrap()
            .contains("role=integration")
    );
    for tool in ["project_export_yaml", "project_plan_import"] {
        let e = events
            .iter()
            .find(|e| e.action == format!("mcp:{tool}"))
            .unwrap_or_else(|| panic!("no {tool} event in {events:?}"));
        assert_eq!(e.username, "admin");
        assert!(e.details.as_deref().unwrap().contains("outcome=ok"));
    }
    // A dry run is not an import.
    assert!(!events.iter().any(|e| e.action == "project_import"));
}
