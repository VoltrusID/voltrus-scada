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
];

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
    api_key: String,
    integration_key: String,
    cookie: String,
}

async fn fixture() -> Fixture {
    let app = test_app_with_seeded_admin().await;
    let cookie = admin_cookie(&app).await;
    let (_, api_key) = make_key(&app, &cookie, "api").await;
    let (_, integration_key) = make_key(&app, &cookie, "integration").await;
    Fixture {
        app,
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
    for expected in READ_TOOLS.iter().chain(WRITE_TOOLS) {
        assert!(
            names.contains(expected),
            "missing tool {expected}: {names:?}"
        );
    }
    assert_eq!(
        names.len(),
        READ_TOOLS.len() + WRITE_TOOLS.len(),
        "{names:?}"
    );
    // Admin-only operations are not exposed.
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
        let write = WRITE_TOOLS.contains(&name);
        assert_eq!(tool["annotations"]["readOnlyHint"], !write, "{name}");
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
