use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState};
use crate::models::*;
use crate::notification::chat;

/// GET /api/v1/notifications/settings
#[utoipa::path(
    get,
    path = "/api/v1/notifications/settings",
    tag = "Notifications",
    responses(
        (status = 200, body = Vec<NotificationSetting>),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn list_notification_settings(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let mut settings = state.storage.get_all_notification_settings().await?;
    for s in &mut settings {
        s.config = chat::mask_config(&s.channel_type, &s.config);
    }
    Ok(Json(settings))
}

fn validate_channel_type(channel_type: &str) -> Result<(), AppError> {
    match channel_type {
        "email" | "webhook" | "push" | "telegram" | "whatsapp" => Ok(()),
        _ => Err(AppError::bad_request(
            "channel_type must be 'email', 'webhook', 'push', 'telegram', or 'whatsapp'",
        )),
    }
}

/// Telegram and WhatsApp are Enterprise add-on features.
fn ensure_chat_allowed(state: &AppState, channel_type: &str) -> Result<(), AppError> {
    if chat::is_chat_channel(channel_type) && !state.tier().chat_notifications() {
        return Err(AppError::forbidden(
            "Telegram and WhatsApp notifications require an Integrator or Enterprise licence",
        ));
    }
    Ok(())
}

/// The targets of an email, webhook or push setting: a non-empty JSON array of strings.
fn string_targets(config: &str) -> Result<Vec<String>, AppError> {
    let targets: Vec<String> = serde_json::from_str(config).map_err(|_| {
        AppError::bad_request(
            "config must be a JSON array of strings (email addresses or webhook URLs)",
        )
    })?;
    if targets.is_empty() {
        return Err(AppError::bad_request("config array must not be empty"));
    }
    Ok(targets)
}

/// Each email address has an `@` and a `.`, each webhook URL starts with `http://` or `https://`.
fn check_string_targets(channel_type: &str, targets: &[String]) -> Result<(), AppError> {
    match channel_type {
        "email" => {
            if let Some(bad) = targets
                .iter()
                .find(|t| !t.contains('@') || !t.contains('.'))
            {
                return Err(AppError::bad_request(format!(
                    "invalid email address: {bad}"
                )));
            }
        }
        "webhook" => {
            if let Some(bad) = targets
                .iter()
                .find(|t| !t.starts_with("http://") && !t.starts_with("https://"))
            {
                return Err(AppError::bad_request(format!(
                    "invalid webhook URL (must start with http:// or https://): {bad}"
                )));
            }
        }
        _ => {}
    }
    Ok(())
}

/// The checks `POST` and `PUT /notifications/settings` share: `config` has the
/// shape its channel needs.
fn validate_config(channel_type: &str, config: &str) -> Result<(), AppError> {
    if chat::is_chat_channel(channel_type) {
        chat::validate(channel_type, config).map_err(AppError::bad_request)
    } else {
        check_string_targets(channel_type, &string_targets(config)?)
    }
}

/// POST /api/v1/notifications/settings
#[utoipa::path(
    post,
    path = "/api/v1/notifications/settings",
    tag = "Notifications",
    request_body = CreateNotificationSettingRequest,
    responses(
        (status = 201, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn create_notification_setting(
    State(state): State<AppState>,
    Json(req): Json<CreateNotificationSettingRequest>,
) -> Result<impl IntoResponse, AppError> {
    validate_channel_type(&req.channel_type)?;
    ensure_chat_allowed(&state, &req.channel_type)?;
    validate_config(&req.channel_type, &req.config)?;

    let id = state
        .storage
        .create_notification_setting(&req.source_key, &req.channel_type, &req.config, req.enabled)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "source_key": req.source_key,
            "channel_type": req.channel_type,
            "config": chat::mask_config(&req.channel_type, &req.config),
            "enabled": req.enabled,
        })),
    ))
}

/// PUT /api/v1/notifications/settings/{id}
#[utoipa::path(
    put,
    path = "/api/v1/notifications/settings/{id}",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification setting ID")
    ),
    request_body = UpdateNotificationSettingRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn update_notification_setting(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateNotificationSettingRequest>,
) -> Result<impl IntoResponse, AppError> {
    if let Some(ct) = &req.channel_type {
        validate_channel_type(ct)?;
        ensure_chat_allowed(&state, ct)?;
    }

    // The config has to suit the channel it ends up with: the new values
    // where sent, the stored ones for the rest. A masked secret stands for the
    // stored one.
    let mut config = req.config.clone();
    if req.channel_type.is_some() || req.config.is_some() {
        let current = state
            .storage
            .get_all_notification_settings()
            .await?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or_else(|| AppError::not_found("Notification setting not found"))?;
        let channel = req.channel_type.as_deref().unwrap_or(&current.channel_type);
        let same_channel = channel == current.channel_type;
        let effective = match &req.config {
            Some(new) => chat::restore_secret(
                channel,
                new,
                same_channel.then_some(current.config.as_str()),
            )
            .map_err(AppError::bad_request)?,
            None => current.config.clone(),
        };
        validate_config(channel, &effective)?;
        if req.config.is_some() {
            config = Some(effective);
        }
    }

    state
        .storage
        .update_notification_setting(
            id,
            req.source_key.as_deref(),
            req.channel_type.as_deref(),
            config.as_deref(),
            req.enabled,
        )
        .await?;

    Ok(Json(json!({"ok": true})))
}

