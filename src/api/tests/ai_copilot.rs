//! AI copilot proposals: generation against a mock provider, and the apply
//! path (exact tag matching, register maps that are polled, all-or-nothing).

use super::*;
use axum::Json;
use http::StatusCode;
use serde_json::{Value, json};
use std::sync::Mutex;

struct Copilot {
    app: Router,
    state: AppState,
    cookie: String,
    audit: Mutex<tokio::sync::mpsc::Receiver<crate::security::AuditEvent>>,
}

async fn copilot() -> Copilot {
    let mut state = seeded_admin_app_state().await;
    let (logger, rx) = crate::security::AuditLogger::new();
    state.audit_logger = logger;
    let app = create_router(state.clone());
    let cookie = format!("voltrus_token={}", admin_cookie(&app).await);
    Copilot {
        app,
        state,
        cookie,
        audit: Mutex::new(rx),
    }
}

impl Copilot {
    /// Run SQL on the database (triggers that make a write fail).
    async fn sql(&self, batch: &'static str) {
        self.state
            .storage
            .with_conn(move |c| Ok(c.execute_batch(batch)?))
            .await
            .unwrap();
    }

    /// The details of the audit entries with `action` logged since the last call.
    fn audited(&self, action: &str) -> Vec<Value> {
        let mut rx = self.audit.lock().unwrap();
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter(|e| e.action == action)
            .map(|e| serde_json::from_str(&e.details.unwrap_or_default()).unwrap_or(Value::Null))
            .collect()
    }

