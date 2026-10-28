pub mod chat;
pub mod email;
pub mod push;
pub mod webhook;

use crate::models::Alarm;
use crate::storage::Storage;
use email::{AlarmEmailData, EmailNotifier};
use webhook::{WebhookPayload, format_triggered_at, send_webhook_alarm};

/// Dispatch alarm notifications to all configured channels for the given source_key.
///
/// Loads notification settings from the database and sends via email, webhook, push, and/or the chat channels of the edition.
/// This is spawned as an async task so it never blocks the alarm creation flow.
pub async fn dispatch_alarm_notifications(
    storage: Storage,
    alarm: &Alarm,
    email_notifier: Option<EmailNotifier>,
) {
    let settings = match storage
        .get_notification_settings_by_source(&alarm.source_key)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                "Failed to load notification settings for {}: {}",
                alarm.source_key,
                e
            );
            return;
        }
    };

    if settings.is_empty() {
        tracing::debug!(
            "No notification settings configured for source {}",
            alarm.source_key
        );
        return;
    }

    // Resolve source_name and device_id from data_sources
    let sources = storage.list_data_sources(None).await.unwrap_or_default();
    let source_info = sources.iter().find(|s| s.key == alarm.source_key);

    let source_name = source_info
        .map(|s| s.name.clone())
        .unwrap_or_else(|| alarm.source_key.clone());

    let device_id = source_info.map(|s| s.device_id.clone()).unwrap_or_else(|| {
        alarm
            .source_key
            .split(':')
            .next()
            .unwrap_or("unknown")
            .to_string()
    });

    let triggered_at_str = format_triggered_at(alarm.triggered_at);

    for setting in &settings {
        if !setting.enabled {
            continue;
        }

        match setting.channel_type.as_str() {
            "email" => {
                if let Some(ref notifier) = email_notifier {
                    // config is a JSON array of recipient email addresses
                    let recipients: Vec<String> =
                        serde_json::from_str(&setting.config).unwrap_or_default();
                    for to in &recipients {
                        let data = AlarmEmailData {
                            source_key: alarm.source_key.clone(),
                            source_name: source_name.clone(),
                            severity: alarm.severity.clone(),
                            value: alarm.value,
                            threshold: alarm.threshold,
                            message: alarm.message.clone(),
                            unit: source_info.map(|s| s.unit.clone()).unwrap_or_default(),
                            triggered_at: triggered_at_str.clone(),
                        };
                        if let Err(e) = notifier.send_alarm_email(to, &data).await {
                            tracing::error!("Failed to send alarm email to {}: {}", to, e);
                        }
                    }
                } else {
                    tracing::warn!("Email notification requested but SMTP is not configured");
                }
            }
            "webhook" => {
                // config is a JSON array of webhook URLs
                let urls: Vec<String> = serde_json::from_str(&setting.config).unwrap_or_default();
                for url in &urls {
                    let payload = WebhookPayload {
                        severity: alarm.severity.clone(),
                        source_key: alarm.source_key.clone(),
                        source_name: source_name.clone(),
                        value: alarm.value,
                        threshold: alarm.threshold,
                        message: alarm.message.clone(),
                        triggered_at: triggered_at_str.clone(),
                        device_info: device_id.clone(),
                    };
                    if let Err(e) = send_webhook_alarm(url, &payload).await {
                        tracing::error!("Failed to send webhook to {}: {}", url, e);
                    }
                }
            }
            channel @ ("telegram" | "whatsapp") => {
                let Some(sender) = crate::edition::chat_sender() else {
                    tracing::warn!(
                        "{} notification requested but chat delivery is not available in this edition",
                        channel
                    );
                    continue;
                };
                let chat_alarm = crate::edition::ChatAlarm {
                    severity: alarm.severity.clone(),
                    source_key: alarm.source_key.clone(),
                    source_name: source_name.clone(),
                    value: alarm.value,
                    threshold: alarm.threshold,
                    unit: source_info.map(|s| s.unit.clone()).unwrap_or_default(),
                    message: alarm.message.clone(),
                    triggered_at: triggered_at_str.clone(),
                };
                match sender.send(channel, &setting.config, &chat_alarm).await {
                    Ok(deliveries) => {
                        for d in deliveries {
                            if let Some(err) = d.error {
                                tracing::error!(
                                    "Failed to send {} notification to {}: {}",
                                    channel,
                                    d.target,
                                    err
                                );
                            }
                        }
                    }
                    Err(e) => tracing::error!("Failed to send {} notification: {}", channel, e),
                }
            }
            other => {
                tracing::warn!("Unknown notification channel type: {}", other);
            }
        }
    }

    // ── Outbound webhooks (v0.39.0) ──
    // Fire any configured outbound webhooks that listen for "alarm" events.
    dispatch_outbound_webhooks(
        &storage,
        "alarm",
        &serde_json::json!({
            "event": "alarm",
            "alarm_id": alarm.id,
            "source_key": alarm.source_key,
            "source_name": source_name,
            "severity": alarm.severity,
            "value": alarm.value,
            "threshold": alarm.threshold,
            "message": alarm.message,
            "device_id": device_id,
            "triggered_at": alarm.triggered_at,
            "triggered_at_iso": triggered_at_str,
        }),
    )
    .await;
}

