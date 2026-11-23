use crate::api::{AppError, AppState, AuthUser, Rejected, RejectedJson};
use crate::flow_engine::engine::{FireError, Flow as RuntimeFlow};
use crate::flow_engine::executor::{self, ExecEnv, ExecResult};
use crate::flow_engine::graph::{FlowGraph, GraphError, TRIGGER_ID};
use crate::flow_engine::message::Message;
use crate::flow_engine::trigger::TriggerConfig;
use crate::models::Flow;
use axum::{
    Json,
    body::Bytes,
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::HashMap;

/// Flow ids the routes under `/api/v1/flows/` use for themselves.
const RESERVED_FLOW_IDS: &[&str] = &["test", "node-types"];

/// Webhook calls accepted per flow per minute (wrong tokens included).
pub const WEBHOOK_MAX_PER_MINUTE: u32 = 30;

fn reject(status: StatusCode, message: impl Into<String>) -> AppError {
    AppError(anyhow::Error::new(Rejected(status, message.into())))
}

/// 400 with `{error, node_id}`; `node_id` names the node the problem is about
/// (null when it is about the trigger or the document as a whole).
fn invalid(e: GraphError) -> AppError {
    AppError(anyhow::Error::new(RejectedJson(
        StatusCode::BAD_REQUEST,
        json!({"error": e.message, "node_id": e.node_id}),
    )))
}

/// A flow as `POST`/`PUT /api/v1/flows` take it. `trigger_config` and `nodes`
/// may be JSON text (as stored and as `GET` returns them) or JSON values;
/// `nodes` may be the legacy array (v1) or the graph object (v2).
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct FlowInput {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub enabled: bool,
    #[schema(value_type = Object)]
    pub trigger_config: Value,
    #[schema(value_type = Object)]
    pub nodes: Value,
}

/// Parse and check a flow definition; `nodes` comes back as the v2 graph.
pub fn check_definition(
    trigger_config: &Value,
    nodes: &Value,
) -> Result<(TriggerConfig, FlowGraph, String), GraphError> {
    let trigger = TriggerConfig::from_value(trigger_config.clone()).map_err(GraphError::new)?;
    let graph = FlowGraph::from_value(nodes.clone())?;
    // Keep the trigger as given (fields an editor adds survive), as JSON text.
    let trigger_text = match trigger_config {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    };
    Ok((trigger, graph, trigger_text))
}

/// The stored flow with `nodes` as the v2 graph (v1 converted). A flow whose
/// nodes no longer parse is returned as stored.
fn with_v2_nodes(mut flow: Flow) -> Flow {
    if let Ok(graph) = FlowGraph::parse(&flow.nodes) {
        flow.nodes = graph.to_json_string();
    }
    flow
}

#[utoipa::path(
/// The flow as `role` may read it: roles that may not configure see the
/// headers and URL credentials of `http_out` nodes masked.
fn readable_by(mut flow: Flow, role: &str) -> Flow {
    if !crate::api::role_allows_configuration(role) {
        flow.nodes = crate::security::redact::mask_flow_nodes(&flow.nodes);
    }
    flow
}

    get,
    path = "/api/v1/flows",
    tag = "Flows",
    responses(
        (status = 200, body = serde_json::Value, description = "`{flows: [Flow]}`; each flow's `nodes` is the v2 graph as JSON text"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_flows(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AppError> {
    Extension(auth): Extension<AuthUser>,
    let flows: Vec<Flow> = state
        .storage
        .list_flows()
        .await?
        .into_iter()
        .map(|f| readable_by(with_v2_nodes(f), &auth.role))
        .collect();
    Ok(Json(json!({ "flows": flows })))
}

#[utoipa::path(
    get,
    path = "/api/v1/flows/{id}",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, body = Flow, description = "`nodes` is the v2 graph as JSON text"),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Extension(auth): Extension<AuthUser>,
) -> Result<Json<Flow>, AppError> {
    let flow = state
        .storage
        .get_flow(&id)
        .await?
        .ok_or_else(|| AppError::not_found("Flow not found"))?;
    Ok(Json(readable_by(with_v2_nodes(flow), &auth.role)))
}

#[utoipa::path(
    post,
    path = "/api/v1/flows",
    tag = "Flows",
    request_body = FlowInput,
    responses(
        (status = 200, body = serde_json::Value, description = "`{id}`"),
        (status = 400, description = "Invalid trigger or graph; the message names the node"),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<FlowInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = req.id.trim().to_string();
    if id.is_empty() {
        return Err(AppError::bad_request("id must not be empty"));
    }
    if RESERVED_FLOW_IDS.contains(&id.as_str()) {
        return Err(AppError::bad_request(format!("flow id '{id}' is reserved")));
    }
    let (trigger, graph, trigger_text) =
        check_definition(&req.trigger_config, &req.nodes).map_err(invalid)?;
    if state.storage.get_flow(&id).await?.is_some() {
        return Err(reject(
            StatusCode::CONFLICT,
            format!("flow '{id}' already exists"),
        ));
    }
    state
        .storage
        .create_flow(
            &id,
            &req.name,
            req.enabled,
            &trigger_text,
            &graph.to_json_string(),
        )
        .await?;
    if let Some(engine) = &state.flow_engine {
        engine
            .add_flow(RuntimeFlow {
                id: id.clone(),
                name: req.name.clone(),
                enabled: req.enabled,
                trigger,
                graph,
            })
            .await;
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_flow".to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: None,
    });
    Ok(Json(json!({ "id": id })))
}

#[utoipa::path(
    put,
    path = "/api/v1/flows/{id}",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    request_body = FlowInput,
    responses(
        (status = 204),
        (status = 400, description = "Invalid trigger or graph; the message names the node"),
        (status = 403),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<FlowInput>,
) -> Result<StatusCode, AppError> {
    let (trigger, graph, trigger_text) =
        check_definition(&req.trigger_config, &req.nodes).map_err(invalid)?;
    let Some(stored) = state.storage.get_flow(&id).await? else {
        return Err(AppError::not_found("Flow not found"));
    };
    // An editor that read the flow masked sends the mask back: keep the
    // stored header values and URL credentials of those http_out nodes.
    let mut doc = graph.to_value();
    let (trigger, graph, trigger_text) =
        if crate::security::redact::restore_flow_graph(&mut doc, Some(&stored.nodes)) {
            check_definition(&req.trigger_config, &doc).map_err(invalid)?
        } else {
            (trigger, graph, trigger_text)
        };
    state
        .storage
        .update_flow(
            &id,
            &req.name,
            req.enabled,
            &trigger_text,
            &graph.to_json_string(),
        )
        .await?;
    if let Some(engine) = &state.flow_engine {
        engine
            .update_flow(RuntimeFlow {
                id: id.clone(),
                name: req.name.clone(),
                enabled: req.enabled,
                trigger,
                graph,
            })
            .await;
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "update_flow".to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: None,
    });
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    path = "/api/v1/flows/{id}",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 204),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    state.storage.delete_flow(&id).await?;
    if let Some(engine) = &state.flow_engine {
        engine.remove_flow(&id).await;
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "delete_flow".to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: None,
    });
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/start",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 204),
        (status = 403),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn start_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    super::signature::MaybeSigned(signature): super::signature::MaybeSigned,
) -> Result<axum::response::Response, AppError> {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    set_flow_enabled(&state, &auth, origin, &id, true, signature.as_ref()).await
}