    async fn proposal_body(&self, id: i64) -> Value {
        let (status, body) = self
            .send("GET", &format!("/api/v1/ai/proposals/{id}"), None)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn modbus_device(&self, id: &str, registers: Value) {
        let (status, body) = self
            .send(
                "POST",
                "/api/v1/modbus-devices",
                Some(json!({"id": id, "ip": "127.0.0.1", "port": 1, "registers": registers})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }

    async fn send(&self, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        send_request(&self.app, method, uri, None, Some(&self.cookie), body).await
    }

    /// A pending proposal, as a generator would store it.
    async fn proposal(&self, kind: &str, config: Value, target: Option<&str>) -> i64 {
        self.state
            .storage
            .create_ai_proposal(kind, "test", "{}", &config.to_string(), target, "m", 0)
            .await
            .unwrap()
    }

    async fn approve_and_apply(&self, id: i64) -> (StatusCode, Value) {
        let (status, body) = self
            .send(
                "POST",
                &format!("/api/v1/ai/proposals/{id}/approve"),
                Some(json!({})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        self.send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
            .await
    }

    async fn tag(&self, key: &str, device: &str) -> i64 {
        let (status, body) = self
            .send(
                "POST",
                "/api/v1/data-sources",
                Some(json!({"key": key, "name": key.replace(':', "_"), "device_id": device, "warn_val": 1, "crit_val": 2})),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        body["id"].as_i64().unwrap()
    }

    async fn tags(&self) -> Vec<Value> {
        let (_, body) = self.send("GET", "/api/v1/data-sources", None).await;
        body.as_array().unwrap().clone()
    }

    async fn status_of(&self, id: i64) -> String {
        let (_, body) = self
            .send("GET", &format!("/api/v1/ai/proposals/{id}"), None)
            .await;
        body["status"].as_str().unwrap().to_string()
    }

    async fn use_llm(&self, base: &str) {
        let (status, body) = self
            .send(
                "PUT",
                "/api/v1/ai/settings",
                Some(json!({"provider": "byok_openai_compatible", "base_url": base, "default_model": "m", "api_key": "sk-test"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
}

fn tag_by_id(tags: &[Value], id: i64) -> &Value {
    tags.iter().find(|t| t["id"] == id).unwrap()
}

type Seen = Arc<Mutex<Vec<Value>>>;

/// An OpenAI-style server answering every call with `status`, `body` and an
/// optional Retry-After; returns its base URL and the request bodies.
async fn mock_llm(status: u16, retry_after: Option<&'static str>, body: Value) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let recorder = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |Json(req): Json<Value>| {
            let recorder = recorder.clone();
            let body = body.clone();
            async move {
                recorder.lock().unwrap().push(req);
                let mut resp = axum::response::IntoResponse::into_response((
                    StatusCode::from_u16(status).unwrap(),
                    Json(body),
                ));
                if let Some(secs) = retry_after {
                    resp.headers_mut()
                        .insert("retry-after", secs.parse().unwrap());
                }
                resp
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}/v1"), seen)
}

fn reply(content: &str, finish_reason: &str) -> Value {
    json!({
        "model": "m",
        "choices": [{"message": {"content": content}, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })
}

// ── Bug 1: an applied register map is polled ──

#[cfg(feature = "modbus")]
#[tokio::test]
async fn applied_register_map_adds_polled_registers_and_modbus_tags() {
    let c = copilot().await;
    let (status, body) = c
        .send(
            "POST",
            "/api/v1/modbus-devices",
            Some(
                json!({"id": "meter1", "name": "Meter 1", "ip": "127.0.0.1", "port": 1,
                "registers": [{"name": "existing", "address": 40100, "register_type": "uint16"}]}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let id = c
        .proposal(
            "register_map",
            json!([
                {"name": "voltage_l1", "address": 40001, "register_type": "float32", "register_area": "holding", "scale": 0.1, "unit": "V"},
                {"name": "current_l1", "address": 30005, "register_type": "int16", "unit": "A"},
                {"name": "breaker_closed", "offset": 7, "register_area": "coil"}
            ]),
            Some("meter1"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["registers_added"], 3, "{body}");
    assert_eq!(c.status_of(id).await, "applied");

    // The poller's table has them, so they are read.
    let (_, dev) = c.send("GET", "/api/v1/modbus-devices/meter1", None).await;
    let regs = dev["registers"].as_array().unwrap();
    let reg = |name: &str| regs.iter().find(|r| r["name"] == name).unwrap().clone();
    assert_eq!(regs.len(), 4, "{dev}");
    assert_eq!(reg("existing")["address"], 40100);
    assert_eq!(reg("voltage_l1")["address"], 40001);
    assert_eq!(reg("voltage_l1")["register_type"], "float32");
    assert_eq!(reg("voltage_l1")["scale"], 0.1);
    assert_eq!(reg("current_l1")["register_area"], "input");
    assert_eq!(reg("breaker_closed")["register_area"], "coil");
    assert_eq!(reg("breaker_closed")["address"], 7);
    assert_eq!(reg("breaker_closed")["zero_based"], true);

    let configs = c.state.storage.get_all_device_configs().await.unwrap();
    let cfg = configs.iter().find(|d| d.id == "meter1").unwrap();
    assert_eq!(cfg.registers.len(), 4);

    // One modbus tag per new register, keyed like the poller's metric.
    let tags = c.tags().await;
    for name in ["voltage_l1", "current_l1", "breaker_closed"] {
        let t = tags.iter().find(|t| t["key"] == name).unwrap();
        assert_eq!(t["source_type"], "modbus");
        assert_eq!(t["device_id"], "meter1");
    }
    assert_eq!(
        tags.iter().find(|t| t["key"] == "voltage_l1").unwrap()["unit"],
        "V"
    );
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn register_map_apply_is_all_or_nothing() {
    let c = copilot().await;
    c.send(
        "POST",
        "/api/v1/modbus-devices",
        Some(json!({"id": "m1", "ip": "127.0.0.1", "port": 1,
            "registers": [{"name": "taken", "address": 40001, "register_type": "uint16"}]})),
    )
    .await;

    for (config, want) in [
        // A register name the device already has.
        (
            json!([{"name": "fresh", "address": 40002, "register_type": "int16"},
                   {"name": "taken", "address": 40003, "register_type": "int16"}]),
            "already has a register named 'taken'",
        ),
        // A tag name the naming convention refuses, after a valid one.
        (
            json!([{"name": "fresh", "address": 40002, "register_type": "int16"},
                   {"name": "bad/name", "address": 40004, "register_type": "int16"}]),
            "bad/name",
        ),
    ] {
        let id = c.proposal("register_map", config, Some("m1")).await;
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains(want), "{body}");
        assert_eq!(c.status_of(id).await, "approved");
    }
    let (_, dev) = c.send("GET", "/api/v1/modbus-devices/m1", None).await;
    assert_eq!(dev["registers"].as_array().unwrap().len(), 1, "{dev}");
    assert!(c.tags().await.is_empty());

    // No device, or a device that does not exist: refused, nothing written.
    for target in [None, Some("ghost")] {
        let id = c
            .proposal(
                "register_map",
                json!([{"name": "v", "address": 40001, "register_type": "int16"}]),
                target,
            )
            .await;
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
    assert!(c.tags().await.is_empty());
}

// ── Bugs 3 and 6: addresses out of range and unsupported types ──

#[cfg(feature = "modbus")]
#[tokio::test]
async fn register_map_with_a_6_digit_address_or_unknown_type_is_refused() {
    let c = copilot().await;
    c.send(
        "POST",
        "/api/v1/modbus-devices",
        Some(json!({"id": "m1", "ip": "127.0.0.1", "port": 1})),
    )
    .await;
    for (reg, want) in [
        (
            json!({"name": "v", "address": 400001, "register_type": "int16"}),
            "6-digit references are not supported",
        ),
        (
            json!({"name": "v", "address": 40001, "register_type": "uint32"}),
            "register_type 'uint32' is not supported",
        ),
        (
            json!({"name": "v", "address": 40001, "register_type": "bool"}),
            "register_type 'bool' is not supported",
        ),
    ] {
        let id = c.proposal("register_map", json!([reg]), Some("m1")).await;
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains(want), "{body}");
    }
    let (_, dev) = c.send("GET", "/api/v1/modbus-devices/m1", None).await;
    assert!(dev["registers"].as_array().unwrap().is_empty(), "{dev}");
}

// ── Bug 2: thresholds land on exactly the tag they name ──

#[tokio::test]
async fn alarm_thresholds_match_the_tag_exactly() {
    let c = copilot().await;
    let temp = c.tag("pump1:temp", "pump1").await;
    let bearing = c.tag("pump1:temp_bearing", "pump1").await;

    // Substring matching used to put this on pump1:temp, the first tag whose
    // key the proposed key contains.
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "pump1:temp_bearing", "warn_val": 70, "crit_val": 85}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, bearing)["warn_val"], 70.0);
    assert_eq!(tag_by_id(&tags, bearing)["crit_val"], 85.0);
    assert_eq!(tag_by_id(&tags, temp)["warn_val"], 1.0);
    assert_eq!(tag_by_id(&tags, temp)["crit_val"], 2.0);

    // A key that only partly matches is refused, as is a tag_id whose key differs.
    for t in [
        json!({"source_key": "pump1:te", "warn_val": 5, "crit_val": 6}),
        json!({"source_key": "pump1", "warn_val": 5, "crit_val": 6}),
        json!({"source_key": "pump1:temp", "tag_id": bearing, "warn_val": 5, "crit_val": 6}),
    ] {
        let id = c.proposal("alarm_thresholds", json!([t]), None).await;
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{t}: {body}");
    }

    // Two devices, one key: the key alone is ambiguous, the tag id is not.
    let a = c.tag("temp", "dev_a").await;
    let b = c.tag("temp", "dev_b").await;
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "temp", "warn_val": 50, "crit_val": 60}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("Several tags"),
        "{body}"
    );
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "temp", "tag_id": b, "warn_val": 50, "crit_val": 60}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, b)["warn_val"], 50.0);
    assert_eq!(tag_by_id(&tags, a)["warn_val"], 1.0);
}

// ── Bug 5: apply validates every item before writing any ──

#[tokio::test]
async fn alarm_and_rename_applies_are_all_or_nothing() {
    let c = copilot().await;
    let one = c.tag("t1", "d").await;
    let two = c.tag("t2", "d").await;

    // The second threshold has its warning above its critical limit (after
    // merging with the tag's current critical limit of 2).
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "t1", "warn_val": 10, "crit_val": 20},
                   {"source_key": "t2", "warn_val": 5}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, one)["warn_val"], 1.0);

    // The third new name breaks the naming convention: none is renamed.
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "t1", "proposed": "TT_001"},
                   {"current": "t2", "proposed": "TT_002"},
                   {"current": "t2", "proposed": "TT_003"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "t1", "proposed": "TT_001"},
                   {"current": "t2", "proposed": "bad/name"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("bad/name"),
        "{body}"
    );
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, one)["name"], "t1");
    assert_eq!(tag_by_id(&tags, two)["name"], "t2");

    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "t1", "proposed": "TT_001"},
                   {"current": "t2", "proposed": "TT_002"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, one)["name"], "TT_001");
    assert_eq!(tag_by_id(&tags, two)["name"], "TT_002");
}

#[tokio::test]
async fn flow_rule_apply_uses_the_flow_route_checks() {
    let c = copilot().await;
    // A node type the flow graph does not know is refused by the flows route.
    let id = c
        .proposal(
            "flow_rule",
            json!({"name": "x", "trigger_config": {"type": "timer", "interval_ms": 60000},
                   "nodes": [{"id": "n1", "type": "launch_missiles"}]}),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (_, flows) = c.send("GET", "/api/v1/flows", None).await;
    assert!(flows["flows"].as_array().unwrap().is_empty(), "{flows}");
    assert_eq!(c.status_of(id).await, "approved");
}

// ── Bug 4: an invalid edit is an error, not a silent approval ──

#[tokio::test]
async fn approving_with_an_invalid_edit_is_refused() {
    let c = copilot().await;
    let id = c
        .proposal(
            "register_map",
            json!([{"name": "v", "address": 40001, "register_type": "int16"}]),
            Some("m1"),
        )
        .await;
    for edited in [
        json!("[{\"name\": \"v\", \"address\": 40001"),
        json!({"name": "v"}),
        json!([{"name": "v", "address": 400001, "register_type": "int16"}]),
    ] {
        let (status, body) = c
            .send(
                "POST",
                &format!("/api/v1/ai/proposals/{id}/approve"),
                Some(json!({"applied_config": edited})),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{edited}: {body}");
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("edited configuration is not valid"),
            "{body}"
        );
        assert_eq!(c.status_of(id).await, "pending");
    }

    let (status, _) = c
        .send(
            "POST",
            &format!("/api/v1/ai/proposals/{id}/approve"),
            Some(json!({"applied_config": [{"name": "v2", "address": 40002, "register_type": "int16"}]})),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    // An approved (or applied) proposal cannot be approved again.
    let (status, _) = c
        .send(
            "POST",
            &format!("/api/v1/ai/proposals/{id}/approve"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ── Bugs 7 and 8: truncated replies and provider errors ──

#[tokio::test]
async fn reply_cut_at_the_token_limit_fails_clearly() {
    let c = copilot().await;
    let (base, _) = mock_llm(
        200,
        None,
        reply("[{\"name\": \"voltage\", \"address\": 40001", "length"),
    )
    .await;
    c.use_llm(&base).await;
    let (status, body) = c
        .send(
            "POST",
            "/api/v1/ai/generate/register-map",
            Some(json!({"device_name": "m", "file_content": "voltage 40001 float32", "file_type": "csv"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{body}");
    assert_eq!(body["code"], "ai_truncated", "{body}");
    assert!(body["error"].as_str().unwrap().contains("4096"), "{body}");
    let (_, list) = c.send("GET", "/api/v1/ai/proposals", None).await;
    assert!(list.as_array().unwrap().is_empty(), "{list}");
}

#[tokio::test]
async fn provider_401_and_429_are_clear_errors() {
    let c = copilot().await;
    let request =
        json!({"device_name": "m", "file_content": "voltage 40001 float32", "file_type": "csv"});

    let (base, _) = mock_llm(
        401,
        None,
        json!({"error": {"message": "Incorrect API key provided"}}),
    )
    .await;
    c.use_llm(&base).await;
    let (status, body) = c
        .send(
            "POST",
            "/api/v1/ai/generate/register-map",
            Some(request.clone()),
        )
        .await;
    // Not 401: the web client takes that for its own session expiring.
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "ai_invalid_api_key", "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("API key"),
        "{body}"
    );

    let (base, _) = mock_llm(
        429,
        Some("30"),
        json!({"error": {"message": "Rate limit reached"}}),
    )
    .await;
    c.use_llm(&base).await;
    let (status, body) = c
        .send("POST", "/api/v1/ai/generate/register-map", Some(request))
        .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert_eq!(body["code"], "ai_rate_limited", "{body}");
    assert_eq!(body["retry_after_secs"], 30, "{body}");
}

// ── Bug 9: PDFs ──

#[tokio::test]
async fn pdf_datasheet_is_refused_before_calling_the_provider() {
    let c = copilot().await;
    let (base, seen) = mock_llm(200, None, reply("[]", "stop")).await;
    c.use_llm(&base).await;
    let (status, body) = c
        .send(
            "POST",
            "/api/v1/ai/generate/register-map",
            Some(json!({"device_name": "m", "file_content": "%PDF-1.7\n%\u{fffd}\u{fffd}\n1 0 obj", "file_type": "pdf"})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains(".csv or .txt"),
        "{body}"
    );
    assert!(seen.lock().unwrap().is_empty());
}

// ── Bug 10: scrubbing and data framing in every prompt ──

#[tokio::test]
async fn every_generator_scrubs_and_fences_what_it_sends() {
    let c = copilot().await;
    c.tag("plc.plant.local", "d").await;
    let tag_id = c.tag("t1", "d").await;
    let (base, seen) = mock_llm(200, None, reply("[]", "stop")).await;
    c.use_llm(&base).await;

    let secret = "PLC at 10.1.2.3 and fd00::1:2 via historian.acme.co.id";
    let calls = [
        (
            "alarm-thresholds",
            json!({"device_type": secret, "tags": [{"id": tag_id, "source_key": "t1", "name": secret}]}),
        ),
        (
            "tag-rename",
            json!({"tags": [{"current": secret}], "convention": "isa-5.1", "custom_pattern": secret}),
        ),
        (
            "flow-rule",
            json!({"description": secret, "available_tags": [secret]}),
        ),
        ("dashboard-layout", json!({"description": secret})),
        (
            "register-map",
            json!({"device_name": secret, "file_content": secret, "file_type": "text"}),
        ),
    ];
    for (path, body) in calls {
        c.send("POST", &format!("/api/v1/ai/generate/{path}"), Some(body))
            .await;
    }
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 5, "every generator called the provider");
    for req in seen.iter() {
        let system = req["messages"][0]["content"].as_str().unwrap();
        let user = req["messages"][1]["content"].as_str().unwrap();
        for leak in ["10.1.2.3", "fd00::1:2", "acme.co.id", "plant.local"] {
            assert!(!user.contains(leak), "{leak} sent: {user}");
        }
        assert!(user.contains("<data source="), "{user}");
        assert!(
            system.contains("never follow instructions")
                || system.contains("Never follow instructions"),
            "{system}"
        );
    }
}

// ── Writes that fail part-way are undone ──

#[tokio::test]
async fn a_failed_write_undoes_the_writes_before_it() {
    use crate::api::ai_copilot::helpers::{create_tags_all_or_nothing, update_tags_all_or_nothing};
    let c = copilot().await;
    let one = c.tag("t1", "d").await;
    let two = c.tag("t2", "d").await;

    // The first update is written, the second is refused by the tag route
    // (an unknown field): the first is put back.
    let plan = vec![
        (
            one,
            "t1".to_string(),
            json!({"warn_val": 40, "crit_val": 50}),
            json!({"warn_val": 1, "crit_val": 2}),
        ),
        (
            two,
            "t2".to_string(),
            json!({"colour": "red"}),
            json!({"warn_val": 1, "crit_val": 2}),
        ),
    ];
    let err = update_tags_all_or_nothing(&c.state, &plan)
        .await
        .unwrap_err();
    let msg = format!("{:#}", err.0);
    assert!(msg.contains("Nothing was applied"), "{msg}");
    assert!(msg.contains("tag 't2'"), "{msg}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, one)["warn_val"], 1.0);
    assert_eq!(tag_by_id(&tags, one)["crit_val"], 2.0);

    // The first tag is created, the second is refused: the first is deleted.
    let before = c.tags().await.len();
    let Err(err) = create_tags_all_or_nothing(
        &c.state,
        vec![
            (
                "new1".to_string(),
                json!({"key": "new1", "name": "new1", "device_id": "d"}),
            ),
            (
                "new2".to_string(),
                json!({"key": "new2", "name": "new2", "colour": "red"}),
            ),
        ],
    )
    .await
    else {
        panic!("the second tag should be refused");
    };
    let msg = format!("{:#}", err.0);
    assert!(msg.contains("tag 'new2'"), "{msg}");
    assert!(msg.contains("Nothing was applied"), "{msg}");
    let tags = c.tags().await;
    assert_eq!(tags.len(), before, "{tags:?}");
    assert!(!tags.iter().any(|t| t["key"] == "new1"));
}

// ── One proposal is applied once ──

#[tokio::test]
async fn concurrent_applies_of_one_proposal_apply_it_once() {
    let c = copilot().await;
    let id = c
        .proposal(
            "flow_rule",
            json!({"name": "hot", "trigger_config": {"type": "modbus", "device_id": "*"},
                   "nodes": [{"id": "n1", "type": "filter", "condition": "metrics.temp > 0"}]}),
            None,
        )
        .await;
    let (status, body) = c
        .send(
            "POST",
            &format!("/api/v1/ai/proposals/{id}/approve"),
            Some(json!({})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let uri = format!("/api/v1/ai/proposals/{id}/apply");
    let ((s1, b1), (s2, b2)) = tokio::join!(c.send("POST", &uri, None), c.send("POST", &uri, None));
    let mut statuses = [s1, s2];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::BAD_REQUEST],
        "{b1} / {b2}"
    );
    let (_, flows) = c.send("GET", "/api/v1/flows", None).await;
    let flows = flows["flows"].as_array().unwrap();
    assert_eq!(flows.len(), 1, "{flows:?}");
    assert_eq!(flows[0]["enabled"], false, "AI flows start disabled");
    assert_eq!(c.status_of(id).await, "applied");

    // Applied is final: not rejected afterwards.
    let (status, _) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/reject"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(c.status_of(id).await, "applied");

    // A pending proposal cannot be applied (a 400, not a server error).
    let pending = c
        .proposal(
            "tag_rename",
            json!([{"current": "x", "proposed": "y"}]),
            None,
        )
        .await;
    let (status, body) = c
        .send(
            "POST",
            &format!("/api/v1/ai/proposals/{pending}/apply"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, _) = c
        .send("POST", "/api/v1/ai/proposals/999999/apply", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── Dashboard layouts and equipment templates ──

#[tokio::test]
async fn dashboard_layout_apply_needs_a_screen_and_whole_grid_items() {
    let c = copilot().await;
    let (status, body) = c
        .send("POST", "/api/v1/screens", Some(json!({"name": "line1"})))
        .await;
    assert!(status.is_success(), "{body}");

    let item = json!({"i": "t1", "x": 0, "y": 0, "w": 4, "h": 2});
    for (config, target, want) in [
        (
            json!({"widgets": [], "layout": [item]}),
            Some("ghost"),
            "does not exist",
        ),
        (
            json!({"widgets": [], "layout": [{"i": "t1", "x": 0, "y": 0, "w": 0, "h": 2}]}),
            Some("line1"),
            "'w' must be a whole number",
        ),
        (json!({"widgets": []}), Some("line1"), "missing 'layout'"),
    ] {
        let id = c.proposal("dashboard_layout", config, target).await;
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains(want), "{body}");
    }
    let (_, saved) = c.send("GET", "/api/v1/screens/line1/layouts", None).await;
    assert!(!saved.to_string().contains("\"t1\""), "{saved}");

    let id = c
        .proposal(
            "dashboard_layout",
            json!({"widgets": [], "layout": [item]}),
            Some("line1"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, saved) = c.send("GET", "/api/v1/screens/line1/layouts", None).await;
    assert!(saved.to_string().contains("\"t1\""), "{saved}");
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn equipment_template_apply_creates_all_its_tags_or_none() {
    let c = copilot().await;
    c.modbus_device("pump1", json!([])).await;
    c.modbus_device("pump2", json!([])).await;
    c.tag("PT_1", "pump1").await;
    let before = c.tags().await.len();

    // One of the template's tags exists on the device already: nothing is created.
    let id = c
        .proposal(
            "equipment_template",
            json!({"prefix": "pump", "tags": [
                {"name_pattern": "FT_{n}", "unit": "m3/h"},
                {"name_pattern": "PT_{n}", "unit": "bar"}]}),
            Some("pump1"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("already has a tag 'PT_1'"),
        "{body}"
    );
    assert_eq!(c.tags().await.len(), before);

    let id = c
        .proposal(
            "equipment_template",
            json!({"prefix": "pump", "tags": [
                {"name_pattern": "FT_{n}", "unit": "m3/h", "register_area": "input"},
                {"name_pattern": "TT_{n}", "unit": "C"}]}),
            Some("pump2"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["tags_created"], 2, "{body}");
    let tags = c.tags().await;
    // Keyed by the tag name, as the poller keys a register's value.
    let ft = tags
        .iter()
        .find(|t| t["key"] == "FT_1" && t["device_id"] == "pump2")
        .unwrap();
    assert_eq!(ft["source_type"], "modbus");
    assert_eq!(ft["unit"], "m3/h");
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn equipment_template_needs_a_known_device_and_valid_tags() {
    let c = copilot().await;
    c.modbus_device("pump1", json!([])).await;
    let ok_tags = json!({"tags": [{"name_pattern": "FT_{n}"}]});

    for (config, target, want) in [
        // No device, or one that does not exist: no falling back to the prefix.
        (ok_tags.clone(), None, "no target device"),
        (ok_tags.clone(), Some("equip"), "does not exist"),
        (json!({"prefix": "pump"}), Some("pump1"), "no 'tags' array"),
        (json!({"tags": []}), Some("pump1"), "no tags"),
        (
            json!({"tags": [{"unit": "bar"}]}),
            Some("pump1"),
            "'name_pattern' is missing",
        ),
        (
            json!({"tags": [{"name_pattern": "FT_1", "register_area": "eeprom"}]}),
            Some("pump1"),
            "register_area 'eeprom'",
        ),
        (
            json!({"tags": [{"name_pattern": "FT_1", "default_type": "string"}]}),
            Some("pump1"),
            "default_type 'string'",
        ),
        (
            json!({"tags": [{"name_pattern": "FT_{n}"}, {"name_pattern": "FT_1"}]}),
            Some("pump1"),
            "twice",
        ),
    ] {
        let id = c
            .proposal("equipment_template", config.clone(), target)
            .await;
        // An invalid configuration is refused at approval when edited, and at apply.
        let (status, body) = c.approve_and_apply(id).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{config}: {body}");
        assert!(
            body["error"].as_str().unwrap().contains(want),
            "{config}: {body}"
        );
        assert_eq!(c.status_of(id).await, "approved");
    }
    assert!(c.tags().await.is_empty());

    let pending = c
        .proposal("equipment_template", ok_tags.clone(), Some("pump1"))
        .await;
    let (status, body) = c
        .send(
            "POST",
            &format!("/api/v1/ai/proposals/{pending}/approve"),
            Some(json!({"applied_config": {"prefix": "x"}})),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

// ── Applies that fail part-way, or whose outcome cannot be saved ──

#[tokio::test]
async fn a_proposal_whose_applied_status_cannot_be_saved_is_not_applied_twice() {
    let c = copilot().await;
    let id = c
        .proposal(
            "flow_rule",
            json!({"name": "hot", "trigger_config": {"type": "modbus", "device_id": "*"},
                   "nodes": [{"id": "n1", "type": "filter", "condition": "metrics.temp > 0"}]}),
            None,
        )
        .await;
    c.sql(
        "CREATE TRIGGER no_applied BEFORE UPDATE OF status ON ai_proposals
         WHEN NEW.status = 'applied' BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;

    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let msg = body["error"].as_str().unwrap();
    assert!(
        msg.contains("was applied, but its status could not be saved"),
        "{msg}"
    );
    let p = c.proposal_body(id).await;
    assert_eq!(p["status"], "failed", "{p}");
    assert!(
        p["error_message"].as_str().unwrap().contains("was applied"),
        "{p}"
    );
    let audited = c.audited("ai_proposal_applied");
    assert_eq!(audited.len(), 1, "{audited:?}");
    assert_eq!(audited[0]["status_saved"], false);

    // Applying again is refused: no second flow.
    let (status, body) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (_, flows) = c.send("GET", "/api/v1/flows", None).await;
    assert_eq!(flows["flows"].as_array().unwrap().len(), 1, "{flows}");
}

#[tokio::test]
async fn an_apply_cut_off_by_a_crash_is_flagged_and_never_reapplied() {
    let c = copilot().await;
    c.tag("t1", "d").await;
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "t1", "warn_val": 10, "crit_val": 20}]),
            None,
        )
        .await;
    // What a crash between the first write and the end of the apply leaves.
    c.state
        .storage
        .update_ai_proposal_status(id, "approved", Some("admin"), None)
        .await
        .unwrap();
    assert!(c.state.storage.begin_ai_proposal_apply(id).await.unwrap());
    assert!(!c.state.storage.begin_ai_proposal_apply(id).await.unwrap());

    let (status, body) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, _) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/reject"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The next start marks it failed, with a message for the reviewer.
    assert_eq!(
        c.state.storage.flag_interrupted_ai_applies().await.unwrap(),
        1
    );
    let p = c.proposal_body(id).await;
    assert_eq!(p["status"], "failed", "{p}");
    assert!(
        p["error_message"].as_str().unwrap().contains("interrupted"),
        "{p}"
    );
    let (status, _) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        c.state.storage.flag_interrupted_ai_applies().await.unwrap(),
        0
    );
}

#[cfg(feature = "modbus")]
#[tokio::test]
async fn register_map_failures_after_a_write_leave_the_device_as_it_was() {
    let c = copilot().await;
    c.modbus_device(
        "m1",
        json!([{"name": "old", "address": 40001, "register_type": "uint16"}]),
    )
    .await;
    let map = json!([{"name": "v1", "address": 40002, "register_type": "int16"},
                     {"name": "v2", "address": 40003, "register_type": "int16"}]);
    let registers = || async {
        let (_, dev) = c.send("GET", "/api/v1/modbus-devices/m1", None).await;
        dev["registers"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    // The second tag cannot be created: the first is deleted and the
    // registers are never written.
    c.sql(
        "CREATE TRIGGER no_v2 BEFORE INSERT ON data_sources
         WHEN NEW.key = 'v2' BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;
    let id = c.proposal("register_map", map.clone(), Some("m1")).await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Nothing was applied"),
        "{body}"
    );
    assert_eq!(c.status_of(id).await, "approved");
    assert_eq!(registers().await, ["old"]);
    assert!(c.tags().await.is_empty());
    let failed = c.audited("ai_proposal_apply_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["undo"], "nothing to undo or fully undone");
    c.sql("DROP TRIGGER no_v2;").await;

    // The tags are created, then the register write fails: the tags go.
    c.sql(
        "CREATE TRIGGER no_registers BEFORE INSERT ON modbus_registers
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;
    let (status, body) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Nothing was applied"),
        "{body}"
    );
    assert_eq!(c.status_of(id).await, "approved");
    assert_eq!(registers().await, ["old"]);
    assert!(c.tags().await.is_empty());

    // ... and when deleting them fails too, the proposal needs attention.
    c.sql(
        "CREATE TRIGGER keep_tags BEFORE DELETE ON data_sources
         BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;
    let (status, body) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Undoing the changes"),
        "{body}"
    );
    let p = c.proposal_body(id).await;
    assert_eq!(p["status"], "failed", "{p}");
    assert!(
        p["error_message"].as_str().unwrap().contains("Undoing"),
        "{p}"
    );
    let failed = c.audited("ai_proposal_apply_failed");
    assert_eq!(failed.last().unwrap()["undo"], "failed", "{failed:?}");

    // The success path restarts nothing twice and records the old registers.
    c.sql("DROP TRIGGER no_registers; DROP TRIGGER keep_tags;")
        .await;
    let id = c
        .proposal(
            "register_map",
            json!([{"name": "v3", "address": 40004, "register_type": "int16"}]),
            Some("m1"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["previous_registers"][0]["name"], "old");
}

#[tokio::test]
async fn a_failed_undo_marks_the_proposal_failed_with_a_server_error() {
    let c = copilot().await;
    let one = c.tag("t1", "d").await;
    c.tag("t2", "d").await;
    // The second write fails, and so does putting the first one back.
    c.sql(
        "CREATE TRIGGER fail_t2 BEFORE UPDATE ON data_sources
         WHEN NEW.warn_val = 30 BEGIN SELECT RAISE(ABORT, 'disk full'); END;
         CREATE TRIGGER fail_undo BEFORE UPDATE ON data_sources
         WHEN NEW.warn_val = 1 BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;
    let id = c
        .proposal(
            "alarm_thresholds",
            json!([{"source_key": "t1", "warn_val": 10, "crit_val": 20},
                   {"source_key": "t2", "warn_val": 30, "crit_val": 40}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    let msg = body["error"].as_str().unwrap();
    assert!(msg.contains("Undoing the changes"), "{msg}");
    assert!(msg.contains("tag 't1'"), "{msg}");
    let p = c.proposal_body(id).await;
    assert_eq!(p["status"], "failed", "{p}");
    assert!(
        p["error_message"].as_str().unwrap().contains("tag 't1'"),
        "{p}"
    );
    let failed = c.audited("ai_proposal_apply_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert_eq!(failed[0]["undo"], "failed");
    assert!(
        failed[0]["undo_failures"][0]
            .as_str()
            .unwrap()
            .contains("t1")
    );
    // Left as the failed undo left it, and not applied again.
    assert_eq!(tag_by_id(&c.tags().await, one)["warn_val"], 10.0);
    let (status, _) = c
        .send("POST", &format!("/api/v1/ai/proposals/{id}/apply"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_rename_is_undone_even_when_the_old_names_break_the_convention() {
    let c = copilot().await;
    let one = c.tag("t1", "d").await;
    let two = c.tag("t2", "d").await;
    // KKS: the new names pass, the old ones (t1, t2) do not.
    let (status, body) = c
        .send(
            "PUT",
            "/api/v1/settings/naming-convention",
            Some(json!({"template": "kks"})),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    c.sql(
        "CREATE TRIGGER fail_second BEFORE UPDATE OF name ON data_sources
         WHEN NEW.name = 'AB10CT002' BEGIN SELECT RAISE(ABORT, 'disk full'); END;",
    )
    .await;
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "t1", "proposed": "AB10CT001"},
                   {"current": "t2", "proposed": "AB10CT002"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Nothing was applied"),
        "{body}"
    );
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, one)["name"], "t1");
    assert_eq!(tag_by_id(&tags, two)["name"], "t2");
    assert_eq!(c.status_of(id).await, "approved");
}

#[tokio::test]
async fn a_rename_can_name_its_tag_by_id_and_the_audit_keeps_old_values() {
    let c = copilot().await;
    let a = c.tag("temp", "dev_a").await;
    let b = c.tag("temp", "dev_b").await;

    // Two tags called "temp": by name alone it is ambiguous.
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "temp", "proposed": "TT_B"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("Several tags"),
        "{body}"
    );

    // A tag id whose tag is not `current` is refused.
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "other", "tag_id": b, "proposed": "TT_B"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    c.audited("ai_proposal_applied");
    let id = c
        .proposal(
            "tag_rename",
            json!([{"current": "temp", "tag_id": b, "proposed": "TT_B"}]),
            None,
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let tags = c.tags().await;
    assert_eq!(tag_by_id(&tags, b)["name"], "TT_B");
    assert_eq!(tag_by_id(&tags, a)["name"], "temp");
    let audited = c.audited("ai_proposal_applied");
    let change = &audited[0]["result"]["changes"][0];
    assert_eq!(change["tag_id"], b, "{audited:?}");
    assert_eq!(change["before"]["name"], "temp");
    assert_eq!(change["after"]["name"], "TT_B");
}

#[tokio::test]
async fn a_dashboard_apply_records_the_layouts_it_replaced() {
    let c = copilot().await;
    let (status, body) = c
        .send("POST", "/api/v1/screens", Some(json!({"name": "line1"})))
        .await;
    assert!(status.is_success(), "{body}");
    let old = json!([{"i": "old", "x": 0, "y": 0, "w": 2, "h": 2}]);
    let (status, body) = c
        .send(
            "PUT",
            "/api/v1/screens/line1/layouts",
            Some(json!({"lg": old, "md": old, "sm": old})),
        )
        .await;
    assert!(status.is_success(), "{body}");
    let id = c
        .proposal(
            "dashboard_layout",
            json!({"widgets": [], "layout": [{"i": "new", "x": 0, "y": 0, "w": 4, "h": 2}]}),
            Some("line1"),
        )
        .await;
    let (status, body) = c.approve_and_apply(id).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["previous_layouts"]["lg"], old, "{body}");
    assert_eq!(body["result"]["previous_layouts"]["sm"], old, "{body}");
}

#[tokio::test]
async fn a_missing_proposal_or_tag_is_not_found() {
    let c = copilot().await;
    let (status, _) = c.send("GET", "/api/v1/ai/proposals/999999", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An update of a tag that is gone is not reported as done.
    let (status, body) = c
        .send(
            "PUT",
            "/api/v1/data-sources/999999",
            Some(json!({"warn_val": 1})),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
