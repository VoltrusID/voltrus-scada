use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

use super::{AppError, AppState};
use crate::models::*;

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
    let settings = state.storage.get_all_notification_settings().await?;
    Ok(Json(settings))
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
    // Validate channel_type
    if req.channel_type != "email"
        && req.channel_type != "webhook"
        && req.channel_type != "push"
        && req.channel_type != "sms"
    {
        return Err(AppError(anyhow::anyhow!(
            "channel_type must be 'email', 'webhook', 'push', or 'sms'"
        )));
    }

    // SMS is an Enterprise add-on feature.
    if req.channel_type == "sms" && !state.tier().sms_notifications() {
        return Err(AppError(anyhow::anyhow!(
            "SMS notifications require an Integrator or Enterprise licence"
        )));
    }

    // Validate config based on channel type
    if req.channel_type == "sms" {
        // SMS config: JSON array of objects with "phone" and "carrier" fields
        let sms_targets: Vec<serde_json::Value> = match serde_json::from_str(&req.config) {
            Ok(v) => v,
            Err(_) => {
                return Err(AppError(anyhow::anyhow!(
                    "SMS config must be a JSON array of objects with 'phone' and 'carrier' fields"
                )));
            }
        };
        if sms_targets.is_empty() {
            return Err(AppError(anyhow::anyhow!("config array must not be empty")));
        }
        for target in &sms_targets {
            let phone = target["phone"].as_str().unwrap_or("");
            let carrier = target["carrier"].as_str().unwrap_or("");
            if phone.is_empty() {
                return Err(AppError(anyhow::anyhow!(
                    "each SMS target must have a 'phone' field"
                )));
            }
            if carrier.is_empty() {
                return Err(AppError(anyhow::anyhow!(
                    "each SMS target must have a 'carrier' field"
                )));
            }
            if crate::edition::sms_sender()
                .and_then(|s| s.carrier_gateway(carrier))
                .is_none()
            {
                return Err(AppError(anyhow::anyhow!(
                    "unknown carrier '{}'. Supported: att, verizon, tmobile, sprint, uscellular",
                    carrier
                )));
            }
        }
    } else {
        // Email/webhook/push config: JSON array of strings
        let targets: Vec<String> = match serde_json::from_str(&req.config) {
            Ok(v) => v,
            Err(_) => {
                return Err(AppError(anyhow::anyhow!(
                    "config must be a JSON array of strings (email addresses or webhook URLs)"
                )));
            }
        };

        if targets.is_empty() {
            return Err(AppError(anyhow::anyhow!("config array must not be empty")));
        }

        // Validate individual entries based on channel type
        if req.channel_type == "email" {
            for entry in &targets {
                if !entry.contains('@') || !entry.contains('.') {
                    return Err(AppError(anyhow::anyhow!(
                        "invalid email address: {}",
                        entry
                    )));
                }
            }
        } else if req.channel_type == "webhook" {
            for entry in &targets {
                if !entry.starts_with("http://") && !entry.starts_with("https://") {
                    return Err(AppError(anyhow::anyhow!(
                        "invalid webhook URL (must start with http:// or https://): {}",
                        entry
                    )));
                }
            }
        }
    }

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
            "config": req.config,
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
    // Validate channel_type if provided
    if let Some(ref ct) = req.channel_type
        && ct != "email"
        && ct != "webhook"
        && ct != "push"
        && ct != "sms"
    {
        return Err(AppError(anyhow::anyhow!(
            "channel_type must be 'email', 'webhook', 'push', or 'sms'"
        )));
    }

    // Validate config if provided
    if let Some(ref cfg) = req.config {
        let targets: Vec<String> = match serde_json::from_str(cfg) {
            Ok(v) => v,
            Err(_) => {
                return Err(AppError(anyhow::anyhow!(
                    "config must be a JSON array of strings"
                )));
            }
        };
        if targets.is_empty() {
            return Err(AppError(anyhow::anyhow!("config array must not be empty")));
        }
    }

    state
        .storage
        .update_notification_setting(
            id,
            req.source_key.as_deref(),
            req.channel_type.as_deref(),
            req.config.as_deref(),
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
    if req.channel_type != "email" && req.channel_type != "webhook" && req.channel_type != "sms" {
        return Err(AppError(anyhow::anyhow!(
            "channel_type must be 'email', 'webhook', or 'sms' (push is tested via browser subscription)"
        )));
    }

    // SMS is an Enterprise add-on feature.
    if req.channel_type == "sms" && !state.tier().sms_notifications() {
        return Err(AppError(anyhow::anyhow!(
            "SMS notifications require an Integrator or Enterprise licence"
        )));
    }

    let source_key = req.source_key.as_deref().unwrap_or("test_source");
    let source_name = format!("Test Notification ({})", source_key);

    match req.channel_type.as_str() {
        "email" => {
            let targets: Vec<String> = match serde_json::from_str(&req.config) {
                Ok(v) => v,
                Err(_) => {
                    return Err(AppError(anyhow::anyhow!(
                        "config must be a JSON array of email addresses"
                    )));
                }
            };
            if targets.is_empty() {
                return Err(AppError(anyhow::anyhow!("config array must not be empty")));
            }

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
                None => Err(AppError(anyhow::anyhow!("SMTP is not configured"))),
            }
        }
        "webhook" => {
            let targets: Vec<String> = match serde_json::from_str(&req.config) {
                Ok(v) => v,
                Err(_) => {
                    return Err(AppError(anyhow::anyhow!(
                        "config must be a JSON array of webhook URLs"
                    )));
                }
            };
            if targets.is_empty() {
                return Err(AppError(anyhow::anyhow!("config array must not be empty")));
            }

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
        "sms" => {
            let sms_targets: Vec<serde_json::Value> = match serde_json::from_str(&req.config) {
                Ok(v) => v,
                Err(_) => {
                    return Err(AppError(anyhow::anyhow!(
                        "config must be a JSON array of objects with 'phone' and 'carrier' fields"
                    )));
                }
            };
            if sms_targets.is_empty() {
                return Err(AppError(anyhow::anyhow!("config array must not be empty")));
            }

            let Some(sender) = crate::edition::sms_sender() else {
                return Err(AppError(anyhow::anyhow!(
                    "SMS delivery is not available in this edition"
                )));
            };
            let smtp_cfg = state.smtp_config.read().await;
            match smtp_cfg.as_ref() {
                Some(smtp_cfg) => {
                    let mut results = Vec::new();
                    for target in &sms_targets {
                        let phone = target["phone"].as_str().unwrap_or("");
                        let carrier = target["carrier"].as_str().unwrap_or("");
                        if phone.is_empty() || carrier.is_empty() {
                            results.push(json!({"phone": phone, "carrier": carrier, "status": "error", "error": "missing phone or carrier"}));
                            continue;
                        }
                        match sender.send_alarm(
                            smtp_cfg,
                            phone,
                            carrier,
                            "This is a test alarm notification from Voltrus",
                        ).await {
                            Ok(_) => results.push(json!({"phone": phone, "carrier": carrier, "status": "sent"})),
                            Err(e) => results.push(json!({"phone": phone, "carrier": carrier, "status": "error", "error": e.to_string()})),
                        }
                    }
                    Ok(Json(json!({
                        "channel": "sms",
                        "results": results,
                    })))
                }
                None => Err(AppError(anyhow::anyhow!(
                    "SMTP is not configured (required for SMS gateway)"
                ))),
            }
        }
        _ => unreachable!(),
    }
}