/// DELETE /api/v1/notifications/settings/{id}
#[utoipa::path(
    delete,
    path = "/api/v1/notifications/settings/{id}",
    tag = "Notifications",
    params(
        ("id" = i64, Path, description = "Notification setting ID")
    ),
    responses(
        (status = 200, body = serde_json::Value),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn delete_notification_setting(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, AppError> {
    state.storage.delete_notification_setting(id).await?;
    Ok(Json(json!({"ok": true})))
}

/// POST /api/v1/notifications/test
///
/// Sends a test notification to validate configuration without creating an actual alarm.
#[utoipa::path(
    post,
    path = "/api/v1/notifications/test",
    tag = "Notifications",
    request_body = TestNotificationRequest,
    responses(
        (status = 200, body = serde_json::Value),
        (status = 400),
        (status = 403),
    ),
    security(("api_key" = []), ("cookie_auth" = []))
)]
pub async fn test_notification(
    State(state): State<AppState>,
    Json(req): Json<TestNotificationRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.channel_type != "email"
        && req.channel_type != "webhook"
        && !chat::is_chat_channel(&req.channel_type)
    {
        return Err(AppError::bad_request(
            "channel_type must be 'email', 'webhook', 'telegram', or 'whatsapp' (push is tested via browser subscription)",
        ));
    }

    ensure_chat_allowed(&state, &req.channel_type)?;

    let source_key = req.source_key.as_deref().unwrap_or("test_source");
    let source_name = format!("Test Notification ({})", source_key);

    match req.channel_type.as_str() {
        "email" => {
            let targets = string_targets(&req.config)?;
            check_string_targets("email", &targets)?;

            let alarm_data = crate::notification::email::AlarmEmailData {
                source_key: source_key.to_string(),
                source_name: source_name.clone(),
                severity: "warning".to_string(),
                value: 42.0,
                threshold: 40.0,
                message: "This is a test alarm notification".to_string(),
                unit: "test".to_string(),
                triggered_at: crate::notification::webhook::format_triggered_at(
                    crate::timeutil::now_secs(),
                ),
            };

            let smtp_cfg = state.smtp_config.read().await;
            match smtp_cfg.as_ref() {
                Some(smtp_cfg) => {
                    let notifier = crate::notification::email::EmailNotifier::new(smtp_cfg)?;
                    let mut results = Vec::new();
                    for to in &targets {
                        match notifier.send_alarm_email(to, &alarm_data).await {
                            Ok(_) => results.push(json!({"to": to, "status": "sent"})),
                            Err(e) => results
                                .push(json!({"to": to, "status": "error", "error": e.to_string()})),
                        }
                    }
                    Ok(Json(json!({
                        "channel": "email",
                        "results": results,
                    })))
                }
                None => Err(AppError::bad_request("SMTP is not configured")),
            }
        }
        "webhook" => {
            let targets = string_targets(&req.config)?;
            check_string_targets("webhook", &targets)?;

            let payload = crate::notification::webhook::WebhookPayload {
                severity: "warning".to_string(),
                source_key: source_key.to_string(),
                source_name: source_name.clone(),
                value: 42.0,
                threshold: 40.0,
                message: "This is a test alarm notification".to_string(),
                triggered_at: crate::notification::webhook::format_triggered_at(
                    crate::timeutil::now_secs(),
                ),
                device_info: source_key.to_string(),
            };

            let mut results = Vec::new();
            for url in &targets {
                match crate::notification::webhook::send_webhook_alarm(url, &payload).await {
                    Ok(_) => results.push(json!({"url": url, "status": "sent"})),
                    Err(e) => {
                        results.push(json!({"url": url, "status": "error", "error": e.to_string()}))
                    }
                }
            }

            Ok(Json(json!({
                "channel": "webhook",
                "results": results,
            })))
        }
        channel @ ("telegram" | "whatsapp") => {
            // The editor sends the secret as the mask for a stored rule.
            let stored = match req.setting_id {
                Some(id) => state
                    .storage
                    .get_all_notification_settings()
                    .await?
                    .into_iter()
                    .find(|s| s.id == id && s.channel_type == channel)
                    .map(|s| s.config),
                None => None,
            };
            let config = chat::restore_secret(channel, &req.config, stored.as_deref())
                .map_err(AppError::bad_request)?;
            chat::validate(channel, &config).map_err(AppError::bad_request)?;

            let Some(sender) = crate::edition::chat_sender() else {
                return Err(AppError::bad_request(
                    "Telegram and WhatsApp delivery is not available in this edition",
                ));
            };
            let alarm = crate::edition::ChatAlarm {
                severity: "warning".to_string(),
                source_key: source_key.to_string(),
                source_name: source_name.clone(),
                value: 42.0,
                threshold: 40.0,
                unit: "test".to_string(),
                message: "This is a test alarm notification".to_string(),
                triggered_at: crate::notification::webhook::format_triggered_at(
                    crate::timeutil::now_secs(),
                ),
            };
            let deliveries = sender
                .send(channel, &config, &alarm)
                .await
                .map_err(|e| AppError::bad_request(e.to_string()))?;
            let results: Vec<_> = deliveries
                .into_iter()
                .map(|d| match d.error {
                    None => json!({"target": d.target, "status": "sent"}),
                    Some(e) => json!({"target": d.target, "status": "error", "error": e}),
                })
                .collect();
            Ok(Json(json!({
                "channel": channel,
                "results": results,
            })))
        }
        _ => unreachable!(),
    }
}