/// Dispatch outbound webhook payloads for a given event type.
/// Loads all enabled outbound webhooks that subscribe to `event`, filters by device,
/// and fires HTTP POST requests with retry logic.
pub async fn dispatch_outbound_webhooks(
    storage: &Storage,
    event: &str,
    payload: &serde_json::Value,
) {
    let webhooks = match storage.get_outbound_webhooks_for_event(event).await {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("Failed to load outbound webhooks: {}", e);
            return;
        }
    };

    if webhooks.is_empty() {
        return;
    }

    // Extract device_id from payload for device filtering
    let payload_device = payload
        .get("device_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let now = crate::timeutil::now_secs();

    for wh in webhooks {
        let wh_id = wh.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let url = match wh.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => continue,
        };
        let device_filter = wh
            .get("device_filter")
            .and_then(|v| v.as_str())
            .unwrap_or("\"all\"")
            .to_string();
        let headers_str = wh
            .get("headers")
            .and_then(|v| v.as_str())
            .unwrap_or("{}")
            .to_string();
        let retry_count = wh.get("retry_count").and_then(|v| v.as_u64()).unwrap_or(3) as u32;
        let timeout_ms = wh
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(10000);

        // Device filter: "all" or JSON array of device IDs
        if device_filter != "\"all\"" && device_filter != "all" {
            let filter_str = device_filter.trim_matches('"');
            let allowed: Vec<String> = if filter_str.starts_with('[') {
                // JSON array like ["plc1","plc2"]
                serde_json::from_str::<Vec<String>>(filter_str).unwrap_or_default()
            } else {
                filter_str.split(',').map(|s| s.to_string()).collect()
            };
            if !allowed.is_empty() && !allowed.iter().any(|d| d == payload_device) {
                continue;
            }
        }

        // Fire the webhook with retry
        let storage_clone = storage.clone();
        let payload_clone = payload.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let mut success = false;

            for attempt in 0..=retry_count {
                let mut request = client
                    .post(&url)
                    .json(&payload_clone)
                    .timeout(std::time::Duration::from_millis(timeout_ms));

                // Add custom headers
                if let Ok(hdrs) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(&headers_str)
                {
                    for (k, v) in &hdrs {
                        request = request.header(k, v);
                    }
                }

                match request.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!(
                            "Outbound webhook {} fired to {} (attempt {})",
                            wh_id,
                            url,
                            attempt + 1
                        );
                        success = true;
                        break;
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        if attempt < retry_count {
                            let delay = 500 * 2u64.pow(attempt);
                            tracing::warn!(
                                "Outbound webhook {} returned {}, retrying in {}ms",
                                wh_id,
                                status,
                                delay
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                    }
                    Err(e) => {
                        if attempt < retry_count {
                            let delay = 500 * 2u64.pow(attempt);
                            tracing::warn!(
                                "Outbound webhook {} error: {}, retrying in {}ms",
                                wh_id,
                                e,
                                delay
                            );
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                        }
                    }
                }
            }

            // Update last_triggered status
            let status_str = if success { "success" } else { "error" };
            if let Err(e) = storage_clone
                .update_outbound_webhook_status(wh_id, Some(now), Some(status_str.to_string()))
                .await
            {
                tracing::warn!("Failed to update outbound webhook status: {}", e);
            }
        });
    }
}
