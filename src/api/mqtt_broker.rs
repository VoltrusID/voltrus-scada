use axum::Json;
use axum::extract::{Extension, State};
use axum::extract::{Path, Query};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use crate::api::{AppError, AppState, AuthUser};
use crate::mqtt_broker::{self, BrokerStatus};

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttConfigSummary {
    pub port: u16,
    pub max_connections: u32,
    pub auth_enabled: bool,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttStatusResponse {
    pub status: String,
    pub connected_clients: usize,
    pub clients: Vec<crate::mqtt_broker::ConnectedClient>,
    pub config: Option<MqttConfigSummary>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttConfigUpdateResponse {
    pub message: String,
    pub config: Option<MqttConfigSummary>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttActionStatusResponse {
    pub message: String,
    pub status: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttTopicMessagesResponse {
    pub topic: String,
    pub messages: Vec<crate::mqtt_broker::RecentMessage>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttMessagesResponse {
    pub messages: Vec<crate::mqtt_broker::RecentMessage>,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct MqttPublishResponse {
    pub message: String,
    pub topic: String,
}

/// GET /api/v1/mqtt-broker/status
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-broker/status",
    responses(
        (status = 200, body = MqttStatusResponse),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_status(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    let status = broker.current_status().await;
    let clients = broker.clients.read().await;

    Ok(Json(json!({
        "status": match &status {
            BrokerStatus::Running => "running",
            BrokerStatus::Stopped => "stopped",
            BrokerStatus::Starting => "starting",
            BrokerStatus::Error(msg) => msg,
        },
        "connected_clients": clients.len(),
        "clients": clients.as_slice(),
        "config": match broker.config.read().await.as_ref() {
            Some(c) => json!({
                "port": c.port,
                "max_connections": c.max_connections,
                "auth_enabled": c.auth.mode != "none" && !c.auth.users.is_empty(),
            }),
            None => serde_json::Value::Null,
        },
    })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateMqttConfigRequest {
    pub port: Option<u16>,
    pub max_connections: Option<u32>,
    pub auth: Option<MqttAuthUpdate>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MqttAuthUpdate {
    pub enabled: Option<bool>,
    pub users: Option<Vec<MqttUserUpdate>>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct MqttUserUpdate {
    pub username: String,
    pub password: String,
}

/// PUT /api/v1/mqtt-broker/config
#[utoipa::path(
    put,
    path = "/api/v1/mqtt-broker/config",
    request_body = UpdateMqttConfigRequest,
    responses(
        (status = 200, body = MqttConfigUpdateResponse),
        (status = 400),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_config(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
    Json(req): Json<UpdateMqttConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    if broker.current_status().await == BrokerStatus::Running {
        return Err(AppError(anyhow::anyhow!(
            "Stop the broker before changing configuration"
        )));
    }

    {
        let mut cfg_guard = broker.config.write().await;
        let mut cfg = cfg_guard.clone().unwrap_or_default();

        if let Some(port) = req.port {
            if port == 0 {
                return Err(AppError(anyhow::anyhow!("Port must be > 0")));
            }
            cfg.port = port;
        }
        if let Some(max_conn) = req.max_connections {
            if max_conn == 0 {
                return Err(AppError(anyhow::anyhow!("max_connections must be > 0")));
            }
            cfg.max_connections = max_conn;
        }
        if let Some(auth_update) = req.auth {
            if let Some(enabled) = auth_update.enabled {
                cfg.auth.mode = if enabled {
                    "password".to_string()
                } else {
                    "none".to_string()
                };
            }
            // Users cannot be read back, so a client cannot resend the full list:
            // merge by username (add new users, change passwords) and keep the rest.
            if let Some(users) = auth_update.users {
                for u in users {
                    match cfg.auth.users.iter_mut().find(|e| e.username == u.username) {
                        Some(existing) => existing.password = u.password,
                        None => cfg.auth.users.push(crate::models::MqttUser {
                            username: u.username,
                            password: u.password,
                        }),
                    }
                }
            }
        }

        cfg.enabled = true;
        *cfg_guard = Some(cfg);
    }

    let cfg = broker.config.read().await;
    Ok(Json(json!({
        "message": "Configuration updated. Start the broker to apply changes.",
        "config": match cfg.as_ref() {
            Some(c) => json!({
                "port": c.port,
                "max_connections": c.max_connections,
                "auth_enabled": c.auth.mode != "none" && !c.auth.users.is_empty(),
            }),
            None => serde_json::Value::Null,
        },
    })))
}

/// POST /api/v1/mqtt-broker/start
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-broker/start",
    responses(
        (status = 200, body = MqttActionStatusResponse),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn start_broker(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    let config = broker.config.read().await.clone().ok_or_else(|| {
        AppError(anyhow::anyhow!(
            "No MQTT broker configuration. Set config via PUT /api/v1/mqtt-broker/config first."
        ))
    })?;

    mqtt_broker::start_broker(broker.clone(), config)
        .await
        .map_err(|e| AppError(anyhow::anyhow!("{}", e)))?;

    Ok(Json(json!({
        "message": "MQTT broker started",
        "status": "running",
    })))
}

/// POST /api/v1/mqtt-broker/stop
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-broker/stop",
    responses(
        (status = 200, body = MqttActionStatusResponse),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn stop_broker(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    mqtt_broker::stop_broker(broker.clone())
        .await
        .map_err(|e| AppError(anyhow::anyhow!("{}", e)))?;

    Ok(Json(json!({
        "message": "MQTT broker stopped",
        "status": "stopped",
    })))
}

/// GET /api/v1/mqtt-broker/topics
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-broker/topics",
    responses(
        (status = 200, body = serde_json::Value),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_topics(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    Ok(Json(
        mqtt_broker::get_topic_tree(&broker.topic_tracker).await,
    ))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct TopicMessagesQuery {
    pub limit: Option<usize>,
}

/// GET /api/v1/mqtt-broker/topics/:topic/messages
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-broker/topics/{topic}/messages",
    params(
        ("topic" = String, Path, description = "Topic name"),
        TopicMessagesQuery
    ),
    responses(
        (status = 200, body = MqttTopicMessagesResponse),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_topic_messages(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
    Path(topic): Path<String>,
    Query(_q): Query<TopicMessagesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    let messages = broker.topic_tracker.get_topic_messages(&topic).await;

    Ok(Json(json!({
        "topic": topic,
        "messages": messages,
    })))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct MessagesQuery {
    pub limit: Option<usize>,
}

/// GET /api/v1/mqtt-broker/messages
#[utoipa::path(
    get,
    path = "/api/v1/mqtt-broker/messages",
    params(MessagesQuery),
    responses(
        (status = 200, body = MqttMessagesResponse),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn get_messages(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
    Query(q): Query<MessagesQuery>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    let messages = broker
        .topic_tracker
        .get_recent_messages(q.limit.unwrap_or(100))
        .await;

    Ok(Json(json!({
        "messages": messages,
    })))
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct PublishRequest {
    pub topic: String,
    pub payload: String,
}

/// POST /api/v1/mqtt-broker/publish
#[utoipa::path(
    post,
    path = "/api/v1/mqtt-broker/publish",
    request_body = PublishRequest,
    responses(
        (status = 200, body = MqttPublishResponse),
        (status = 400),
        (status = 403),
    ),
    tag = "MQTT Broker",
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn publish_message(
    State(state): State<AppState>,
    Extension(_user): Extension<AuthUser>,
    Json(req): Json<PublishRequest>,
) -> Result<impl IntoResponse, AppError> {
    let broker = state
        .mqtt_broker_state
        .as_ref()
        .ok_or_else(|| AppError(anyhow::anyhow!("MQTT broker module not initialized")))?;

    if req.topic.is_empty() {
        return Err(AppError(anyhow::anyhow!("Topic cannot be empty")));
    }

    broker
        .publish_sync(&req.topic, req.payload.as_bytes())
        .map_err(|e| AppError(anyhow::anyhow!("{}", e)))?;

    Ok(Json(json!({
        "message": "Published",
        "topic": req.topic,
    })))
}