#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/stop",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 204),
        (status = 403),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn stop_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    super::signature::MaybeSigned(signature): super::signature::MaybeSigned,
) -> Result<axum::response::Response, AppError> {
    let origin = super::signature::origin_of(&auth, api_key.is_some());
    set_flow_enabled(&state, &auth, origin, &id, false, signature.as_ref()).await
}

/// Persist `enabled` on the stored flow, then mirror it into the running
/// engine, so a restart or reload keeps what start/stop did. Starting or
/// stopping a flow is a mode switch an add-on edition may want signed.
pub(crate) async fn set_flow_enabled(
    state: &AppState,
    auth: &AuthUser,
    origin: crate::edition::ActionOrigin,
    id: &str,
    enabled: bool,
    signature: Option<&crate::models::SignatureInput>,
) -> Result<axum::response::Response, AppError> {
    let Some(flow) = state.storage.get_flow(id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    // A flow that is invalid as stored cannot be started.
    let parsed = if enabled {
        Some(RuntimeFlow::from_db(&flow).map_err(AppError::bad_request)?)
    } else {
        None
    };
    let signed = match super::signature::authorize_action(
        state,
        auth,
        origin,
        crate::edition::SignedAction::ModeSwitch,
        format!("flow:{id}"),
        serde_json::json!({"mode": if enabled { "start" } else { "stop" }, "flow_id": id, "name": flow.name}),
        signature,
    )
    .await
    {
        Ok(signed) => signed,
        Err(e) => return Ok(e.into_response()),
    };
    state
        .storage
        .update_flow(id, &flow.name, enabled, &flow.trigger_config, &flow.nodes)
        .await?;
    if let Some(engine) = &state.flow_engine {
        match (engine.get_flow(id).await, parsed) {
            (Some(mut running), _) => {
                running.enabled = enabled;
                engine.update_flow(running).await;
            }
            // Stopped at startup, so the engine never loaded it.
            (None, Some(mut parsed)) => {
                parsed.enabled = true;
                engine.add_flow(parsed).await;
            }
            (None, None) => {}
        }
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: if enabled { "start_flow" } else { "stop_flow" }.to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: signed
            .as_ref()
            .map(|s| format!("{} reason={}", s.audit_suffix().trim_start(), s.reason)),
    });
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `{payload, metadata}` a run or test run starts with.
#[derive(Debug, Clone, Default, Deserialize, utoipa::ToSchema)]
pub struct RunInput {
    #[serde(default)]
    #[schema(value_type = Object)]
    pub payload: Value,
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl RunInput {
    fn into_message(self) -> Message {
        let payload = if self.payload.is_null() {
            json!({})
        } else {
            self.payload
        };
        let mut msg = Message::new(payload);
        msg.metadata = self.metadata;
        msg
    }
}

/// The flow under test: what an editor has, saved or not.
#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct TestFlowDefinition {
    /// Optional; only names the flow in the trace (`flow_id` in templates).
    #[serde(default)]
    pub id: Option<String>,
    #[schema(value_type = Object)]
    pub trigger_config: Value,
    #[schema(value_type = Object)]
    pub nodes: Value,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
pub struct TestFlowRequest {
    pub flow: TestFlowDefinition,
    #[serde(default)]
    pub input: RunInput,
}

fn result_json(r: &ExecResult) -> Value {
    json!({
        "ok": r.ok,
        "error": r.error,
        "trace": r.trace,
        "outputs_reached": r.outputs_reached,
    })
}

async fn dry_run(state: &AppState, flow_id: &str, graph: &FlowGraph, msg: Message) -> ExecResult {
    let env = ExecEnv {
        flow_id: flow_id.to_string(),
        // Fresh memory: a test run neither sees nor changes what live runs remember.
        state: crate::flow_engine::state::FlowState::new(),
        storage: state.storage.clone(),
        cache: state.cache.clone(),
        smtp: None,
        dry_run: true,
    };
    executor::execute(graph, TRIGGER_ID, 0, msg, &env).await
}

/// Test-run a flow definition (saved or not) on a sample message. Nothing acts
/// outside the flow: write, HTTP, notify, alarm and tag-out nodes report what
/// they would have done (`side_effect_skipped: true`) and delay does not wait.
#[utoipa::path(
    post,
    path = "/api/v1/flows/test",
    tag = "Flows",
    request_body = TestFlowRequest,
    responses(
        (status = 200, body = serde_json::Value, description = "`{ok, error, trace: [{node_id, type, input, output, port, ports, error, duration_us, side_effect_skipped}], outputs_reached}`"),
        (status = 400, description = "Invalid trigger or graph"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn test_flow_definition(
    State(state): State<AppState>,
    Json(req): Json<TestFlowRequest>,
) -> Result<Json<Value>, AppError> {
    let (_, graph, _) =
        check_definition(&req.flow.trigger_config, &req.flow.nodes).map_err(invalid)?;
    let flow_id = req.flow.id.unwrap_or_else(|| "test".to_string());
    let r = dry_run(&state, &flow_id, &graph, req.input.into_message()).await;
    Ok(Json(result_json(&r)))
}

/// Test-run a saved flow on `{payload}` (dry run, as `POST /api/v1/flows/test`).
#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/test",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    request_body = RunInput,
    responses(
        (status = 200, body = serde_json::Value, description = "The test run result plus the older `success`, `output` and `node_results` fields"),
        (status = 400),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn test_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RunInput>,
) -> Result<Json<serde_json::Value>, AppError> {
    let flow = state
        .storage
        .get_flow(&id)
        .await?
        .ok_or_else(|| AppError::not_found("Flow not found"))?;
    let graph = FlowGraph::parse(&flow.nodes).map_err(invalid)?;
    let r = dry_run(&state, &flow.id, &graph, req.into_message()).await;
    let node_results: Vec<Value> = r
        .trace
        .iter()
        .map(|t| match &t.error {
            Some(e) => json!({"node_id": t.node_id, "error": e}),
            None => json!({"node_id": t.node_id, "port": t.port, "output": t.output}),
        })
        .collect();
    let mut out = result_json(&r);
    out["success"] = json!(true);
    out["output"] = r.last_payload.clone().unwrap_or(Value::Null);
    out["node_results"] = json!(node_results);
    Ok(Json(out))
}

/// Run a saved flow now, for real (run button / manual trigger), whatever its
/// trigger and whether or not it is started. Body (optional): `{input: {payload, metadata}}`.
#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/run",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    request_body(content = serde_json::Value, description = "Optional `{input: {payload, metadata}}`"),
    responses(
        (status = 200, body = serde_json::Value, description = "`{ok, error, trace, outputs_reached}`; the run is also in the run log"),
        (status = 400),
        (status = 404),
        (status = 409, description = "A run of this flow is in progress"),
        (status = 503, description = "Passive node of a redundant pair"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn run_flow(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    api_key: Option<Extension<super::ApiKeyAuth>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<axum::response::Response, AppError> {
    #[derive(Deserialize, Default)]
    struct Body {
        #[serde(default)]
        input: RunInput,
        /// When the edition wants a manual run signed (428).
        #[serde(default)]
        signature: Option<crate::models::SignatureInput>,
    }
    let body: Body = if body.iter().all(u8::is_ascii_whitespace) {
        Body::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("invalid body: {e}")))?
    };
    let stored = state
        .storage
        .get_flow(&id)
        .await?
        .ok_or_else(|| AppError::not_found("Flow not found"))?;
    let flow = RuntimeFlow::from_db(&stored).map_err(AppError::bad_request)?;
    // Running a flow by hand is a mode switch the edition may want signed.
    // The writes the run makes are still asked as flow writes (origin `flow`).
    let signed = match super::signature::authorize_action(
        &state,
        &auth,
        super::signature::origin_of(&auth, api_key.is_some()),
        crate::edition::SignedAction::ModeSwitch,
        format!("flow:{id}"),
        json!({"mode": "run", "flow_id": id, "name": stored.name}),
        body.signature.as_ref(),
    )
    .await
    {
        Ok(signed) => signed,
        Err(e) => return Ok(e.into_response()),
    };
    let mut msg = body.input.into_message();
    msg.metadata
        .entry("source".into())
        .or_insert_with(|| "manual".into());
    msg.metadata.insert("user".into(), auth.username.clone());
    let result = match &state.flow_engine {
        Some(engine) => engine.run_now(flow, msg).await.map_err(fire_error)?,
        None => {
            if !crate::node_role::is_active() {
                return Err(fire_error(FireError::Passive));
            }
            let smtp = state.smtp_config.read().await.clone();
            crate::flow_engine::engine::run_once(&state.storage, &state.cache, smtp, &flow, msg)
                .await
        }
    };
    state.audit_logger.log(crate::security::AuditEvent {
        action: "run_flow".to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: signed
            .as_ref()
            .map(|s| format!("{} reason={}", s.audit_suffix().trim_start(), s.reason)),
    });
    Ok(Json(result_json(&result)).into_response())
}

fn fire_error(e: FireError) -> AppError {
    let status = match e {
        FireError::NotFound => StatusCode::NOT_FOUND,
        FireError::Passive => StatusCode::SERVICE_UNAVAILABLE,
        FireError::Disabled | FireError::WrongTrigger | FireError::Busy => StatusCode::CONFLICT,
    };
    reject(status, e.message())
}

/// The node and trigger catalogue the flow editors build their palette and
/// forms from.
#[utoipa::path(
    get,
    path = "/api/v1/flows/node-types",
    tag = "Flows",
    responses(
        (status = 200, body = serde_json::Value, description = "`{triggers: [{type, label, description, fields}], nodes: [{type, label, category, description, outputs, side_effect, fields}]}`"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn node_types() -> Json<Value> {
    Json(crate::flow_engine::catalogue::catalogue())
}

fn sha256_hex(text: &str) -> String {
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// Create (or replace) a flow's webhook token. The token is returned once;
/// only its hash is stored.
#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/webhook-token",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, body = serde_json::Value, description = "`{token}`, shown only now"),
        (status = 403),
        (status = 404),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_webhook_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthUser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let token = hex::encode(rand::random::<[u8; 32]>());
    if !state
        .storage
        .set_flow_webhook_token_hash(&id, &sha256_hex(&token))
        .await?
    {
        return Err(AppError::not_found("Flow not found"));
    }
    state.audit_logger.log(crate::security::AuditEvent {
        action: "create_flow_webhook_token".to_string(),
        username: auth.username.clone(),
        resource: format!("flow:{}", id),
        details: None,
    });
    Ok(Json(json!({ "token": token })))
}

/// flow id -> (start of the current minute window, calls in it).
static WEBHOOK_CALLS: std::sync::LazyLock<dashmap::DashMap<String, (std::time::Instant, u32)>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

/// Count a webhook call; false when the flow had its share this minute.
fn webhook_allowed(flow_id: &str) -> bool {
    let now = std::time::Instant::now();
    let mut entry = WEBHOOK_CALLS.entry(flow_id.to_string()).or_insert((now, 0));
    if now.duration_since(entry.0) >= std::time::Duration::from_secs(60) {
        *entry = (now, 0);
    }
    entry.1 += 1;
    entry.1 <= WEBHOOK_MAX_PER_MINUTE
}

/// Fire a flow with a webhook trigger. Needs the flow's token in
/// `X-Flow-Token`; the JSON body (optional) becomes the payload. No session
/// or API key. At most [`WEBHOOK_MAX_PER_MINUTE`] calls per flow per minute.
#[utoipa::path(
    post,
    path = "/api/v1/flows/{id}/webhook",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID"),
        ("X-Flow-Token" = String, Header, description = "The flow's webhook token"),
    ),
    request_body(content = serde_json::Value, description = "Becomes the message payload"),
    responses(
        (status = 202, body = serde_json::Value, description = "`{accepted: true}`; the run happens in the background"),
        (status = 400, description = "Body is not JSON"),
        (status = 401, description = "Missing or wrong token"),
        (status = 409, description = "Flow stopped, not a webhook flow, or a run is in progress"),
        (status = 429, description = "Too many calls this minute"),
        (status = 503),
    )
)]
pub async fn flow_webhook(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), AppError> {
    if !webhook_allowed(&id) {
        return Err(reject(
            StatusCode::TOO_MANY_REQUESTS,
            "too many webhook calls for this flow; try again in a minute",
        ));
    }
    let token = headers
        .get("x-flow-token")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .unwrap_or("");
    let stored = if token.is_empty() {
        None
    } else {
        state.storage.get_flow_webhook_token_hash(&id).await?
    };
    // Both sides are SHA-256 digests, so comparing them leaks nothing about the token.
    if stored.is_none_or(|hash| hash != sha256_hex(token)) {
        return Err(reject(
            StatusCode::UNAUTHORIZED,
            "missing or wrong X-Flow-Token",
        ));
    }
    let payload: Value = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| AppError::bad_request(format!("body is not JSON: {e}")))?
    };
    let Some(engine) = &state.flow_engine else {
        return Err(reject(
            StatusCode::SERVICE_UNAVAILABLE,
            "the flow engine is not running",
        ));
    };
    match engine.fire_webhook(&id, payload).await {
        Ok(()) => Ok((StatusCode::ACCEPTED, Json(json!({"accepted": true})))),
        // Stopped at startup: the engine does not hold it.
        Err(FireError::NotFound) => Err(fire_error(FireError::Disabled)),
        Err(e) => Err(fire_error(e)),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/flows/{id}/runs",
    tag = "Flows",
    params(
        ("id" = String, Path, description = "Flow ID")
    ),
    responses(
        (status = 200, body = serde_json::Value, description = "`{runs: [FlowRun]}`, newest first; the newest runs carry a per-node `trace`"),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_flow_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let runs = state.storage.list_flow_runs(&id, 20).await?;
    Ok(Json(json!({ "runs": runs })))
}
